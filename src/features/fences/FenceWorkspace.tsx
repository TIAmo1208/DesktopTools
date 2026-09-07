import { useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent } from "react";

import type { DesktopItemSnapshot } from "../../contracts/desktopItem";
import fenceLayoutListIcon from "../../../resources/fence-layout-list.svg?raw";
import fenceLayoutTileIcon from "../../../resources/fence-layout-tile.svg?raw";
import fenceToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import fenceToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import { exitDesktopTools } from "../../bridge/desktopHost";
import type { DesktopHostMonitor, DesktopHostSnapshot } from "../../contracts/desktopHost";
import {
    FENCE_MIN_HEIGHT,
    FENCE_MIN_WIDTH,
    type FenceSnapshot,
    type WorkspaceCommand,
    type WorkspaceContextMenuTarget,
    type WorkspaceFocusTarget,
    type WorkspaceInteractionCommand,
    type WorkspaceRect,
    type WorkspaceSnapshot,
} from "../../contracts/workspace";
import { DesktopItemTile } from "../desktop-items/DesktopItemGrid";
import { projectDesktopItems, type DesktopItemViewModel } from "../desktop-items/desktopItemViewModel";
import { OfficialComponentHost } from "../components/OfficialComponentHost";
import { useWorkspaceInteraction } from "../workspace/interaction/useWorkspaceInteraction";
import { useWorkspaceDrag } from "../workspace/interaction/useWorkspaceDrag";
import { clientPointToPhysical, monitorAtPhysicalPoint, physicalPointToMonitorDip } from "../workspace/interaction/workspaceDrag";
import { moveFence, resizeFence, snapFenceEdges, type FenceResizeDirection } from "./fenceGeometry";
import "./FenceWorkspace.css";

/** 围栏工作区接收的权威快照与命令边界。 */
interface FenceWorkspaceProps {
    desktopItems?: DesktopItemSnapshot;
    workspace?: WorkspaceSnapshot;
    completeWorkspace?: WorkspaceSnapshot;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    hostSnapshot?: DesktopHostSnapshot;
    failure?: string;
    status?: string;
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onOpenItems: (itemIds: string[]) => Promise<void>;
    onRenameItem: (itemId: string, newName: string) => Promise<void>;
    onDeleteItems: (itemIds: string[], permanent: boolean) => Promise<void>;
    onSetClipboard: (itemIds: string[], cut: boolean) => Promise<void>;
    onPasteItems: () => Promise<void>;
    onStartShellDrag: (itemIds: string[]) => Promise<void>;
    onContextMenu: (target: WorkspaceContextMenuTarget, clientX: number, clientY: number) => Promise<WorkspaceSnapshot>;
    onItemInput?: (itemId: string) => void;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
}

/** 单个围栏的呈现与交互参数。 */
interface FencePanelProps {
    fence: FenceSnapshot;
    snapTargets: WorkspaceRect[];
    items: DesktopItemViewModel[];
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onFenceContextMenu: (event: MouseEvent<HTMLElement>, fenceId: string) => void;
    onItemContextMenu: (event: MouseEvent<HTMLLIElement>, itemId: string) => void;
    onOpenItems: (itemIds: string[]) => Promise<void>;
    onItemPointerDown: (event: PointerEvent<HTMLLIElement>, item: DesktopItemViewModel) => void;
    onItemInput?: (itemId: string) => void;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    selectedItemIds: Set<string>;
    keyboardFocus?: WorkspaceFocusTarget;
    renameItemId?: string;
    renameValue: string;
    onRenameValueChange: (value: string) => void;
    onRenameCommit: () => void;
    onRenameCancel: () => void;
}

// 围栏提供的八个缩放边框方向。
const RESIZE_DIRECTIONS: FenceResizeDirection[] = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];
const FENCE_Z_INDEX_BASE = 1000; // 围栏始终位于未归类图标之上，拖动预览使用更高层级。

/** 将权威围栏边界直接投射到交互预览 DOM。 */
const applyFenceBounds = (element: HTMLElement, bounds: WorkspaceRect, collapsed = false) => {
    element.style.left = `${bounds.left}px`;
    element.style.top = `${bounds.top}px`;
    element.style.width = `${bounds.width}px`;
    element.style.height = `${collapsed ? 28 : bounds.height}px`;
};

