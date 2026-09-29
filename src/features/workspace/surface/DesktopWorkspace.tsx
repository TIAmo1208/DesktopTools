import { useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { DesktopHostMonitor, DesktopHostSnapshot } from "../../../contracts/desktopHost";
import type {
    WorkspaceCommand,
    WorkspacePoint,
    WorkspaceContextMenuTarget,
    WorkspaceInteractionCommand,
    WorkspaceSnapshot,
} from "../../../contracts/workspace";
import { DesktopItemTile } from "../../desktop-items/DesktopItemGrid";
import { projectDesktopItems } from "../../desktop-items/desktopItemViewModel";
import { OfficialComponentHost } from "../../components/OfficialComponentHost";
import { DebugTest } from "../../tile-content/debug/DebugTest";
import { FenceTile } from "../../tile-content/fence/FenceTile";
import { MessageNotificationHost } from "../../notifications/MessageNotificationHost";
import { publishMessage } from "../../notifications/messageNotification";
import { useWorkspaceInteraction } from "../interaction/useWorkspaceInteraction";
import { useWorkspaceDrag } from "../interaction/useWorkspaceDrag";
import "./DesktopWorkspace.css";

/** 跨类型工作区接收的权威快照与命令边界。 */
interface DesktopWorkspaceProps {
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

/** 装配未归类项目、围栏和官方组件，共享工作区选择与 OLE 输入。 */
export const DesktopWorkspace = ({
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
}: DesktopWorkspaceProps) => {
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
                <FenceTile
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
                    onMarqueePointerDown={handleWorkspacePointerDown}
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
