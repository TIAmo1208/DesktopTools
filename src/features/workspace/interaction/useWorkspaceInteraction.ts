import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react";

import type { WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceInteractionCommand, WorkspaceSnapshot } from "../../../contracts/workspace";
import {
    intersectInteractionRects,
    uncoveredInteractionRects,
    itemIdsIntersectingRect,
    nextDirectionalItemId,
    nextWorkspaceFocus,
    selectionModeForDirectionalKey,
    type InteractionItem,
    type InteractionSurface,
    type NavigationDirection,
    workspaceFocusTargetsInVisualOrder,
} from "./workspaceInteraction";

/** 工作区级选择、空间导航和高层焦点交互参数。 */
interface WorkspaceInteractionOptions {
    workspaceRef: RefObject<HTMLElement | null>;
    workspace?: WorkspaceSnapshot;
    unclassifiedItemIds: string[];
    onCommand: (command: WorkspaceCommand) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onOpenItems: (itemIds: string[]) => Promise<void>;
    onBeginRename: (itemId: string) => void;
    onRenameUnavailable: () => void;
    onDeleteItems: (itemIds: string[], permanent: boolean) => Promise<void>;
    onSetClipboard: (itemIds: string[], cut: boolean) => Promise<void>;
    onPasteItems: () => Promise<void>;
    onContextMenu: (target: WorkspaceContextMenuTarget, clientX: number, clientY: number) => Promise<WorkspaceSnapshot>;
    isContextMenuOpen: () => boolean;
}

// 方向键到空间导航方向的稳定映射。
const ARROW_DIRECTIONS: Partial<Record<string, NavigationDirection>> = {
    ArrowUp: "up",
    ArrowRight: "right",
    ArrowDown: "down",
    ArrowLeft: "left",
};

/** 收集当前宿主中可见桌面项目的空间边界。 */
const collectInteractionItems = (workspaceElement: HTMLElement | null): InteractionItem[] =>
    Array.from(workspaceElement?.querySelectorAll<HTMLElement>("[data-item-id]") ?? []).flatMap((element) => {
        const id = element.dataset.itemId; // 当前项目稳定引用。
        const rect = element.getBoundingClientRect(); // 当前项目客户区交互边界。
        return id && rect.width > 0 && rect.height > 0 ? [{ id, rect }] : [];
    });

