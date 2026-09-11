import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react";

import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceRect } from "../../contracts/workspace";
import { moveFence, resizeFence, snapFenceEdges, type FenceResizeDirection } from "../fences/fenceGeometry";
import { clientPointToPhysical, monitorAtPhysicalPoint, physicalPointToMonitorDip } from "../workspace/interaction/workspaceDrag";
import { publishCrossMonitorDrag, resolveCrossMonitorDrop, type CrossMonitorDrag } from "../workspace/interaction/crossMonitorDrag";
import { captureTileDragVisual, createTileDragPreview } from "./tileDragPreview";

/** 公共几何交互使用内容的最小尺寸与领域提交入口。 */
interface DesktopTileGeometryOptions {
    surfaceRef: RefObject<HTMLElement | null>; // 当前公共表面。
    bounds: WorkspaceRect; // 权威展开边界。
    collapsed: boolean; // 当前可见折叠状态。
    title: string; // 跨屏预览标题。
    minimum: { width: number; height: number }; // 内容最小可读尺寸。
    snap: boolean; // 宿主吸附偏好。
    currentMonitor?: DesktopHostMonitor; // 当前显示器。
    monitors: DesktopHostMonitor[]; // 所有跨屏目标。
    onFocus: () => void; // 业务身份对应的焦点命令。
    onCommit: (bounds: WorkspaceRect, monitorId?: string) => Promise<unknown>; // 原子几何提交。
    onBeginInputDrag: () => Promise<void>; // 原生连续输入入口。
    onEndInputDrag: () => Promise<void>; // 原生连续输入释放入口。
    onFailure: (error: unknown) => void; // 统一失败反馈。
}

