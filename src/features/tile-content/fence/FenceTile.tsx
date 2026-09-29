import { useEffect, useRef, type MouseEvent, type PointerEvent } from "react";
import type { DesktopHostMonitor } from "../../../contracts/desktopHost";
import {
    type FenceSnapshot,
    type WorkspaceCommand,
    type WorkspaceInteractionCommand,
    type WorkspaceFocusTarget,
    type WorkspaceRect,
    type WorkspaceSnapshot,
} from "../../../contracts/workspace";
import type { TileAppearance, TileAppearanceChange } from "../../../contracts/tile";
import { DesktopItemTile } from "../../desktop-items/DesktopItemGrid";
import type { DesktopItemViewModel } from "../../desktop-items/desktopItemViewModel";
import { publishMessage } from "../../notifications/messageNotification";
import { DesktopTileBase } from "../../tiles/DesktopTileBase";
import { DesktopTileHeader } from "../../tiles/header/DesktopTileHeader";
import { DesktopTileViewport } from "../../tiles/DesktopTileViewport";
import fenceLayoutListIcon from "../../../../resources/fence-layout-list.svg?raw";
import fenceLayoutTileIcon from "../../../../resources/fence-layout-tile.svg?raw";
import fenceSortIcon from "../../../../resources/fence-sort.svg?raw";
import fenceToggleCollapseIcon from "../../../../resources/fence-toggle-collapse.svg?raw";
import fenceToggleExpandIcon from "../../../../resources/fence-toggle-expand.svg?raw";
import "./FenceTile.css";

/** 单个围栏的呈现与交互参数。 */
interface FenceTileProps {
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
    onMarqueePointerDown: (event: PointerEvent<HTMLElement>) => void; // 空白内容区共用工作区框选入口。
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
export const FenceTile = ({
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
    onMarqueePointerDown,
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
}: FenceTileProps) => {
    const fenceRef = useRef<HTMLElement>(null); // 公共格子表面。
    const appearance: TileAppearance = savedAppearance ?? {
        title: persistedFence.title,
        titleMode: "fixed",
        buttonMode: "hover",
        collapsed: persistedFence.collapsed,
        autoExpand: persistedFence.autoExpand ?? false,
    }; // 未设置时保留已有围栏数据。
    const contentMinimum = { width: 164, height: 110 }; // 围栏内容视口下限，包含内容自身留白，不含公共外壳和标题。
    const minimum = { width: contentMinimum.width + 4, height: contentMinimum.height + 4 + (appearance.titleMode === "fixed" ? 28 : 0) }; // 外壳每侧 1px 边框及 1px 留白，悬浮标题不占内容高度。
    const bounds = persistedFence.projectionBounds
        ? persistedFence.bounds
        : {
              ...persistedFence.bounds,
              width: Math.max(persistedFence.bounds.width, minimum.width),
              height: Math.max(persistedFence.bounds.height, minimum.height),
          }; // 标题切换或小尺寸数据先修正呈现，临时投射仍遵守可用视口，不自动写回布局。
    const fence = { ...persistedFence, bounds, title: appearance.title ?? persistedFence.title, collapsed: appearance.collapsed }; // 内容的有效呈现状态。
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

    const geometry = {
        bounds: fence.bounds,
        title: fence.title,
        minimum,
        snap: true,
        currentMonitor,
        monitors,
        onFocus: () => focusFence(),
        onCommit: (bounds: WorkspaceRect, monitorId?: string) => onCommand({ type: "update-fence-bounds", fenceId: fence.id, bounds }, monitorId),
        onBeginInputDrag,
        onEndInputDrag,
        onFailure: (error: unknown) =>
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
    }; // 围栏只提供业务参数，外框拥有完整几何交互。

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
            geometry={geometry}
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
            className="fence"
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
                height: fence.bounds.height,
                zIndex: FENCE_Z_INDEX_BASE + fence.zIndex,
            }}
            aria-label={`围栏 ${fence.title}`}
            onContextMenu={() => focusFence()}
        >
            <DesktopTileHeader
                title={fence.title}
                onRenameStart={onRenameStart}
                editing={renaming}
                onRenameEnd={onRenameEnd}
                onRename={(title) => onCommand({ type: "rename-fence", fenceId: fence.id, title })}
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
                            {/* 内联仓库内的排序图标，使 currentColor 继承系统主题。 */}
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
                className="fence__items"
                data-fence-content-id={fence.id}
                onScroll={handleScroll}
                onPointerDown={onMarqueePointerDown}
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