/** 管理工作区背景框选、窗口失焦和键盘焦点交互。 */
export const useWorkspaceInteraction = ({
    workspaceRef,
    workspace,
    unclassifiedItemIds,
    onCommand,
    onInteraction,
    onOpenItems,
    onBeginRename,
    onRenameUnavailable,
    onDeleteItems,
    onSetClipboard,
    onPasteItems,
    onContextMenu,
    isContextMenuOpen,
}: WorkspaceInteractionOptions) => {
    const lastPointer = useRef({ x: 20, y: 20 }); // 当前显示器内最近一次可用菜单锚点。
    const cancelMarquee = useRef<(() => void) | undefined>(undefined); // 当前会话及尚未提交结果的取消入口。
    useEffect(() => () => cancelMarquee.current?.(), []);
    useEffect(() => {
        /** 窗口失焦时清空当前项目选择。 */
        const handleWindowBlur = () => {
            cancelMarquee.current?.();
            if (isContextMenuOpen()) return;
            void onInteraction({ type: "focus-surface", focus: { type: "background" } }).catch(() => undefined);
        };
        window.addEventListener("blur", handleWindowBlur);
        return () => window.removeEventListener("blur", handleWindowBlur);
    }, [isContextMenuOpen, onInteraction]);

    /** 从空白内容启动框选，范围始终限定为按下时的容器。 */
    const handleWorkspacePointerDown = (event: PointerEvent<HTMLElement>) => {
        if (event.button !== 0 || event.target !== event.currentTarget) return;
        cancelMarquee.current?.();
        event.preventDefault();
        event.stopPropagation();
        const handle = event.currentTarget; // 捕获指针的起点内容容器。
        const fenceId = handle.dataset.fenceContentId; // 未归类容器没有围栏身份。
        const surface = handle.closest<HTMLElement>("[data-fence-id]"); // 保留当前围栏语义焦点。
        const focus = fenceId ? { type: "fence" as const, fenceId } : { type: "background" as const }; // 空选择的焦点归属。
        (surface ?? workspaceRef.current)?.focus({ preventScroll: true });
        const focused = onInteraction({ type: "focus-surface", focus }).then(
            () => true,
            () => false,
        ); // 初始清空完成后才能提交最终选择。

        // 当前框选开始时的客户区位置。
        const start = { x: event.clientX, y: event.clientY };
        // 当前框选矩形预览。
        const marquee = document.createElement("div");
        marquee.className = "fence-workspace__marquee";
        workspaceRef.current?.append(marquee);
        let selection = { left: start.x, top: start.y, width: 0, height: 0 }; // 当前客户区框选矩形。
        let finished = false; // 当前连续框选结束标记。
        let cancelled = false; // 取消后拒绝迟到的异步提交。
        const pointerId = event.pointerId; // 当前唯一捕获指针。

        /** 在滚动或尺寸变化后读取真实可见的内容视口。 */
        const viewport = () => {
            const rect = (fenceId ? handle : (workspaceRef.current ?? handle)).getBoundingClientRect(); // 当前容器几何。
            return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
        };

        try {
            handle.setPointerCapture(event.pointerId);
        } catch {
            marquee.remove();
            return;
        }

        /** 更新当前框选矩形预览。 */
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            if (moveEvent.pointerId !== pointerId) return;
            const clip = viewport(); // 框选预览不得越过起点视口。
            const x = Math.max(clip.left, Math.min(moveEvent.clientX, clip.left + clip.width)); // 当前裁剪后横坐标。
            const y = Math.max(clip.top, Math.min(moveEvent.clientY, clip.top + clip.height)); // 当前裁剪后纵坐标。
            selection = {
                left: Math.min(start.x, x),
                top: Math.min(start.y, y),
                width: Math.abs(x - start.x),
                height: Math.abs(y - start.y),
            };
            marquee.style.left = `${selection.left}px`;
            marquee.style.top = `${selection.top}px`;
            marquee.style.width = `${selection.width}px`;
            marquee.style.height = `${selection.height}px`;
        };

        /** 清理捕获、预览与本次会话的全部监听。 */
        const cleanup = () => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishMarquee);
            handle.removeEventListener("pointercancel", cancel);
            handle.removeEventListener("lostpointercapture", cancel);
            window.removeEventListener("keydown", handleCancelKey, true);
            if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
            marquee.remove();
        };

        /** 取消既中止当前捕获，也阻止等待初始焦点的迟到结果。 */
        const cancel = () => {
            cancelled = true;
            cleanup();
            if (cancelMarquee.current === cancel) cancelMarquee.current = undefined;
        };
        /** Escape 只取消正在进行的选框。 */
        const handleCancelKey = (keyEvent: globalThis.KeyboardEvent) => {
            if (keyEvent.key !== "Escape") return;
            keyEvent.preventDefault();
            keyEvent.stopPropagation();
            cancel();
        };

        /** 释放时只提交起点容器内实际可见的成员。 */
        const finishMarquee = (finishEvent: globalThis.PointerEvent) => {
            if (finished || finishEvent.pointerId !== pointerId) return;
            handlePointerMove(finishEvent);
            cleanup();
            const clip = intersectInteractionRects(selection, viewport()); // 释放时按最新视口再次裁剪。
            const tiles = Array.from(workspaceRef.current?.querySelectorAll<HTMLElement>(".desktop-tile") ?? []); // 实际呈现的所有语义表面。
            const layer = surface ? Number.parseInt(window.getComputedStyle(surface).zIndex, 10) || 0 : -Infinity; // 起点围栏的实际层级。
            const covers = tiles
                .filter((tile) => {
                    const z = Number.parseInt(window.getComputedStyle(tile).zIndex, 10) || 0; // 含临时展开层级的真实样式。
                    return tile !== surface && (z > layer || (z === layer && tiles.indexOf(tile) > tiles.indexOf(surface!)));
                })
                .map((tile) => tile.getBoundingClientRect()); // 更高表面的语义矩形。
            const header = surface?.querySelector<HTMLElement>(".desktop-tile-header[data-visible='true']"); // 悬浮标题覆盖部分也不参与内容选择。
            if (header) covers.push(header.getBoundingClientRect());
            const regions = clip ? uncoveredInteractionRects(clip, covers) : []; // 选框实际可命中的可见片段。
            const container = fenceId ? handle : workspaceRef.current?.querySelector<HTMLElement>(".fence-workspace__unclassified"); // 限定候选 DOM 来源。
            const interactionItems = collectInteractionItems(container ?? null).filter(({ rect }) =>
                regions.some((region) => intersectInteractionRects(rect, region)),
            ); // 排除裁剪或遮挡部分的项目。
            const itemIds = itemIdsIntersectingRect(selection, interactionItems); // 沿用现有视觉顺序和锚点规则。
            void focused
                .then(async (ready) => {
                    if (!ready || cancelled || !handle.isConnected) return;
                    if (itemIds.length > 0) await onInteraction({ type: "select-items", itemIds });
                })
                .catch(() => undefined)
                .finally(() => {
                    if (cancelMarquee.current === cancel) cancelMarquee.current = undefined;
                });
        };

        cancelMarquee.current = cancel;
        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishMarquee);
        handle.addEventListener("pointercancel", cancel);
        handle.addEventListener("lostpointercapture", cancel);
        window.addEventListener("keydown", handleCancelKey, true);
    };

    /** 记录菜单键打开桌面背景菜单时使用的最近指针位置。 */
    const handleWorkspacePointerMove = (event: PointerEvent<HTMLElement>) => {
        lastPointer.current = { x: event.clientX, y: event.clientY };
    };

    /** 在桌面空白区域显示 Shell 背景菜单和产品命令。 */
    const handleWorkspaceContextMenu = (event: import("react").MouseEvent<HTMLElement>) => {
        if (event.target instanceof Element && event.target.closest("[data-item-id],[data-fence-id],button")) return;
        event.preventDefault();
        event.stopPropagation();
        workspaceRef.current?.focus({ preventScroll: true });
        void onInteraction({ type: "focus-surface", focus: { type: "background" } })
            .then(() => onContextMenu({ type: "background" }, event.clientX, event.clientY))
            .catch(() => undefined);
    };

    /** 处理当前工作区的选择、空间导航和高层焦点键盘命令。 */
    const handleWorkspaceKeyDown = (event: KeyboardEvent<HTMLElement>) => {
        if (!workspace || (event.target instanceof Element && event.target.closest("button,input,textarea,[contenteditable='true']"))) return;
        const focus = workspace.interaction.keyboardFocus; // 当前权威键盘焦点。
        const focusedItemIds =
            focus.type === "item"
                ? workspace.interaction.selectedItemIds.includes(focus.itemId)
                    ? workspace.interaction.selectedItemIds
                    : [focus.itemId]
                : []; // 当前项目焦点对应的完整选择集合。

        if (event.key === "Enter" && focus.type === "item") {
            event.preventDefault();
            void onOpenItems([focus.itemId]).catch(() => undefined);
            return;
        }
        if (event.key === "F2" && focus.type === "item") {
            event.preventDefault();
            if (!event.repeat) {
                if (focusedItemIds.length === 1) onBeginRename(focus.itemId);
                else onRenameUnavailable();
            }
            return;
        }
        if (event.key === "Delete" && focusedItemIds.length > 0) {
            event.preventDefault();
            if (!event.repeat) void onDeleteItems(focusedItemIds, event.shiftKey).catch(() => undefined);
            return;
        }
        if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === "c" && focusedItemIds.length > 0) {
            event.preventDefault();
            if (!event.repeat) void onSetClipboard(focusedItemIds, false).catch(() => undefined);
            return;
        }
        if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === "x" && focusedItemIds.length > 0) {
            event.preventDefault();
            if (!event.repeat) void onSetClipboard(focusedItemIds, true).catch(() => undefined);
            return;
        }
        if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === "v") {
            event.preventDefault();
            if (!event.repeat) void onPasteItems().catch(() => undefined);
            return;
        }
        if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
            event.preventDefault();
            const focusElement =
                focus.type === "item"
                    ? Array.from(workspaceRef.current?.querySelectorAll<HTMLElement>("[data-item-id]") ?? []).find(
                          (element) => element.dataset.itemId === focus.itemId,
                      )
                    : focus.type === "fence"
                      ? workspaceRef.current?.querySelector<HTMLElement>('[data-fence-id="' + CSS.escape(focus.fenceId) + '"]')
                      : undefined; // 当前菜单键焦点目标对应的可见 DOM 节点。
            const bounds = focusElement?.getBoundingClientRect(); // 当前菜单键目标的客户区边界。
            const anchor = bounds ? { x: bounds.left + 12, y: bounds.top + 12 } : lastPointer.current; // 当前菜单键使用的可见锚点。
            const target: WorkspaceContextMenuTarget =
                focus.type === "item"
                    ? {
                          type: "items",
                          itemIds: workspace.interaction.selectedItemIds.includes(focus.itemId) ? workspace.interaction.selectedItemIds : [focus.itemId],
                      }
                    : focus.type === "fence"
                      ? { type: "fence", fenceId: focus.fenceId }
                      : { type: "background" }; // 当前焦点对应的原生菜单语义目标。
            void onContextMenu(target, anchor.x, anchor.y).catch(() => undefined);
            return;
        }

        if (event.ctrlKey && !event.shiftKey && event.key.toLowerCase() === "z") {
            event.preventDefault();
            void onCommand({ type: "undo-layout" }).catch(() => undefined);
            return;
        }
        if (event.ctrlKey && (event.key.toLowerCase() === "y" || (event.shiftKey && event.key.toLowerCase() === "z"))) {
            event.preventDefault();
            void onCommand({ type: "redo-layout" }).catch(() => undefined);
            return;
        }

        if (event.ctrlKey && event.code === "Space" && focus.type === "item") {
            event.preventDefault();
            void onInteraction({ type: "focus-item", itemId: focus.itemId, selectionMode: "toggle" }).catch(() => undefined);
            return;
        }
        if (event.ctrlKey && event.key.toLowerCase() === "a") {
            event.preventDefault();
            void onInteraction({ type: "select-focus-container" }).catch(() => undefined);
            return;
        }
        if (event.key === "Tab") {
            event.preventDefault();
            // 当前工作区已呈现的围栏标题和组件表面。
            const surfaces: InteractionSurface[] = Array.from(
                workspaceRef.current?.querySelectorAll<HTMLElement>("[data-fence-id],[data-component-id]") ?? [],
            ).flatMap((element) => {
                const fenceId = element.dataset.fenceId; // 当前围栏稳定标识。
                const componentId = element.dataset.componentId; // 当前组件稳定标识。
                // 当前高层表面的语义焦点目标。
                const target = fenceId ? ({ type: "fence", fenceId } as const) : componentId ? ({ type: "component", componentId } as const) : undefined;
                return target
                    ? [
                          {
                              target,
                              rect: element.getBoundingClientRect(),
                              zIndex: Number.parseInt(window.getComputedStyle(element).zIndex, 10) || 0,
                          },
                      ]
                    : [];
            });
            const targets = workspaceFocusTargetsInVisualOrder(surfaces); // 背景、围栏标题和组件组成的当前视觉 Tab 顺序。
            const focusedFence = focus.type === "item" ? workspace.fences.find((fence) => fence.members.includes(focus.itemId)) : undefined; // 当前项目焦点所属的围栏。
            // 当前项目焦点对应的高层语义目标。
            const current =
                focus.type === "item" ? (focusedFence ? { type: "fence" as const, fenceId: focusedFence.id } : { type: "background" as const }) : focus;
            const next = nextWorkspaceFocus(targets, current, event.shiftKey); // 当前 Tab 方向命中的下一高层焦点。
            if (next && next.type !== "item")
                void onInteraction({ type: "focus-surface", focus: next })
                    .then(() => {
                        const element =
                            next.type === "fence"
                                ? workspaceRef.current?.querySelector<HTMLElement>(`[data-fence-id="${CSS.escape(next.fenceId)}"]`)
                                : next.type === "component"
                                  ? workspaceRef.current?.querySelector<HTMLElement>(`[data-component-id="${CSS.escape(next.componentId)}"]`)
                                  : workspaceRef.current; // 语义焦点与真实 DOM 焦点同步。
                        element?.focus({ preventScroll: true });
                    })
                    .catch(() => undefined);
            return;
        }

        const direction = ARROW_DIRECTIONS[event.key]; // 当前按键对应的空间导航方向。
        if (!direction) return;
        event.preventDefault();
        // 当前焦点容器的项目视觉顺序。
        const containerItemIds =
            focus.type === "fence"
                ? (workspace.fences.find((fence) => fence.id === focus.fenceId)?.members ?? [])
                : focus.type === "item"
                  ? (workspace.fences.find((fence) => fence.members.includes(focus.itemId))?.members ?? unclassifiedItemIds)
                  : focus.type === "background"
                    ? unclassifiedItemIds
                    : [];
        const allInteractionItems = collectInteractionItems(workspaceRef.current); // 当前显示器中全部可见项目的空间边界。
        const interactionItems = allInteractionItems.filter(({ id }) => containerItemIds.includes(id)); // 当前焦点容器中可见项目的空间边界。
        const currentItemId = focus.type === "item" ? focus.itemId : interactionItems[0]?.id; // 当前方向键计算使用的起点项目。
        if (!currentItemId) return;
        // 当前空间方向命中的目标项目。
        const targetItemId = focus.type === "item" ? nextDirectionalItemId(interactionItems, allInteractionItems, currentItemId, direction) : currentItemId;
        if (!targetItemId) return;
        void onInteraction({ type: "focus-item", itemId: targetItemId, selectionMode: selectionModeForDirectionalKey(event) })
            .then(() => {
                // 已获得键盘焦点的项目节点。
                const target = Array.from(workspaceRef.current?.querySelectorAll<HTMLElement>("[data-item-id]") ?? []).find(
                    (element) => element.dataset.itemId === targetItemId,
                );
                target?.scrollIntoView({ block: "nearest", inline: "nearest" });
            })
            .catch(() => undefined);
    };

    return { handleWorkspacePointerDown, handleWorkspacePointerMove, handleWorkspaceContextMenu, handleWorkspaceKeyDown };
};
