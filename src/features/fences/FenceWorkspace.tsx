import { useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent, type ReactNode } from "react";

import type { DesktopItemSnapshot } from "../../contracts/desktopItem";
import fenceLayoutListIcon from "../../../resources/fence-layout-list.svg?raw";
import fenceLayoutTileIcon from "../../../resources/fence-layout-tile.svg?raw";
import fenceSortIcon from "../../../resources/fence-sort.svg?raw";
import fenceToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import fenceToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import { DebugTest } from "../components/DebugTest";
import { MessageNotificationHost } from "../notifications/MessageNotificationHost";
import { publishMessage } from "../notifications/messageNotification";

import type { DesktopHostMonitor, DesktopHostSnapshot } from "../../contracts/desktopHost";
import {
    FENCE_MIN_HEIGHT,
    FENCE_MIN_WIDTH,
    type FenceSnapshot,
    type WorkspaceCommand,
    type WorkspacePoint,
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

import "./FenceWorkspace.css";
import { DesktopTileBase } from "../tiles/DesktopTileBase";
import { DesktopTileHeader } from "../tiles/DesktopTileHeader";
import { DesktopTileViewport } from "../tiles/DesktopTileViewport";
import { useDesktopTileGeometry } from "../tiles/useDesktopTileGeometry";
import { useDesktopTileExpansion } from "../tiles/useDesktopTileExpansion";
import type { TileAppearance, TileAppearanceChange } from "../../contracts/tile";

/** 围栏工作区接收的权威快照与命令边界。 */
interface FenceWorkspaceProps {
    children?: ReactNode; // 共享工作区主题与命中图的附加内容。
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
    onStartShellDrag: (token: number, pointerOffset: WorkspacePoint) => Promise<void>;
    onContextMenu: (target: WorkspaceContextMenuTarget, clientX: number, clientY: number) => Promise<WorkspaceSnapshot>;
    onItemInput?: (itemId: string) => void;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
}

/** 单个围栏的呈现与交互参数。 */
interface FencePanelProps {
    appearance?: TileAppearance; // 当前共享外观。
    onRenameStart: () => void; // 开始标题编辑。
    renaming: boolean; // 是否编辑围栏标题。
    onRenameEnd: () => void; // 结束标题编辑。
    fence: FenceSnapshot;
    items: DesktopItemViewModel[];
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;

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

const FENCE_Z_INDEX_BASE = 1000; // 围栏始终位于未归类图标之上，拖动预览使用更高层级。

/** 呈现一个可移动、缩放、折叠、滚动和接收项目的围栏。 */
const FencePanel = ({
    fence: persistedFence,
    appearance: savedAppearance,
    onRenameStart,
    renaming,
    onRenameEnd,
    items,
    onCommand,
    onInteraction,

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
    const fenceRef = useRef<HTMLElement>(null); // 公共格子表面。
    const appearance: TileAppearance = savedAppearance ?? {
        title: persistedFence.title,
        titleMode: "fixed",
        buttonMode: "hover",
        collapsed: persistedFence.collapsed,
        autoExpand: persistedFence.autoExpand ?? false,
    }; // 未设置时保留已有围栏数据。
    const expansion = useDesktopTileExpansion(fenceRef, appearance.collapsed, appearance.autoExpand, currentMonitor?.id); // 临时展开不保存。
    const fence = { ...persistedFence, title: appearance.title ?? persistedFence.title, collapsed: expansion.collapsed }; // 内容的有效呈现状态。
    const contentRef = useRef<HTMLUListElement>(null); // 内容滚动表面。
    const handleAppearance = (change: TileAppearanceChange) => {
        void onCommand({ type: "set-tile-appearance", target: { type: "fence", fenceId: fence.id }, change }).catch(() => undefined);
    }; // 单字段外观命令。
    const sortItems = [
        {
            label: `按名称排序${fence.sortMode === "name-ascending" ? "（升序）" : fence.sortMode === "name-descending" ? "（降序）" : ""}`,
            radio: true,
            checked: Boolean(fence.sortMode && fence.sortMode !== "none"),
            onSelect: () =>
                void onCommand({
                    type: "set-fence-sort",
                    fenceId: fence.id,
                    sortMode: fence.sortMode === "name-ascending" ? "name-descending" : "name-ascending",
                }).catch(() => undefined),
        },
        {
            label: "无排序",
            radio: true,
            checked: !fence.sortMode || fence.sortMode === "none",
            onSelect: () => void onCommand({ type: "set-fence-sort", fenceId: fence.id, sortMode: "none" }).catch(() => undefined),
        },
    ]; // 两个排序入口使用同一内容。
    const scrollSync = useRef({ running: false, requested: undefined as number | undefined }); // 合并滚动写入，防止旧快照拉回正在滚动的内容。
    useEffect(() => {
        if (contentRef.current && !scrollSync.current.running) contentRef.current.scrollTop = fence.scrollTop;
    }, [fence.scrollTop, fence.collapsed]);

    const { handlePointerDown: beginGeometry, handleKeyDown: handleGeometryKey } = useDesktopTileGeometry({
        surfaceRef: fenceRef,
        bounds: fence.bounds,
        collapsed: fence.collapsed,
        title: fence.title,
        minimum: { width: FENCE_MIN_WIDTH, height: FENCE_MIN_HEIGHT },
        snap: true,
        currentMonitor,
        monitors,
        onFocus: () => focusFence(),
        onCommit: (bounds, monitorId) => onCommand({ type: "update-fence-bounds", fenceId: fence.id, bounds }, monitorId),
        onBeginInputDrag,
        onEndInputDrag,
        onFailure: (error) =>
            publishMessage({
                type: "error",
                content:
                    error instanceof Error
                        ? error.message
                        : error && typeof error === "object" && "message" in error
                          ? String(error.message)
                          : "格子几何操作失败",
                source: "desktop-tile",
            }),
    }); // 公共几何沿用原型调用路径。

    /** 将当前原生滚动位置立即提交到权威工作区。 */
    const handleScroll = () => {
        if (contentRef.current?.dataset.dragScrolling === "true") return;
        scrollSync.current.requested = contentRef.current?.scrollTop ?? 0;
        if (scrollSync.current.running) return;
        scrollSync.current.running = true;
        void (async () => {
            try {
                while (scrollSync.current.requested !== undefined) {
                    const scrollTop = scrollSync.current.requested; // 当前最新滚动位置，同一时刻只允许一个写请求。
                    scrollSync.current.requested = undefined;
                    await onCommand({ type: "set-fence-scroll", fenceId: fence.id, scrollTop });
                }
            } finally {
                scrollSync.current.running = false;
            }
        })().catch(() => undefined);
    };

    /** 清空项目选择并将键盘焦点交给当前围栏标题。 */
    const focusFence = () => {
        fenceRef.current?.focus({ preventScroll: true });
        void onInteraction({ type: "focus-surface", focus: { type: "fence", fenceId: fence.id } }).catch(() => undefined);
    };

    return (
        <DesktopTileBase
            ref={fenceRef}
            collapsed={fence.collapsed}
            appearance={appearance}
            renaming={renaming}
            onRenameStart={onRenameStart}
            onAppearanceChange={handleAppearance}
            sortItems={sortItems}
            menuItems={[
                { label: "排序方式", separatorBefore: true, children: sortItems },
                {
                    label: "解除围栏",
                    separatorBefore: true,
                    onSelect: () => void onCommand({ type: "dissolve-fence", fenceId: fence.id }).catch(() => undefined),
                },
            ]}
            onResize={(event, direction) => void beginGeometry(event, direction)}
            onKeyDown={handleGeometryKey}
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
            onPointerEnter={expansion.onPointerEnter}
            onPointerLeave={expansion.onPointerLeave}
            onContextMenu={() => focusFence()}
        >
            <DesktopTileHeader
                title={fence.title}
                onRenameStart={onRenameStart}
                editing={renaming}
                onRenameEnd={onRenameEnd}
                onRename={(title) => onCommand({ type: "rename-fence", fenceId: fence.id, title })}
                onPointerDown={(event) => {
                    if (event.target instanceof Element && event.target.closest("button,input")) return;
                    focusFence();
                    void beginGeometry(event);
                }}
                commands={(sortMenuOpen) => (
                    <>
                        <button
                            type="button"
                            className="fence__sort"
                            data-sort-active={Boolean(fence.sortMode && fence.sortMode !== "none")}
                            title="排序方式"
                            aria-label="排序方式"
                            aria-haspopup="menu"
                            aria-expanded={sortMenuOpen}

                            onClick={(event) => {
                                const bounds = event.currentTarget.getBoundingClientRect();
                                event.currentTarget.closest(".desktop-tile")?.dispatchEvent(
                                    new CustomEvent("desktop-tile-menu", {
                                        detail: { left: bounds.left, top: bounds.bottom, sorting: true, trigger: event.currentTarget },
                                    }),
                                );
                            }}
                        >
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fenceSortIcon }} />
                        </button>
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
                                dangerouslySetInnerHTML={{ __html: fence.layoutMode === "tile" ? fenceLayoutTileIcon : fenceLayoutListIcon }}
                            />
                        </button>
                        <button
                            type="button"
                            title={persistedFence.collapsed ? "展开围栏" : "折叠围栏"}
                            onClick={() =>
                                void onCommand({ type: "set-fence-collapsed", fenceId: fence.id, collapsed: !persistedFence.collapsed }).catch(() => undefined)
                            }
                        >
                            <span
                                aria-hidden="true"
                                dangerouslySetInnerHTML={{ __html: persistedFence.collapsed ? fenceToggleExpandIcon : fenceToggleCollapseIcon }}
                            />
                        </button>
                    </>
                )}
            />

            <DesktopTileViewport
                contentRef={contentRef}
                as="ul"
                collapsed={fence.collapsed}
                className="fence__items"
                data-fence-content-id={fence.id}
                onScroll={handleScroll}
                onPointerDown={(event) => {
                    if (event.target === event.currentTarget || (event.target instanceof Element && event.target.closest(".fence__empty"))) focusFence();
                }}
            >
                {items.map((item) => (
                    <DesktopItemTile
                        key={item.key}
                        item={item}
                        desktopHost
                        className={`desktop-item--fence${fence.layoutMode === "list" ? " desktop-item--list" : ""}`}
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
            </DesktopTileViewport>
        </DesktopTileBase>
    );
};

/** 呈现正式透明宿主中的未归类区域与全部围栏。 */
export const FenceWorkspace = ({
    children,
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
    // 将原有工作区反馈送入统一消息入口，避免在围栏内重复呈现。
    useEffect(() => {
        const content = failure ?? dragFailure; // 当前优先呈现的操作错误。
        if (content) publishMessage({ type: "error", content, source: "workspace", dedupeKey: content });
    }, [failure, dragFailure]);
    useEffect(() => {
        const content = status ?? interactionStatus; // 当前 Shell 或交互操作反馈。
        if (content) publishMessage({ type: "info", content, source: "workspace", dedupeKey: content });
    }, [status, interactionStatus]);
    const [renameItemId, setRenameItemId] = useState<string>(); // 当前 F2 原位编辑的单一项目引用。
    const [renameFenceId, setRenameFenceId] = useState<string>(); // 当前编辑标题的围栏。
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
        if (target.type === "fence" && workspace) {
            workspaceRef.current
                ?.querySelector<HTMLElement>(`[data-fence-id="${CSS.escape(target.fenceId)}"]`)
                ?.dispatchEvent(new CustomEvent("desktop-tile-menu", { detail: { left: clientX, top: clientY } }));
            return workspace;
        }
        contextMenuOpenRef.current = true;
        try {
            return await onContextMenu(target, clientX, clientY);
        } catch (error) {
            const message =
                error instanceof Error ? error.message : error && typeof error === "object" && "message" in error ? String(error.message) : "原生菜单请求失败"; // 不再让外层静默 catch 隐藏真实失败。
            publishMessage({ type: "error", content: message, source: "workspace-menu" });
            throw error;
        } finally {
            contextMenuOpenRef.current = false;
            if (target.type !== "background") workspaceRef.current?.focus({ preventScroll: true });
        }
    };

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
        onOpenItems,
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
            <MessageNotificationHost />
            {children}

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

            {workspace?.fences.map((fence) => (
                <FencePanel
                    key={fence.id}
                    appearance={workspace.tileAppearances?.[`fence:${fence.id}`]}
                    onRenameStart={() => setRenameFenceId(fence.id)}
                    renaming={renameFenceId === fence.id}
                    onRenameEnd={() => setRenameFenceId(undefined)}
                    fence={fence}
                    items={fence.members.flatMap((itemId) => {
                        const item = itemsById.get(itemId); // Shell 当前可解析或保留的围栏成员。
                        return item ? [item] : [];
                    })}
                    onCommand={onCommand}
                    onInteraction={onInteraction}

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
                tileAppearances={workspace?.tileAppearances}
                keyboardFocus={workspace?.interaction.keyboardFocus}
                debugContent={
                    <DebugTest
                        monitor={currentMonitor}
                        host={hostSnapshot}
                        itemCount={items.length}
                        fenceCount={workspace?.fences.length ?? 0}
                        revision={workspace?.revision}
                    />
                }
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
