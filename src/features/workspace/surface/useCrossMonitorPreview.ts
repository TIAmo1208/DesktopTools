import { useEffect, useRef } from "react";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { WorkspaceSnapshot } from "../../../contracts/workspace";
import { invokeRuntime } from "../../../bridge/runtime";
import type { CrossMonitorDrag, CrossMonitorDrop } from "../interaction/crossMonitorDrag";
import { beforeItemIdForFenceDrop, resolveFenceInsertion, resolveFenceAutoScroll, resolveGridInsertion } from "../interaction/workspaceDrag";
import { collectHitRegions, projectWorkspaceSurface } from "./workspaceSurfaceProjection";

/** 目标宿主拥有预览节点与真实 DOM 命中，源宿主仍独占事务提交。 */
export const useCrossMonitorPreview = (monitorId: string | undefined, items: DesktopItemSnapshot | undefined, workspace: WorkspaceSnapshot | undefined) => {
    const state = useRef({ items, workspace }); // 消息处理始终读取最新快照。
    state.current = { items, workspace };
    useEffect(() => {
        if (!monitorId) return;
        let nodes: HTMLElement[] = []; // 仅属于当前跨屏预览的临时 DOM。
        let timeout = 0; // 源 Renderer 失效后清理预览。
        let session: string | undefined; // 当前接收会话。
        let scrollFrame = 0; // 指针静止在目标边缘时继续滚动。
        let renderFrame = 0; // 等待悬停展开提交后的单一渲染帧。
        let pendingSession: string | undefined; // 首次呈现前收到结束时也能取消预览。
        let geometryRevision = 0; // 当前 Renderer 发布的几何帧版本。
        let newestOle: { session: string; sequence: number } | undefined; // 旧自动滚动帧不能覆盖新的原生指针。
        let pendingOperation: string | undefined; // 已释放后不再重放旧指针进行自动滚动。
        let replaying = false; // 自动滚动重算不延长源宿主心跳过期时间。
        const scrolls = new Map<HTMLElement, number>(); // 本次会话涉及的目标围栏初始滚动位置。
        const faded = new Map<HTMLElement, string>(); // 传播预览保留原命中几何，结束时恢复原透明度。
        const clear = (restoreScroll = false, finish = false) => {
            cancelAnimationFrame(renderFrame);
            renderFrame = 0;
            pendingSession = undefined;
            cancelAnimationFrame(scrollFrame);
            scrollFrame = 0;
            nodes.forEach((node) => node.remove());
            nodes = [];
            faded.forEach((opacity, node) => {
                node.style.opacity = opacity;
            });
            faded.clear();
            document.querySelectorAll(".fence--remote-drop-target").forEach((node) => node.classList.remove("fence--remote-drop-target"));
            if (finish) {
                scrolls.forEach((position, content) => {
                    if (restoreScroll) content.scrollTop = position;
                    delete content.dataset.dragScrolling;
                });
                scrolls.clear();
            }
        }; // 结束、切屏和卸载共用清理。
        const handleMessage = (event: Event, deferDrop = true) => {
            const drag = (event as CustomEvent).detail as CrossMonitorDrag & {
                phase: string;
                sequence?: number;
                revision?: number;
                modifiers?: number;
                external?: boolean;
            }; // 本应用原生转发数据。
            if (!drag || typeof drag.session !== "string" || drag.phase === "result") return;
            if (drag.source === "ole" && drag.phase === "pending") {
                pendingOperation = drag.session;
                if (session === drag.session || pendingSession === drag.session) clear(false);
                return;
            }
            if (drag.phase === "move" && pendingOperation === drag.session) return;
            if (drag.source === "ole" && drag.phase === "move" && Number.isSafeInteger(drag.sequence)) {
                if (newestOle?.session === drag.session && drag.sequence! < newestOle.sequence) return;
                newestOle = { session: drag.session, sequence: drag.sequence! };
            }
            if (
                drag.target === monitorId &&
                drag.source !== monitorId &&
                (drag.phase === "drop" || (drag.source === "ole" && drag.phase === "move")) &&
                deferDrop
            ) {
                cancelAnimationFrame(renderFrame);
                pendingSession = drag.session;
                renderFrame = requestAnimationFrame(() => handleMessage(event, false));
                return;
            } // 给悬停展开一次 DOM 提交机会。
            if (drag.source === monitorId) return;
            if (drag.phase === "end" || drag.target !== monitorId) {
                if (pendingOperation === drag.session) pendingOperation = undefined;
                if (session === drag.session || pendingSession === drag.session) clear(drag.target !== monitorId || !drag.committed, drag.phase === "end");
                return;
            }
            if (!["move", "drop"].includes(drag.phase) || !Array.isArray(drag.visuals) || drag.visuals.length > 512 || !Array.isArray(drag.itemIds)) return;
            if (![drag.pointer?.left, drag.pointer?.top, drag.origin?.left, drag.origin?.top].every(Number.isFinite)) return;
            if (session && session !== drag.session) clear(true, true);
            session = drag.session;
            clear();
            if (!replaying) {
                window.clearTimeout(timeout);
                timeout = window.setTimeout(() => clear(true, true), 2500);
            }
            for (const visual of drag.visuals) {
                if (![visual.bounds?.left, visual.bounds?.top, visual.bounds?.width, visual.bounds?.height].every(Number.isFinite)) continue;
                const node = document.createElement("div"); // 结构化预览，不解析跨窗口 HTML。
                node.className = drag.kind === "fence" ? "desktop-tile fence fence--cross-preview" : "desktop-item desktop-item--drag-preview";
                const item = state.current.items?.items.find((item) => item.stableReference === visual.itemId); // 当前窗口已有的权威名称与图像。
                if (item?.systemImage) {
                    const image = document.createElement("img"); // Shell 图像。
                    image.src = item.systemImage;
                    image.style.width = "48px";
                    image.style.height = "48px";
                    node.append(image);
                }
                const label = document.createElement("span"); // 使用 textContent 保留名称中的特殊字符。
                label.textContent = item?.displayName ?? visual.title ?? "";
                node.append(label);
                Object.assign(node.style, {
                    position: "fixed",
                    pointerEvents: "none",
                    zIndex: "20000",
                    opacity: "0.75",
                    left: `${drag.pointer.left + visual.bounds.left - drag.origin.left}px`,
                    top: `${drag.pointer.top + visual.bounds.top - drag.origin.top}px`,
                    width: `${visual.bounds.width}px`,
                    height: `${visual.bounds.height}px`,
                });
                (document.querySelector(".fence-workspace") ?? document.body).append(node);
                nodes.push(node);
            }
            const drop: CrossMonitorDrop = {}; // 本帧目标屏真实命中。
            if (drag.kind === "fence")
                drop.snapTargets = [...document.querySelectorAll<HTMLElement>(".desktop-tile[data-hit-id]")]
                    .filter((tile) => tile.getClientRects().length > 0 && getComputedStyle(tile).visibility !== "hidden")
                    .map((tile) => {
                        const bounds = tile.getBoundingClientRect(); // 当前目标格子的可见 DIP 边界。
                        return { left: bounds.left, top: bounds.top, width: bounds.width, height: bounds.height };
                    }); // 释放时读取目标屏实际展开状态和位置。
            const layoutIds = drag.external && (drag.modifiers ?? 0) & 8 ? [] : drag.itemIds; // Ctrl 复制保留来源在原围栏中的可见位置。
            if (drag.kind === "items") {
                const tile = document
                    .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                    .map((node) => node.closest<HTMLElement>(".desktop-tile"))
                    .find(Boolean); // 最高格子决定接收能力，不能穿透。
                drop.rejected = Boolean(tile && !tile.dataset.fenceId);
                const element = drop.rejected
                    ? undefined
                    : document
                          .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                          .map((node) => node.closest<HTMLElement>("[data-fence-id]"))
                          .find(Boolean); // 最高可见围栏。
                const fence = state.current.workspace?.fences.find((fence) => fence.id === element?.dataset.fenceId); // 当前围栏顺序。
                const content = element?.querySelector<HTMLElement>("[data-fence-content-id]"); // 展开时才存在的滚动区域。
                if (fence && content && content.getClientRects().length > 0) {
                    if (!scrolls.has(content)) scrolls.set(content, content.scrollTop);
                    content.dataset.dragScrolling = "true";
                    element?.classList.add("fence--remote-drop-target");
                    drop.fenceId = fence.id;
                    drop.scrollTop = content.scrollTop;
                    if (drag.phase === "move") {
                        const speed = resolveFenceAutoScroll(drag.pointer.top, content.getBoundingClientRect()); // 与源屏相同的边缘滚动规则。
                        if (speed)
                            scrollFrame = requestAnimationFrame(() => {
                                content.scrollTop += speed;
                                replaying = true;
                                handleMessage(new CustomEvent("desktop-tools-drag", { detail: drag }), false);
                                replaying = false;
                            });
                    }
                    const target = document
                        .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                        .map((node) => node.closest<HTMLElement>("[data-item-id]"))
                        .find((node) => node && content.contains(node) && !layoutIds.includes(node.dataset.itemId ?? "")); // 实际可见的插入项目。
                    const bounds = target?.getBoundingClientRect(); // 完整项目几何。
                    const viewport = content.getBoundingClientRect(); // 与原生命中图一致裁剪到内容视口。
                    const visible = bounds
                        ? {
                              left: Math.max(bounds.left, viewport.left, 0),
                              top: Math.max(bounds.top, viewport.top, 0),
                              width: Math.max(0, Math.min(bounds.right, viewport.right, window.innerWidth) - Math.max(bounds.left, viewport.left, 0)),
                              height: Math.max(0, Math.min(bounds.bottom, viewport.bottom, window.innerHeight) - Math.max(bounds.top, viewport.top, 0)),
                          }
                        : undefined;
                    const insertion = target ? resolveFenceInsertion({ x: drag.pointer.left, y: drag.pointer.top }, visible!, fence.layoutMode) : undefined; // 沿当前排列方式计算插入方向。
                    drop.beforeItemId =
                        fence.sortMode && fence.sortMode !== "none"
                            ? null
                            : beforeItemIdForFenceDrop(fence.members, layoutIds, target?.dataset.itemId, insertion?.placement);
                    if (visible && visible.width > 0 && visible.height > 0 && insertion?.active && (!fence.sortMode || fence.sortMode === "none")) {
                        const marker = document.createElement("div"); // 复用既有插入标记，不建立另一套视觉规则。
                        marker.className = "fence-workspace__insertion-marker";
                        const vertical = fence.layoutMode === "tile"; // 平铺为垂线，列表为横线。
                        Object.assign(marker.style, {
                            left: `${visible.left + (insertion.direction === "right" ? visible.width - 3 : 0)}px`,
                            top: `${visible.top + (insertion.direction === "down" ? visible.height - 3 : 0)}px`,
                            width: `${vertical ? 3 : visible.width}px`,
                            height: `${vertical ? visible.height : 3}px`,
                        });
                        document.body.append(marker);
                        nodes.push(marker);
                    }
                }
                if (!element && !drop.rejected && drag.source === "ole" && (drag.external || !(drag.modifiers! & 32))) {
                    const snapshot = state.current.workspace;
                    const projected = projectWorkspaceSurface(snapshot, monitorId); // 与领域落位使用相同目标屏网格。
                    const target = document
                        .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                        .map((node) => node.closest<HTMLElement>(".desktop-item--unclassified[data-item-id]"))
                        .find(Boolean);
                    if (snapshot && projected && target?.dataset.itemId) {
                        const members = new Set(snapshot.fences.flatMap((fence) => fence.members));
                        const ids = drag.itemIds.map((id, index) =>
                            drag.external && (drag.modifiers! & 8 || !snapshot.unclassifiedItems.some((item) => item.stableReference === id))
                                ? `__ole-reservation-${index}`
                                : id,
                        ); // Ctrl 复制不能移走来源占位。
                        const positions = new Map(
                            snapshot.unclassifiedItems
                                .filter(
                                    (item) =>
                                        ids.includes(item.stableReference) ||
                                        (!members.has(item.stableReference) && (item.projectionMonitorId ?? item.monitorId) === monitorId),
                                )
                                .map((item) => [item.stableReference, item.projectionPosition ?? item.position]),
                        );
                        const grid = projected.grid;
                        const occupied = new Set(projected.unclassifiedItems.map((item) => `${item.position.left},${item.position.top}`)); // 隐藏成员来源同样占用容量。
                        for (const id of ids) {
                            if (positions.has(id)) continue;
                            for (let slot = 0; slot < grid.rows * grid.columns; slot++) {
                                const point = {
                                    left: grid.left + Math.floor(slot / grid.rows) * grid.columnWidth,
                                    top: grid.top + (slot % grid.rows) * grid.rowHeight,
                                };
                                const key = `${point.left},${point.top}`;
                                if (occupied.has(key)) continue;
                                occupied.add(key);
                                positions.set(id, point);
                                break;
                            }
                        }
                        const bounds = target.getBoundingClientRect();
                        const insertion = resolveGridInsertion(
                            ids,
                            target.dataset.itemId,
                            { x: drag.pointer.left, y: drag.pointer.top },
                            bounds,
                            positions,
                            grid,
                        );
                        if (insertion?.active && insertion.direction && insertion.positions) {
                            for (const [id, position] of insertion.positions) {
                                const original = Array.from(document.querySelectorAll<HTMLElement>(".desktop-item--unclassified[data-item-id]")).find(
                                    (node) => node.dataset.itemId === id,
                                );
                                if (!original || ids.includes(id)) continue;
                                const before = positions.get(id);
                                if (before?.left === position.left && before.top === position.top) continue;
                                const copy = original.cloneNode(true) as HTMLElement; // 使用原节点的图像与主题，复制节点不参与命中。
                                copy.removeAttribute("data-hit-id");
                                copy.removeAttribute("data-item-id");
                                const rect = original.getBoundingClientRect();
                                Object.assign(copy.style, {
                                    position: "fixed",
                                    left: `${position.left}px`,
                                    top: `${position.top}px`,
                                    width: `${rect.width}px`,
                                    height: `${rect.height}px`,
                                    pointerEvents: "none",
                                    zIndex: "20000",
                                });
                                faded.set(original, original.style.opacity);
                                original.style.opacity = "0.25";
                                document.body.append(copy);
                                nodes.push(copy);
                            }
                            const marker = document.createElement("div");
                            marker.className = "fence-workspace__insertion-marker";
                            const vertical = insertion.direction === "left" || insertion.direction === "right";
                            Object.assign(marker.style, {
                                left: `${insertion.direction === "right" ? bounds.right - 3 : bounds.left}px`,
                                top: `${insertion.direction === "down" ? bounds.bottom - 3 : bounds.top}px`,
                                width: `${vertical ? 3 : bounds.width}px`,
                                height: `${vertical ? bounds.height : 3}px`,
                            });
                            document.body.append(marker);
                            nodes.push(marker);
                        }
                    }
                }
            }
            if (drag.source === "ole" && drag.phase === "move" && Number.isSafeInteger(drag.sequence) && drag.revision === state.current.workspace?.revision) {
                void invokeRuntime("relay_workspace_drag", {
                    payload: {
                        phase: "ole-preview",
                        geometryRevision: ++geometryRevision,
                        session: drag.session,
                        sequence: drag.sequence,
                        revision: drag.revision,
                        monitorId,
                        pointer: drag.pointer,
                        regions: collectHitRegions(),
                        fenceScrolls: Array.from(scrolls.keys(), (content) => ({ fenceId: content.dataset.fenceContentId, scrollTop: content.scrollTop })),
                    },
                }).catch(() => undefined);
            } // 只异步确认已渲染几何，原生 Drop 不等待该响应。
            if (drag.phase === "drop")
                void invokeRuntime("relay_workspace_drag", { payload: { phase: "result", session: drag.session, target: monitorId, drop } });
        };
        window.addEventListener("desktop-tools-drag", handleMessage);
        return () => {
            window.removeEventListener("desktop-tools-drag", handleMessage);
            window.clearTimeout(timeout);
            clear(true, true);
        };
    }, [monitorId]);
};
