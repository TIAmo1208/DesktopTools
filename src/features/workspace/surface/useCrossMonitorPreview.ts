import { useEffect, useRef } from "react";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { WorkspaceSnapshot } from "../../../contracts/workspace";
import { invokeRuntime } from "../../../bridge/runtime";
import type { CrossMonitorDrag, CrossMonitorDrop } from "../interaction/crossMonitorDrag";
import { beforeItemIdForFenceDrop, resolveFenceInsertion, resolveFenceAutoScroll } from "../interaction/workspaceDrag";

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
        let replaying = false; // 自动滚动重算不延长源宿主心跳过期时间。
        const scrolls = new Map<HTMLElement, number>(); // 本次会话涉及的目标围栏初始滚动位置。
        const clear = (restoreScroll = false, finish = false) => {
            cancelAnimationFrame(scrollFrame);
            scrollFrame = 0;
            nodes.forEach((node) => node.remove());
            nodes = [];
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
            const drag = (event as CustomEvent).detail as CrossMonitorDrag & { phase: string }; // 本应用原生转发数据。
            if (!drag || typeof drag.session !== "string" || drag.phase === "result") return;
            if (drag.phase === "drop" && deferDrop) {
                requestAnimationFrame(() => handleMessage(event, false));
                return;
            } // 给悬停展开一次 DOM 提交机会。
            if (drag.source === monitorId) return;
            if (drag.phase === "end" || drag.target !== monitorId) {
                if (session === drag.session) clear(drag.target !== monitorId || !drag.committed, drag.phase === "end");
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
                node.className = drag.kind === "fence" ? "fence fence--cross-preview" : "desktop-item desktop-item--drag-preview";
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
                document.body.append(node);
                nodes.push(node);
            }
            const drop: CrossMonitorDrop = {}; // 本帧目标屏真实命中。
            if (drag.kind === "items") {
                const element = document
                    .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                    .map((node) => node.closest<HTMLElement>("[data-fence-id]"))
                    .find(Boolean); // 最高可见围栏。
                const fence = state.current.workspace?.fences.find((fence) => fence.id === element?.dataset.fenceId); // 当前围栏顺序。
                const content = element?.querySelector<HTMLElement>("[data-fence-content-id]"); // 展开时才存在的滚动区域。
                if (fence && content) {
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
                                handleMessage(new CustomEvent("desktop-tools-drag", { detail: drag }));
                                replaying = false;
                            });
                    }
                    const target = document
                        .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                        .map((node) => node.closest<HTMLElement>("[data-item-id]"))
                        .find((node) => node && content.contains(node) && !drag.itemIds.includes(node.dataset.itemId ?? "")); // 实际可见的插入项目。
                    const insertion = target
                        ? resolveFenceInsertion({ x: drag.pointer.left, y: drag.pointer.top }, target.getBoundingClientRect(), fence.layoutMode)
                        : undefined; // 沿当前排列方式计算插入方向。
                    drop.beforeItemId =
                        fence.sortMode && fence.sortMode !== "none"
                            ? null
                            : beforeItemIdForFenceDrop(fence.members, drag.itemIds, target?.dataset.itemId, insertion?.placement);
                }
            }
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
