import type { PointerEvent, RefObject } from "react";
import type { DesktopHostMonitor } from "../../../contracts/desktopHost";
import type { ShellDragPreparation } from "../../../contracts/desktopItem";
import { cancelDesktopItemDragPreparation, prepareDesktopItemDrag } from "../../../bridge/desktopItems";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspacePoint, WorkspaceSnapshot } from "../../../contracts/workspace";
import { selectionModeForPointer, shouldOpenItemOnPointerRelease } from "./workspaceInteraction";
import { usePreferences } from "../../settings/PreferencesProvider";
import { itemIdsInWorkspaceVisualOrder } from "./workspaceDrag";

/** 工作区项目拖动需要的权威状态和命令边界。 */
interface WorkspaceDragOptions {
    workspaceRef: RefObject<HTMLElement | null>;
    workspace?: WorkspaceSnapshot;
    completeWorkspace?: WorkspaceSnapshot;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    selectedItemIds: Set<string>;
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
    onStartShellDrag: (token: number, pointerOffset: WorkspacePoint) => Promise<void>;
    onOpenItems: (itemIds: string[]) => Promise<void>;
    onFailure: (failure: string | undefined) => void;
}

/** 管理桌面项目的点击判定和唯一 OLE 拖动交接路径。 */
export const useWorkspaceDrag = ({ workspaceRef, workspace, completeWorkspace, monitors, selectedItemIds, onInteraction, onBeginInputDrag, onEndInputDrag, onStartShellDrag, onOpenItems, onFailure, }: WorkspaceDragOptions) => {
    const { preferences } = usePreferences();

    /** 冻结选择并在越过系统阈值后交接 OLE。 */
    const handleItemPointerDown = async (event: PointerEvent<HTMLLIElement>, anchorItemId: string) => {
        if (event.button !== 0 || !workspace) return;
        event.preventDefault();
        event.stopPropagation();
        onFailure(undefined);
        workspaceRef.current?.focus({ preventScroll: true });
        const handle = event.currentTarget;
        const workspaceElement = workspaceRef.current;
        if (!workspaceElement) return;
        const globalSelectedItemIds = new Set(completeWorkspace?.interaction.selectedItemIds ?? selectedItemIds);
        const preservesSelection = globalSelectedItemIds.has(anchorItemId) && !event.ctrlKey && !event.shiftKey;
        const pointerSelectionMode = preservesSelection ? "replace" : selectionModeForPointer(event);
        const requestedIds = preservesSelection ? globalSelectedItemIds : new Set([anchorItemId]);
        const dragItemIds = itemIdsInWorkspaceVisualOrder(requestedIds, completeWorkspace ?? workspace, monitors);
        const anchorIndex = dragItemIds.indexOf(anchorItemId);
        if (anchorIndex >= 0) dragItemIds.splice(anchorIndex, 1);
        dragItemIds.unshift(anchorItemId);
        const bounds = handle.getBoundingClientRect();
        const pointerOffset = { left: event.clientX - bounds.left, top: event.clientY - bounds.top };
        const openingModifiers = { ctrlKey: event.ctrlKey, shiftKey: event.shiftKey, altKey: event.altKey, metaKey: event.metaKey };
        try {
            await onBeginInputDrag();
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        let finished = false;
        let moved = false;
        let delta = { x: 0, y: 0 };
        let preparationError: string | undefined;
        let prepared: ShellDragPreparation | null = null;
        let launchStarted = false;

        /** 结束监听、释放准备对象并结束原生输入会话。 */
        const cleanup = async (endInput = true) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishPointerDrag);
            handle.removeEventListener("pointercancel", finishPointerDrag);
            handle.removeEventListener("lostpointercapture", finishPointerDrag);
            window.removeEventListener("keydown", handleKeyDown);
            if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
            if (prepared) {
                void cancelDesktopItemDragPreparation(prepared.token).catch(() => undefined);
                prepared = null;
            }
            if (endInput) await onEndInputDrag().catch(() => undefined);
        };

        /** 只在已跨阈值且准备完整成功时启动 OLE。 */
        const launchShellDrag = async () => {
            if (finished || launchStarted || !prepared) return;
            launchStarted = true;
            const token = prepared.token;
            prepared = null;
            await cleanup(false);
            try {
                await onInteraction({ type: "focus-item", itemId: anchorItemId, selectionMode: preservesSelection ? "preserve" : pointerSelectionMode });
                await onStartShellDrag(token, pointerOffset);
            } catch (error: unknown) {
                onFailure(error instanceof Error ? error.message : "启动系统拖放失败");
            } finally {
                await cancelDesktopItemDragPreparation(token).catch(() => undefined);
                await onEndInputDrag().catch(() => undefined);
            }
        };

        /** 准备失败只在用户实际越过拖动阈值后提示。 */
        const rejectShellDrag = async () => {
            if (!preparationError || finished) return;
            onFailure(preparationError);
            await cleanup();
        };

        /** 统一处理指针结束、取消和丢失捕获。 */
        const finishPointerDrag = (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            if (finishEvent.type === "pointerup" && !moved) {
                void (async () => {
                    await cleanup();
                    await onInteraction({ type: "focus-item", itemId: anchorItemId, selectionMode: pointerSelectionMode }).catch(() => undefined);
                    if (shouldOpenItemOnPointerRelease(preferences?.doubleClickToOpen ?? true, false, "pointerup", openingModifiers)) await onOpenItems([anchorItemId]).catch((error: unknown) => onFailure(error instanceof Error ? error.message : "打开桌面项目失败"));
                })();
                return;
            }
            if (prepared && (Math.abs(delta.x) >= prepared.threshold.left || Math.abs(delta.y) >= prepared.threshold.top)) {
                void launchShellDrag();
                return;
            }
            void cleanup();
        };

        /** 指针移动只负责点击阈值判定，页面不再提交布局预览。 */
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            if (finished) return;
            delta = { x: moveEvent.clientX - event.clientX, y: moveEvent.clientY - event.clientY };
            moved ||= Math.abs(delta.x) > 3 || Math.abs(delta.y) > 3;
            if (!prepared && preparationError && moved) void rejectShellDrag();
            if (prepared && moved) void launchShellDrag();
        };

        /** Escape 与系统取消共用清理路径。 */
        const handleKeyDown = (keyEvent: globalThis.KeyboardEvent) => {
            if (keyEvent.key !== "Escape") return;
            keyEvent.preventDefault();
            void cleanup();
        };

        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishPointerDrag);
        handle.addEventListener("pointercancel", finishPointerDrag);
        handle.addEventListener("lostpointercapture", finishPointerDrag);
        window.addEventListener("keydown", handleKeyDown);
        void prepareDesktopItemDrag(dragItemIds)
            .then((result) => {
                if (finished) {
                    if (result) void cancelDesktopItemDragPreparation(result.token).catch(() => undefined);
                    return;
                }
                if (!result) {
                    preparationError = "无法准备完整桌面项目拖放";
                    if (moved) void rejectShellDrag();
                    return;
                }
                prepared = result;
                if (Math.abs(delta.x) >= result.threshold.left || Math.abs(delta.y) >= result.threshold.top) void launchShellDrag();
            })
            .catch((error: unknown) => {
                preparationError = error instanceof Error ? error.message : "无法准备完整桌面项目拖放";
                if (moved) void rejectShellDrag();
            });
    };

    return handleItemPointerDown;
};
