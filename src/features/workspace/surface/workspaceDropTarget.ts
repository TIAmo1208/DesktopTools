import type { WorkspaceSnapshot } from "../../../contracts/workspace";
import type { CrossMonitorDrop, CrossMonitorPreviewEvent } from "../interaction/crossMonitorDrag";
import { beforeItemIdForFenceDrop, resolveFenceInsertion, resolveFenceAutoScroll, resolveGridInsertion } from "../interaction/workspaceDrag";
import { collectHitRegions, projectWorkspaceSurface } from "./workspaceSurfaceProjection";
import { createDesktopItemPreview } from "../../desktop-items/desktopItemPreview";

/** 集中适配目标工作区 DOM，拥有命中、围栏临时滚动和插入预览的恢复。 */
export const createWorkspaceDropTarget = (monitorId: string, onScroll: (drag: CrossMonitorPreviewEvent) => void) => {
    let nodes: HTMLElement[] = []; // 本帧插入标记与重排副本。
    let scrollFrame = 0; // 指针静止时的边缘滚动帧。
    const scrolls = new Map<HTMLElement, number>(); // 会话初始滚动位置。
    const faded = new Map<HTMLElement, string>(); // 重排源节点原始透明度。

    /** 清理当前几何预览，结束会话时按提交结果恢复滚动。 */
    const clear = (restoreScroll = false, finish = false) => {
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
    };

    /** 消费已验证指针，返回投放结果与实际渲染几何，不暴露围栏节点。 */
    const update = (drag: CrossMonitorPreviewEvent, workspace: WorkspaceSnapshot | undefined) => {
        clear(); // 每帧替换投放预览，保留会话初始滚动位置供结束时恢复。

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
            const portal = tile?.dataset.portalId;
            drop.rejected = Boolean(tile && !tile.dataset.fenceId && !portal);
            if (portal) drop.portalId = portal;
            const element = drop.rejected
                ? undefined
                : document
                      .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                      .map((node) => node.closest<HTMLElement>("[data-fence-id]"))
                      .find(Boolean); // 最高可见围栏。
            const fence = workspace?.fences.find((fence) => fence.id === element?.dataset.fenceId); // 当前围栏顺序。
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
                            onScroll(drag);
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
                const snapshot = workspace;
                const projected = projectWorkspaceSurface(snapshot, monitorId); // 与领域落位使用相同目标屏网格。
                const target = document
                    .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                    .map((node) => node.closest<HTMLElement>(".desktop-item--unclassified[data-item-id]"))
                    .find(Boolean);
                if (snapshot && projected && target?.dataset.itemId) {
                    const members = new Set(snapshot.fences.flatMap((fence) => fence.members));
                    const ids = drag.itemIds.map((id, index) =>
                        drag.external && (drag.modifiers! & 8 || (!members.has(id) && !snapshot.unclassifiedItems.some((item) => item.stableReference === id)))
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
                    const occupied = new Set(projected.unclassifiedItems.map((item) => `${item.position.left},${item.position.top}`)); // 只有当前可见桌面项目占用容量。
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
                    const insertion = resolveGridInsertion(ids, target.dataset.itemId, { x: drag.pointer.left, y: drag.pointer.top }, bounds, positions, grid);
                    if (insertion?.active && insertion.direction && insertion.positions) {
                        for (const [id, position] of insertion.positions) {
                            const original = Array.from(document.querySelectorAll<HTMLElement>(".desktop-item--unclassified[data-item-id]")).find(
                                (node) => node.dataset.itemId === id,
                            );
                            if (!original || ids.includes(id)) continue;
                            const before = positions.get(id);
                            if (before?.left === position.left && before.top === position.top) continue;
                            const copy = createDesktopItemPreview(original); // 项目模块统一提供重排副本。
                            copy.style.opacity = getComputedStyle(original).opacity;
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

        return {
            drop,
            regions: collectHitRegions(),
            fenceScrolls: Array.from(scrolls.keys(), (content) => ({ fenceId: content.dataset.fenceContentId, scrollTop: content.scrollTop })),
        };
    };
    return { update, clear };
};