/** 呈现一个可移动、缩放、折叠、滚动和接收项目的围栏。 */
const FencePanel = ({
    fence,
    snapTargets,
    items,
    onCommand,
    onInteraction,
    onFenceContextMenu,
    onItemContextMenu,
    onOpenItems,
    onItemPointerDown,
    onItemInput,
    onBeginInputDrag,
    onEndInputDrag,
    currentMonitor,
    monitors,
    selectedItemIds,
    keyboardFocus,
    renameItemId,
    renameValue,
    onRenameValueChange,
    onRenameCommit,
    onRenameCancel,
}: FencePanelProps) => {
    const fenceRef = useRef<HTMLElement>(null); // 当前围栏表面节点。
    const contentRef = useRef<HTMLUListElement>(null); // 当前围栏滚动内容节点。
    const [scrollbar, setScrollbar] = useState({ maxScrollTop: 0, thumbTop: 0, thumbHeight: 0, trackHeight: 0 }); // 当前覆盖式滚动条几何。

    /** 按原型规则同步围栏覆盖式滚动条。 */
    const updateScrollbar = () => {
        const content = contentRef.current;
        const track = content?.parentElement?.querySelector<HTMLElement>(".fence__scroll-track");
        if (!content || !track) return;
        const maxScrollTop = Math.max(0, content.scrollHeight - content.clientHeight);
        const trackHeight = track.clientHeight;
        const thumbHeight = maxScrollTop > 0 ? Math.max(18, (content.clientHeight / content.scrollHeight) * trackHeight) : 0;
        const maxThumbTop = Math.max(0, trackHeight - thumbHeight);
        setScrollbar({ maxScrollTop, trackHeight, thumbHeight, thumbTop: maxThumbTop > 0 ? (content.scrollTop / maxScrollTop) * maxThumbTop : 0 });
    };

    useEffect(() => {
        if (contentRef.current) contentRef.current.scrollTop = fence.scrollTop;
    }, [fence.scrollTop]);

    useEffect(() => {
        updateScrollbar();
        const observer = new ResizeObserver(updateScrollbar);
        if (contentRef.current) observer.observe(contentRef.current);
        const track = contentRef.current?.parentElement?.querySelector<HTMLElement>(".fence__scroll-track");
        if (track) observer.observe(track);
        return () => observer.disconnect();
    }, [fence.collapsed, fence.layoutMode, items.length]);

    /** 开始围栏移动或缩放，并在指针释放时提交一次边界。 */
    const beginGeometry = async (event: PointerEvent<HTMLElement>, direction?: FenceResizeDirection) => {
        if (event.button !== 0 || (event.target instanceof Element && event.target.closest("button"))) return;
        event.preventDefault();
        event.stopPropagation();
        // 捕获当前连续指针交互的围栏节点。
        const element = fenceRef.current;
        if (!element) return;
        // 指针捕获和原生监听所属的交互节点。
        const handle = event.currentTarget;
        // 当前交互开始时的指针位置。
        const startPointer = { x: event.clientX, y: event.clientY };
        // 当前交互开始时的权威围栏边界。
        const startBounds = { ...fence.bounds };
        // 指针移动期间持续更新的预览边界。
        let preview = startBounds;
        let finished = false; // 当前连续指针交互结束标记。
        try {
            await onBeginInputDrag();
        } catch {
            return;
        }
        try {
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        // 根据最新指针更新直接 DOM 预览。
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            // 当前指针相对交互起点的位移。
            const delta = { x: moveEvent.clientX - startPointer.x, y: moveEvent.clientY - startPointer.y };
            // 当前透明宿主工作区尺寸。
            const viewport = { width: window.innerWidth, height: window.innerHeight };
            preview = direction ? resizeFence(direction, startBounds, delta, viewport) : moveFence(startBounds, delta, viewport);
            preview = snapFenceEdges(
                preview,
                snapTargets,
                viewport,
                8,
                direction
                    ? { left: direction.includes("w"), right: direction.includes("e"), top: direction.includes("n"), bottom: direction.includes("s") }
                    : undefined,
            );
            applyFenceBounds(element, preview, fence.collapsed);
        };

        // 结束指针交互并提交或恢复权威边界。
        const finishGeometry = (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishGeometry);
            handle.removeEventListener("pointercancel", finishGeometry);
            handle.removeEventListener("lostpointercapture", finishGeometry);
            if (finishEvent.type === "pointerup") {
                const centerPhysical = currentMonitor
                    ? clientPointToPhysical({ x: preview.left + preview.width / 2, y: preview.top + preview.height / 2 }, currentMonitor)
                    : undefined; // 围栏中心在虚拟桌面中的物理坐标。
                const targetMonitor = centerPhysical ? monitorAtPhysicalPoint(centerPhysical, monitors) : currentMonitor; // 围栏最终归属的显示器。
                const targetCenter = centerPhysical && targetMonitor ? physicalPointToMonitorDip(centerPhysical, targetMonitor) : undefined; // 围栏中心在目标显示器中的 DIP。
                const targetBounds = targetMonitor
                    ? moveFence(
                          {
                              ...preview,
                              left: (targetCenter?.left ?? preview.left + preview.width / 2) - preview.width / 2,
                              top: (targetCenter?.top ?? preview.top + preview.height / 2) - preview.height / 2,
                          },
                          { x: 0, y: 0 },
                          { width: targetMonitor.workWidth / targetMonitor.scaleFactor, height: targetMonitor.workHeight / targetMonitor.scaleFactor },
                      )
                    : preview; // 保持逻辑尺寸并约束到目标显示器工作区的最终边界。
                const targetViewport = targetMonitor
                    ? { width: targetMonitor.workWidth / targetMonitor.scaleFactor, height: targetMonitor.workHeight / targetMonitor.scaleFactor }
                    : { width: window.innerWidth, height: window.innerHeight };
                const snappedTargetBounds = snapFenceEdges(
                    targetBounds,
                    targetMonitor?.id === currentMonitor?.id ? snapTargets : [],
                    targetViewport,
                    8,
                    direction
                        ? { left: direction.includes("w"), right: direction.includes("e"), top: direction.includes("n"), bottom: direction.includes("s") }
                        : undefined,
                );
                void onCommand({ type: "update-fence-bounds", fenceId: fence.id, bounds: snappedTargetBounds }, targetMonitor?.id)
                    .catch(() => applyFenceBounds(element, startBounds, fence.collapsed))
                    .finally(() => onEndInputDrag().catch(() => undefined));
                return;
            }
            applyFenceBounds(element, startBounds, fence.collapsed);
            void onEndInputDrag().catch(() => undefined);
        };

        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishGeometry);
        handle.addEventListener("pointercancel", finishGeometry);
        handle.addEventListener("lostpointercapture", finishGeometry);
    };

    /** 将当前原生滚动位置立即提交到权威工作区。 */
    const handleScroll = () => {
        if (contentRef.current?.dataset.dragScrolling === "true") return;
        updateScrollbar();
        void onCommand({ type: "set-fence-scroll", fenceId: fence.id, scrollTop: contentRef.current?.scrollTop ?? 0 }).catch(() => undefined);
    };

    /** 按原型步进滚动围栏内容。 */
    const scrollBy = (offset: number) => contentRef.current?.scrollBy({ top: offset });

    /** 拖动覆盖式滚动条滑块并同步内容位置。 */
    const handleThumbPointerDown = (event: PointerEvent<HTMLDivElement>) => {
        if (event.button !== 0 || scrollbar.maxScrollTop <= 0 || scrollbar.trackHeight <= scrollbar.thumbHeight) return;
        event.preventDefault();
        event.stopPropagation();
        const thumb = event.currentTarget;
        const startPointerY = event.clientY;
        const startScrollTop = contentRef.current?.scrollTop ?? 0;
        const maxThumbTop = scrollbar.trackHeight - scrollbar.thumbHeight;
        let finished = false;
        thumb.setPointerCapture(event.pointerId);
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            if (contentRef.current)
                contentRef.current.scrollTop = startScrollTop + ((moveEvent.clientY - startPointerY) / maxThumbTop) * scrollbar.maxScrollTop;
        };
        const finish = (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            if (finishEvent.type !== "lostpointercapture" && thumb.hasPointerCapture(finishEvent.pointerId)) thumb.releasePointerCapture(finishEvent.pointerId);
            thumb.removeEventListener("pointermove", handlePointerMove);
            thumb.removeEventListener("pointerup", finish);
            thumb.removeEventListener("pointercancel", finish);
            thumb.removeEventListener("lostpointercapture", finish);
            handleScroll();
        };
        thumb.addEventListener("pointermove", handlePointerMove);
        thumb.addEventListener("pointerup", finish);
        thumb.addEventListener("pointercancel", finish);
        thumb.addEventListener("lostpointercapture", finish);
    };

    /** 点击覆盖式滚动条轨道分页滚动。 */
    const handleTrackPointerDown = (event: PointerEvent<HTMLDivElement>) => {
        if (event.button !== 0 || event.target !== event.currentTarget || !contentRef.current) return;
        const thumbTop = scrollbar.thumbTop + event.currentTarget.getBoundingClientRect().top;
        scrollBy(event.clientY < thumbTop ? -contentRef.current.clientHeight : contentRef.current.clientHeight);
    };

    /** 清空项目选择并将键盘焦点交给当前围栏标题。 */
    const focusFence = () => {
        fenceRef.current?.focus({ preventScroll: true });
        void onInteraction({ type: "focus-surface", focus: { type: "fence", fenceId: fence.id } }).catch(() => undefined);
    };

    return (
        <section
            ref={fenceRef}
            className={`fence${fence.collapsed ? " fence--collapsed" : ""}`}
            data-fence-id={fence.id}
            data-layout-mode={fence.layoutMode}
            data-hit-id={fence.id}
            data-hit-kind="fence"
            data-hit-priority="50"
            data-keyboard-focus={keyboardFocus?.type === "fence" && keyboardFocus.fenceId === fence.id ? "true" : undefined}
            tabIndex={-1}
            style={{
                left: fence.bounds.left,
                top: fence.bounds.top,
                width: fence.bounds.width,
                height: fence.collapsed ? 28 : fence.bounds.height,
                minWidth: FENCE_MIN_WIDTH,
                minHeight: fence.collapsed ? 28 : FENCE_MIN_HEIGHT,
                zIndex: FENCE_Z_INDEX_BASE + fence.zIndex,
            }}
            aria-label={`围栏 ${fence.title}`}
            onContextMenu={(event) => onFenceContextMenu(event, fence.id)}
        >
            <header
                className="fence__header"
                onPointerDown={(event) => {
                    if (event.target instanceof Element && event.target.closest("button")) return;
                    focusFence();
                    void beginGeometry(event);
                }}
            >
                <h2>{fence.title}</h2>
                <div className="fence__commands">
                    <button
                        type="button"
                        title={fence.layoutMode === "tile" ? "切换为列表" : "切换为平铺"}
                        onClick={() =>
                            void onCommand({
                                type: "set-fence-layout",
                                fenceId: fence.id,
                                layoutMode: fence.layoutMode === "tile" ? "list" : "tile",
                            }).catch(() => undefined)
                        }
                    >
                        {/* 仅内联仓库中可信的原型 SVG，使 currentColor 继承系统主题。 */}
                        <span
                            aria-hidden="true"
                            dangerouslySetInnerHTML={{ __html: fence.layoutMode === "tile" ? fenceLayoutListIcon : fenceLayoutTileIcon }}
                        />
                    </button>
                    <button
                        type="button"
                        title={fence.collapsed ? "展开围栏" : "折叠围栏"}
                        onClick={() => void onCommand({ type: "set-fence-collapsed", fenceId: fence.id, collapsed: !fence.collapsed }).catch(() => undefined)}
                    >
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fence.collapsed ? fenceToggleExpandIcon : fenceToggleCollapseIcon }} />
                    </button>
                </div>
            </header>

            {!fence.collapsed && (
                <div className="fence__body">
                    <ul
                        ref={contentRef}
                        className="fence__items"
                        data-fence-content-id={fence.id}
                        onScroll={handleScroll}
                        onPointerDown={(event) => {
                            if (event.target === event.currentTarget || (event.target instanceof Element && event.target.closest(".fence__empty")))
                                focusFence();
                        }}
                    >
                        {items.map((item) => (
                            <DesktopItemTile
                                key={item.key}
                                item={item}
                                desktopHost
                                className="desktop-item--fence"
                                fenceMemberId={item.key}
                                onItemInput={onItemInput}
                                onItemPointerDown={(event) => onItemPointerDown(event, item)}
                                onItemDoubleClick={() => void onOpenItems([item.key]).catch(() => undefined)}
                                onItemContextMenu={(event) => onItemContextMenu(event, item.key)}
                                selected={selectedItemIds.has(item.key)}
                                keyboardFocused={keyboardFocus?.type === "item" && keyboardFocus.itemId === item.key}
                                renaming={renameItemId === item.key}
                                renameValue={renameValue}
                                onRenameValueChange={onRenameValueChange}
                                onRenameCommit={onRenameCommit}
                                onRenameCancel={onRenameCancel}
                            />
                        ))}
                    </ul>
                    <div className="fence__scrollbar" hidden={scrollbar.maxScrollTop <= 1}>
                        <button type="button" className="fence__scroll-button" title="向上滚动" aria-label="向上滚动" onClick={() => scrollBy(-46)}>
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fenceToggleCollapseIcon }} />
                        </button>
                        <div className="fence__scroll-track" onPointerDown={handleTrackPointerDown}>
                            <div
                                className="fence__scroll-thumb"
                                role="scrollbar"
                                tabIndex={0}
                                aria-label="围栏滚动位置"
                                aria-valuemin={0}
                                aria-valuemax={Math.round(scrollbar.maxScrollTop)}
                                aria-valuenow={Math.round(contentRef.current?.scrollTop ?? 0)}
                                style={{ height: scrollbar.thumbHeight, transform: `translateY(${scrollbar.thumbTop}px)` }}
                                onPointerDown={handleThumbPointerDown}
                                onKeyDown={(event) => {
                                    if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"].includes(event.key)) return;
                                    event.preventDefault();
                                    if (event.key === "Home") scrollBy(-scrollbar.maxScrollTop);
                                    else if (event.key === "End") scrollBy(scrollbar.maxScrollTop);
                                    else
                                        scrollBy(
                                            (event.key.endsWith("Up") ? -1 : 1) * (event.key.startsWith("Page") ? (contentRef.current?.clientHeight ?? 0) : 46),
                                        );
                                }}
                            />
                        </div>
                        <button type="button" className="fence__scroll-button" title="向下滚动" aria-label="向下滚动" onClick={() => scrollBy(46)}>
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fenceToggleExpandIcon }} />
                        </button>
                    </div>
                </div>
            )}

            {!fence.collapsed &&
                RESIZE_DIRECTIONS.map((direction) => (
                    <span
                        className={`fence__resize fence__resize--${direction}`}
                        key={direction}
                        aria-hidden="true"
                        onPointerDown={(event) => void beginGeometry(event, direction)}
                    />
                ))}
        </section>
    );
};