/** 共用原型几何算法、跨屏预览与取消恢复，不重写内容逻辑。 */
export const useDesktopTileGeometry = (options: DesktopTileGeometryOptions) => {
    const cancel = useRef<(() => void) | undefined>(undefined); // 卸载、取消或失焦时的清理入口。
    useEffect(() => () => cancel.current?.(), []);

    /** 开始移动或八向缩放，仅在释放时提交一次布局。 */
    const handlePointerDown = async (event: PointerEvent<HTMLElement>, direction?: FenceResizeDirection) => {
        if (event.button !== 0 || cancel.current || (event.target instanceof Element && event.target.closest("button,input"))) return;
        const element = options.surfaceRef.current; // 当前实际表面。
        if (!element) return;
        event.preventDefault();
        event.stopPropagation();
        options.onFocus();
        const handle = event.currentTarget; // 异步等待前保存指针捕获节点。
        const pointerId = event.pointerId; // 当前交互唯一指针。
        const startPointer = { x: event.clientX, y: event.clientY }; // 当前客户区起点。
        const start = { ...options.bounds }; // 取消恢复的完整权威边界。
        const content = direction ? undefined : captureTileDragVisual(element); // 移动冻结当前帧，缩放保留真实组件的响应式布局。
        const frozen = content ? createTileDragPreview(content) : undefined; // 缩放不复制计算后的固定轨道和内容尺寸。
        const originalOpacity = element.style.opacity; // 跨屏隐藏源表面，结束时恢复。
        const session = crypto.randomUUID(); // 所有宿主共享的预览身份。
        const targets = [...document.querySelectorAll<HTMLElement>(".desktop-tile[data-hit-id]")]
            .filter((other) => other !== element)
            .map((other) => other.getBoundingClientRect()); // 实际可见的其他格子边界。
        let preview = start; // 当前几何预览。
        let cross: CrossMonitorDrag | undefined; // 最近一次跨屏预览。
        let finished = false; // 清理必须只执行一次。
        let abandoned = false; // 原生输入建立期间卸载时禁止继续。
        cancel.current = () => {
            abandoned = true;
        };
        const handlePendingCancel = () => {
            abandoned = true;
        }; // 输入建立前的失焦、释放和卸载都禁止迟到启动。
        const handlePendingPointer = (pending: globalThis.PointerEvent) => {
            if (pending.pointerId === pointerId) handlePendingCancel();
        }; // 其他指针不取消本次输入。
        const handlePendingKey = (key: globalThis.KeyboardEvent) => {
            if (key.key === "Escape") {
                key.preventDefault();
                handlePendingCancel();
            }
        }; // 原生输入仍在等待时也接受取消。
        window.addEventListener("blur", handlePendingCancel);
        window.addEventListener("keydown", handlePendingKey);
        window.addEventListener("pointerup", handlePendingPointer, true);
        window.addEventListener("pointercancel", handlePendingPointer, true);
        try {
            await options.onBeginInputDrag();
        } catch (error) {
            cancel.current = undefined;
            options.onFailure(error);
            return;
        } finally {
            window.removeEventListener("blur", handlePendingCancel);
            window.removeEventListener("keydown", handlePendingKey);
            window.removeEventListener("pointerup", handlePendingPointer, true);
            window.removeEventListener("pointercancel", handlePendingPointer, true);
        }
        if (abandoned || !handle.isConnected) {
            cancel.current = undefined;
            await options.onEndInputDrag().catch(options.onFailure);
            return;
        }
        try {
            handle.setPointerCapture(pointerId);
        } catch (error) {
            cancel.current = undefined;
            await options.onEndInputDrag().catch(options.onFailure);
            options.onFailure(error);
            return;
        }
        element.dataset.geometryActive = "true";
        if (frozen) {
            Object.assign(frozen.style, { position: "fixed", pointerEvents: "none", zIndex: "20000" });
            element.parentElement?.append(frozen);
            element.style.opacity = "0";
            element.dataset.crossDragHidden = "true";
        }

        /** 直接更新临时 DOM，保持正在播放的内容实例不变。 */
        const render = (rect: WorkspaceRect) => {
            Object.assign((frozen ?? element).style, {
                left: `${rect.left}px`,
                top: `${rect.top}px`,
                width: `${rect.width}px`,
                height: `${options.collapsed ? 28 : rect.height}px`,
            });
        };
        /** 更新本屏几何和跨屏物理指针预览。 */
        const handleMove = (moveEvent: globalThis.PointerEvent) => {
            if (moveEvent.pointerId !== pointerId) return;
            const viewport = { width: window.innerWidth, height: window.innerHeight }; // 当前 DIP 工作区。
            const delta = { x: moveEvent.clientX - startPointer.x, y: moveEvent.clientY - startPointer.y }; // 当前指针位移。
            preview = direction ? resizeFence(direction, start, delta, viewport, options.minimum) : moveFence(start, delta, viewport);
            if (options.snap)
                preview = snapFenceEdges(
                    preview,
                    targets,
                    viewport,
                    8,
                    direction
                        ? { left: direction.includes("w"), right: direction.includes("e"), top: direction.includes("n"), bottom: direction.includes("s") }
                        : undefined,
                    options.minimum,
                );
            render(preview);
            if (frozen && options.currentMonitor) {
                const physical = clientPointToPhysical({ x: moveEvent.clientX, y: moveEvent.clientY }, options.currentMonitor); // Windows 物理指针。
                const target = monitorAtPhysicalPoint(physical, options.monitors); // 当前目标显示器。
                if (target) {
                    const remote = target.id !== options.currentMonitor.id; // 同一格子只在指针所在屏幕呈现。
                    frozen.style.opacity = remote ? "0" : originalOpacity || "1"; // opacity 作用于整个子树，子节点显式 visibility 不能穿透。
                    cross = {
                        session,
                        source: options.currentMonitor.id,
                        target: target.id,
                        kind: "fence",
                        snap: options.snap,
                        minimum: options.minimum,
                        itemIds: [],
                        pointer: physicalPointToMonitorDip(physical, target),
                        origin: { left: startPointer.x, top: startPointer.y },
                        delta: { left: delta.x, top: delta.y },
                        visuals: [{ title: options.title, content, bounds: { ...start, height: options.collapsed ? 28 : start.height } }],
                    };
                    void publishCrossMonitorDrag(cross, "move").catch(options.onFailure);
                }
            }
        };
        /** 释放、Escape、失焦和卸载共用完整清理与回滚。 */
        const finish = (commit: boolean, release?: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            cancel.current = undefined;
            delete element.dataset.geometryActive;
            handle.removeEventListener("pointermove", handleMove);
            handle.removeEventListener("pointerup", handleUp);
            handle.removeEventListener("pointercancel", handleCancel);
            handle.removeEventListener("lostpointercapture", handleCancel);
            window.removeEventListener("keydown", handleKey);
            window.removeEventListener("blur", handleCancel);
            if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
            if (!commit) {
                element.style.opacity = originalOpacity;
                frozen?.remove();
                delete element.dataset.crossDragHidden;
                if (cross) void publishCrossMonitorDrag(cross, "end").catch(options.onFailure);
                render(start);
                void options.onEndInputDrag().catch(options.onFailure);
                return;
            }
            const center = options.currentMonitor
                ? clientPointToPhysical(
                      direction
                          ? { x: preview.left + preview.width / 2, y: preview.top + preview.height / 2 }
                          : {
                                x: (release?.clientX ?? startPointer.x) - startPointer.x + start.left + preview.width / 2,
                                y: (release?.clientY ?? startPointer.y) - startPointer.y + start.top + preview.height / 2,
                            },
                      options.currentMonitor,
                  )
                : undefined; // 使用未截断指针计算目标归属。
            const releasePointer =
                !direction && options.currentMonitor && release
                    ? clientPointToPhysical({ x: release.clientX, y: release.clientY }, options.currentMonitor)
                    : center; // 与预览一样使用指针确定目标屏，不用格子中心切换归属。
            const monitor = releasePointer ? monitorAtPhysicalPoint(releasePointer, options.monitors) : options.currentMonitor; // 释放目的屏。
            const targetCenter = center && monitor ? physicalPointToMonitorDip(center, monitor) : undefined; // 目标 DIP 中心。
            const viewport = monitor
                ? { width: monitor.workWidth / monitor.scaleFactor, height: monitor.workHeight / monitor.scaleFactor }
                : { width: window.innerWidth, height: window.innerHeight }; // 目标工作区。
            let target = moveFence(
                {
                    ...preview,
                    left: targetCenter ? targetCenter.left - preview.width / 2 : preview.left,
                    top: targetCenter ? targetCenter.top - preview.height / 2 : preview.top,
                },
                { x: 0, y: 0 },
                viewport,
            ); // 保留逻辑尺寸并约束目标位置。
            cancel.current = handlePendingCancel;
            window.addEventListener("keydown", handlePendingKey);
            window.addEventListener("blur", handlePendingCancel);
            void (async () => {
                try {
                    let snapTargets: WorkspaceRect[] = targets; // 本屏使用开始时的真实表面，跨屏在释放时重新读取。
                    if (options.snap && monitor && options.currentMonitor && monitor.id !== options.currentMonitor.id) {
                        cross = {
                            session,
                            source: options.currentMonitor.id,
                            target: monitor.id,
                            kind: "fence",
                            snap: options.snap,
                            minimum: options.minimum,
                            visuals: [],
                            itemIds: [],
                            pointer: targetCenter ?? { left: target.left, top: target.top },
                            origin: { left: startPointer.x, top: startPointer.y },
                            delta: { left: 0, top: 0 },
                        };
                        const drop = await resolveCrossMonitorDrop(cross); // 目标宿主提供当前可见边界，不猜测隐藏或临时展开状态。
                        if (
                            !Array.isArray(drop.snapTargets) ||
                            drop.snapTargets.some(
                                (rect) =>
                                    !rect || ![rect.left, rect.top, rect.width, rect.height].every(Number.isFinite) || rect.width <= 0 || rect.height <= 0,
                            )
                        )
                            throw new Error("目标屏格子边界无效，格子已恢复原位");
                        snapTargets = drop.snapTargets;
                    }
                    if (abandoned) {
                        render(start);
                        return;
                    }
                    if (options.snap)
                        target = snapFenceEdges(
                            target,
                            snapTargets,
                            viewport,
                            8,
                            direction
                                ? {
                                      left: direction.includes("w"),
                                      right: direction.includes("e"),
                                      top: direction.includes("n"),
                                      bottom: direction.includes("s"),
                                  }
                                : undefined,
                            options.minimum,
                        );
                    window.removeEventListener("keydown", handlePendingKey);
                    window.removeEventListener("blur", handlePendingCancel);
                    cancel.current = () => undefined; // 领域提交开始后只等待结果，禁止新手势并避免承诺撤回已提交命令。
                    await options.onCommit(target, monitor?.id);
                } catch (error) {
                    render(start);
                    if (!abandoned) options.onFailure(error);
                } finally {
                    element.style.opacity = originalOpacity;
                    frozen?.remove();
                    delete element.dataset.crossDragHidden;
                    cancel.current = undefined;
                    window.removeEventListener("keydown", handlePendingKey);
                    window.removeEventListener("blur", handlePendingCancel);
                    if (cross) await publishCrossMonitorDrag(cross, "end").catch(options.onFailure);
                    await options.onEndInputDrag().catch(options.onFailure);
                }
            })();
        };
        const handleUp = (release: globalThis.PointerEvent) => {
            if (release.pointerId === pointerId) finish(true, release);
        }; // 只有当前指针可以提交。
        const handleCancel = () => finish(false); // 所有取消路径恢复原始边界。
        const handleKey = (key: globalThis.KeyboardEvent) => {
            if (key.key === "Escape") {
                key.preventDefault();
                handleCancel();
            }
        }; // Escape 优先取消几何。
        cancel.current = handleCancel;
        handle.addEventListener("pointermove", handleMove);
        handle.addEventListener("pointerup", handleUp);
        handle.addEventListener("pointercancel", handleCancel);
        handle.addEventListener("lostpointercapture", handleCancel);
        window.addEventListener("keydown", handleKey);
        window.addEventListener("blur", handleCancel);
    };

    /** 表面方向键移动，Shift 加方向键缩放；内容控件保留自身键盘行为。 */
    const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
        if (event.target !== event.currentTarget || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
        if (cancel.current || event.ctrlKey || event.altKey || (event.shiftKey && options.collapsed)) return;
        event.preventDefault();
        event.stopPropagation();
        const delta = {
            x: event.key === "ArrowLeft" ? -8 : event.key === "ArrowRight" ? 8 : 0,
            y: event.key === "ArrowUp" ? -8 : event.key === "ArrowDown" ? 8 : 0,
        }; // 既有功能格子键盘步进。
        const viewport = { width: window.innerWidth, height: window.innerHeight }; // 当前逻辑工作区。
        const next = event.shiftKey ? resizeFence("se", options.bounds, delta, viewport, options.minimum) : moveFence(options.bounds, delta, viewport); // 复用鼠标交互的几何算法。
        void options.onCommit(next, options.currentMonitor?.id).catch(options.onFailure);
    };
    return { handlePointerDown, handleKeyDown };
};