/** 呈现正式透明宿主中的未归类区域与全部围栏。 */
export const FenceWorkspace = ({
    desktopItems,
    workspace,
    completeWorkspace,
    currentMonitor,
    monitors,
    hostSnapshot,
    failure,
    status,
    onCommand,
    onInteraction,
    onOpenItems,
    onRenameItem,
    onDeleteItems,
    onSetClipboard,
    onPasteItems,
    onStartShellDrag,
    onContextMenu,
    onItemInput,
    onBeginInputDrag,
    onEndInputDrag,
}: FenceWorkspaceProps) => {
    const workspaceRef = useRef<HTMLElement>(null); // 当前完整工作区交互表面。
    const contextMenuOpenRef = useRef(false); // 原生 owner HWND 打开期间阻止窗口失焦清空选择。
    const [dragFailure, setDragFailure] = useState<string>(); // 当前项目拖动取消或容量错误说明。
    const [interactionStatus, setInteractionStatus] = useState<string>(); // 当前无需调用 Shell 的键盘操作提示。
    const [fenceMenu, setFenceMenu] = useState<{ fenceId: string; left: number; top: number }>(); // 当前原型围栏对象菜单。
    const [renameItemId, setRenameItemId] = useState<string>(); // 当前 F2 原位编辑的单一项目引用。
    const [renameValue, setRenameValue] = useState(""); // 当前重命名输入框中的用户文本。
    // 当前 Shell 项目对应的页面展示模型。
    const items = useMemo(() => (desktopItems ? projectDesktopItems(desktopItems) : []), [desktopItems]);
    // 当前稳定引用对应的桌面项目展示模型。
    const itemsById = useMemo(() => new Map(items.map((item) => [item.key, item])), [items]);
    // 当前全部围栏成员标识。
    const memberIds = useMemo(() => new Set(workspace?.fences.flatMap((fence) => fence.members) ?? []), [workspace]);
    // 当前保留的未归类项目来源位置。
    const sourceItems = useMemo(() => new Map(workspace?.unclassifiedItems.map((item) => [item.stableReference, item]) ?? []), [workspace]);
    // 当前权威选择集合的快速查询表。
    const selectedItemIds = useMemo(() => new Set(workspace?.interaction.selectedItemIds ?? []), [workspace]);
    // 当前未归属围栏的桌面项目。
    const unclassifiedItems = items.filter((item) => !memberIds.has(item.key));
    const readyHostCount =
        hostSnapshot?.hosts.filter(({ readiness }) => readiness.guardian && readiness.page && readiness.systemImages && readiness.pixels && readiness.hitMap)
            .length ?? 0; // 已满足自动接管门槛的逐屏宿主数量。

    /** 进入单项 F2 原位重命名状态。 */
    const handleBeginRename = (itemId: string) => {
        const item = itemsById.get(itemId); // 当前重命名项目的 Shell 展示模型。
        if (!item?.isAvailable) return;
        setInteractionStatus(undefined);
        setRenameItemId(itemId);
        setRenameValue(item.label);
    };

    /** 将原位编辑文本提交给 Shell，失败时保留编辑状态。 */
    const handleRenameCommit = () => {
        if (!renameItemId) return;
        const newName = renameValue.trim(); // 当前提交给 Shell 的项目名称。
        const currentName = itemsById.get(renameItemId)?.label; // Shell 快照中的当前显示名称。
        if (!newName || newName === currentName) {
            setRenameItemId(undefined);
            return;
        }
        void onRenameItem(renameItemId, newName).then(() => setRenameItemId(undefined));
    };

    /** 取消当前原位重命名并保持项目选择。 */
    const handleRenameCancel = () => setRenameItemId(undefined);

    /** 使用同步打开标记承载原生菜单生命周期。 */
    const handleContextMenu = async (target: WorkspaceContextMenuTarget, clientX: number, clientY: number) => {
        contextMenuOpenRef.current = true;
        try {
            return await onContextMenu(target, clientX, clientY);
        } finally {
            contextMenuOpenRef.current = false;
            workspaceRef.current?.focus({ preventScroll: true });
        }
    };

    useEffect(() => {
        /** 在菜单外部发生指针输入时关闭围栏菜单。 */
        const handleDocumentPointerDown = (event: globalThis.PointerEvent) => {
            if (event.target instanceof Element && event.target.closest(".fence-object-menu")) return;
            setFenceMenu(undefined);
        };
        /** 使用 Escape 关闭当前围栏菜单。 */
        const handleDocumentKeyDown = (event: globalThis.KeyboardEvent) => {
            if (event.key === "Escape") setFenceMenu(undefined);
        };
        /** 窗口失去焦点时关闭当前围栏菜单。 */
        const handleWindowBlur = () => setFenceMenu(undefined);
        document.addEventListener("pointerdown", handleDocumentPointerDown, true);
        document.addEventListener("keydown", handleDocumentKeyDown, true);
        window.addEventListener("blur", handleWindowBlur);
        return () => {
            document.removeEventListener("pointerdown", handleDocumentPointerDown, true);
            document.removeEventListener("keydown", handleDocumentKeyDown, true);
            window.removeEventListener("blur", handleWindowBlur);
        };
    }, []);

    const { handleWorkspacePointerDown, handleWorkspacePointerMove, handleWorkspaceContextMenu, handleWorkspaceKeyDown } = useWorkspaceInteraction({
        workspaceRef,
        workspace,
        unclassifiedItemIds: unclassifiedItems.map((item) => item.key),
        onCommand,
        onInteraction,
        onOpenItems,
        onBeginRename: handleBeginRename,
        onRenameUnavailable: () => setInteractionStatus("当前选择不支持批量重命名"),
        onDeleteItems,
        onSetClipboard,
        onPasteItems,
        onContextMenu: handleContextMenu,
        isContextMenuOpen: () => contextMenuOpenRef.current,
    });

    const handleItemPointerDown = useWorkspaceDrag({
        workspaceRef,
        workspace,
        completeWorkspace,
        currentMonitor,
        monitors,
        selectedItemIds,
        onCommand,
        onInteraction,
        onBeginInputDrag,
        onEndInputDrag,
        onStartShellDrag,
        onFailure: setDragFailure,
    });

    /** 在当前选择集合上显示单项或多选 Shell 项目菜单。 */
    const handleItemContextMenu = async (event: MouseEvent<HTMLLIElement>, itemId: string) => {
        event.preventDefault();
        event.stopPropagation();
        workspaceRef.current?.focus({ preventScroll: true });
        const itemIds = selectedItemIds.has(itemId) ? (workspace?.interaction.selectedItemIds ?? [itemId]) : [itemId]; // 当前右键目标对应的完整菜单选择集合。
        try {
            await onInteraction({ type: "focus-item", itemId, selectionMode: selectedItemIds.has(itemId) ? "preserve" : "replace" });
        } catch {
            return;
        }
        await handleContextMenu({ type: "items", itemIds }, event.clientX, event.clientY).catch(() => undefined);
    };

    /** 在当前围栏标题位置显示原型对象菜单。 */
    const handleFenceContextMenu = (event: MouseEvent<HTMLElement>, fenceId: string) => {
        event.preventDefault();
        event.stopPropagation();
        workspaceRef.current?.focus({ preventScroll: true });
        void onInteraction({ type: "focus-surface", focus: { type: "fence", fenceId } }).catch(() => undefined);
        setFenceMenu({ fenceId, left: event.clientX, top: event.clientY });
    };

    return (
        <main
            ref={workspaceRef}
            className="fence-workspace"
            data-hit-id="workspace-background"
            data-hit-kind="workspace-background"
            data-hit-priority="1"
            data-keyboard-focus={workspace?.interaction.keyboardFocus.type === "background" ? "true" : undefined}
            tabIndex={0}
            onKeyDown={handleWorkspaceKeyDown}
            onPointerDown={handleWorkspacePointerDown}
            onPointerMoveCapture={handleWorkspacePointerMove}
            onContextMenu={handleWorkspaceContextMenu}
        >
            <aside
                className="fence-workspace__test-panel"
                data-hit-id="workspace-test-panel"
                data-hit-kind="component"
                data-hit-priority="90"
                aria-label="桌面宿主测试信息"
            >
                <strong>Desktop Tools Tauri/WebView2 正式宿主</strong>
                <p>当前平台桌面接管状态与测试操作</p>
                <dl className="fence-workspace__test-state">
                    <div>
                        <dt>平台</dt>
                        <dd>
                            Windows 10 22H2 · {currentMonitor?.name ?? "正在发现显示器"} · DPI {currentMonitor?.dpi ?? "--"}
                        </dd>
                    </div>
                    <div>
                        <dt>宿主</dt>
                        <dd>Windowed WebView2 · {hostSnapshot?.hosts.length ?? 0} 屏</dd>
                    </div>
                    <div>
                        <dt>阶段</dt>
                        <dd>{hostSnapshot?.phase ?? "starting"}</dd>
                    </div>
                    <div>
                        <dt>就绪</dt>
                        <dd>
                            {readyHostCount} / {hostSnapshot?.hosts.length ?? 0}
                        </dd>
                    </div>
                    <div>
                        <dt>项目</dt>
                        <dd>{items.length} 个真实项目</dd>
                    </div>
                    <div>
                        <dt>围栏</dt>
                        <dd>
                            {workspace?.fences.length ?? 0} 个 · 布局修订 {workspace?.revision ?? "--"}
                        </dd>
                    </div>
                </dl>
                <div className="fence-workspace__test-actions">
                    <button type="button" disabled>
                        {hostSnapshot?.committed ? "接管已提交" : "等待自动接管"}
                    </button>
                    <button type="button" onClick={() => window.location.reload()}>
                        重载渲染器
                    </button>
                    <button className="fence-workspace__test-exit" type="button" onClick={() => void exitDesktopTools()}>
                        退出应用
                    </button>
                </div>
            </aside>

            {(failure ?? dragFailure) && (
                <p className="fence-workspace__error" role="alert" data-hit-id="workspace-error" data-hit-kind="workspace-status" data-hit-priority="80">
                    {failure ?? dragFailure}
                </p>
            )}

            {(status ?? interactionStatus) && !failure && !dragFailure && (
                <p className="fence-workspace__status" role="status" data-hit-id="workspace-status" data-hit-kind="workspace-status" data-hit-priority="80">
                    {status ?? interactionStatus}
                </p>
            )}

            <ul className="fence-workspace__unclassified" aria-label="未归类区域" onPointerDown={handleWorkspacePointerDown}>
                {unclassifiedItems.map((item) => {
                    // 项目持久化位置或首次呈现的 Windows 风格纵向网格位置。
                    const source = sourceItems.get(item.key);
                    if (!source) return null;
                    return (
                        <DesktopItemTile
                            key={item.key}
                            item={item}
                            desktopHost
                            className="desktop-item--unclassified"
                            style={{ left: source.position.left, top: source.position.top, zIndex: source.zIndex }}
                            onItemInput={onItemInput}
                            onItemPointerDown={(event) => void handleItemPointerDown(event, item.key)}
                            onItemDoubleClick={() => void onOpenItems([item.key]).catch(() => undefined)}
                            onItemContextMenu={(event) => void handleItemContextMenu(event, item.key)}
                            selected={selectedItemIds.has(item.key)}
                            keyboardFocused={workspace?.interaction.keyboardFocus.type === "item" && workspace.interaction.keyboardFocus.itemId === item.key}
                            renaming={renameItemId === item.key}
                            renameValue={renameValue}
                            onRenameValueChange={setRenameValue}
                            onRenameCommit={handleRenameCommit}
                            onRenameCancel={handleRenameCancel}
                        />
                    );
                })}
            </ul>

            {fenceMenu && workspace?.fences.find((fence) => fence.id === fenceMenu.fenceId) && (
                <menu
                    className="fence-object-menu"
                    style={{ left: Math.min(fenceMenu.left, window.innerWidth - 172), top: Math.min(fenceMenu.top, window.innerHeight - 180) }}
                    onPointerDown={(event) => event.stopPropagation()}
                >
                    <li>
                        <button type="button" disabled title="正式领域暂未提供围栏重命名命令">
                            重命名
                        </button>
                    </li>
                    <li>
                        <button
                            type="button"
                            onClick={() => {
                                const fence = workspace?.fences.find((item) => item.id === fenceMenu.fenceId);
                                if (fence)
                                    void onCommand({ type: "set-fence-collapsed", fenceId: fence.id, collapsed: !fence.collapsed }).catch(() => undefined);
                                setFenceMenu(undefined);
                            }}
                        >
                            {workspace?.fences.find((fence) => fence.id === fenceMenu.fenceId)?.collapsed ? "展开" : "折叠"}
                        </button>
                    </li>
                    <li>
                        <button type="button" disabled title="正式领域暂未提供围栏排序命令">
                            排序方式
                        </button>
                    </li>
                    <li>
                        <button type="button" disabled title="正式领域暂未提供布局编辑命令">
                            编辑布局
                        </button>
                    </li>
                    <li>
                        <button
                            type="button"
                            onClick={() => {
                                void onCommand({ type: "dissolve-fence", fenceId: fenceMenu.fenceId }).catch(() => undefined);
                                setFenceMenu(undefined);
                            }}
                        >
                            删除围栏
                        </button>
                    </li>
                </menu>
            )}

            {workspace?.fences.map((fence) => (
                <FencePanel
                    key={fence.id}
                    fence={fence}
                    snapTargets={workspace.fences.filter((candidate) => candidate.id !== fence.id).map((candidate) => candidate.bounds)}
                    items={fence.members.flatMap((itemId) => {
                        const item = itemsById.get(itemId); // Shell 当前可解析或保留的围栏成员。
                        return item ? [item] : [];
                    })}
                    onCommand={onCommand}
                    onInteraction={onInteraction}
                    onFenceContextMenu={handleFenceContextMenu}
                    onItemContextMenu={handleItemContextMenu}
                    onOpenItems={onOpenItems}
                    onItemPointerDown={(event, item) => void handleItemPointerDown(event, item.key)}
                    onItemInput={onItemInput}
                    onBeginInputDrag={onBeginInputDrag}
                    onEndInputDrag={onEndInputDrag}
                    currentMonitor={currentMonitor}
                    monitors={monitors}
                    selectedItemIds={selectedItemIds}
                    keyboardFocus={workspace.interaction.keyboardFocus}
                    renameItemId={renameItemId}
                    renameValue={renameValue}
                    onRenameValueChange={setRenameValue}
                    onRenameCommit={handleRenameCommit}
                    onRenameCancel={handleRenameCancel}
                />
            ))}

            <OfficialComponentHost
                instances={workspace?.components ?? []}
                currentMonitor={currentMonitor}
                monitors={monitors}
                onCommand={onCommand}
                onInteraction={onInteraction}
                onBeginInputDrag={onBeginInputDrag}
                onEndInputDrag={onEndInputDrag}
            />
        </main>
    );
};
