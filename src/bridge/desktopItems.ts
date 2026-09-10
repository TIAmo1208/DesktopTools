import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import { parseDesktopItemSnapshot, parseShellOperationResult, type DesktopItemSnapshot, type ShellOperationResult } from "../contracts/desktopItem";
import { invokeRuntime, listenRuntime } from "./runtime";
import { parseShellDragPreparation, type ShellDragPreparation } from "../contracts/desktopItem";
import type { WorkspacePoint } from "../contracts/workspace";

// Rust 发布的桌面项目快照事件名称。
export const DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT = "desktop-item-snapshot-changed";

/** 枚举 Shell 桌面项目并获取新的不可变快照。 */
export const getDesktopItemSnapshot = async (): Promise<DesktopItemSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("get_desktop_item_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });

    return parseDesktopItemSnapshot(response);
};

/** 使用 Shell 默认动词打开一个或多个桌面项目。 */
export const openDesktopItems = async (itemIds: string[]): Promise<void> => {
    await invokeRuntime<unknown>("open_desktop_items", { itemIds }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 使用 IFileOperation 重命名单个桌面项目。 */
export const renameDesktopItem = async (itemId: string, newName: string): Promise<ShellOperationResult> => {
    const response = await invokeRuntime<unknown>("rename_desktop_item", { itemId, newName }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的 Shell 重命名结果。
    return parseShellOperationResult(response);
};

/** 将完整选择集合移入回收站或永久删除。 */
export const deleteDesktopItems = async (itemIds: string[], permanent: boolean): Promise<ShellOperationResult> => {
    const response = await invokeRuntime<unknown>("delete_desktop_items", { itemIds, permanent }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的 Shell 删除结果。
    return parseShellOperationResult(response);
};

/** 将完整选择集合复制或剪切到系统 Shell 剪贴板。 */
export const setDesktopItemClipboard = async (itemIds: string[], cut: boolean): Promise<ShellOperationResult> => {
    const response = await invokeRuntime<unknown>("set_desktop_item_clipboard", { itemIds, cut }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的系统剪贴板结果。
    return parseShellOperationResult(response);
};

/** 将系统 Shell 剪贴板内容粘贴到桌面命名空间。 */
export const pasteDesktopItems = async (): Promise<ShellOperationResult> => {
    const response = await invokeRuntime<unknown>("paste_desktop_items").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的 Shell 粘贴结果。
    return parseShellOperationResult(response);
};

/** 使用完整选择集合启动系统 OLE 拖放。 */
export const startDesktopItemDrag = async (token: number, pointerOffset: WorkspacePoint): Promise<ShellOperationResult> => {
    const response = await invokeRuntime<unknown>("start_desktop_item_drag", { token, pointerOffset }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的系统拖放结果。
    return parseShellOperationResult(response);
};

/** 原指针拖动期间准备完整 Shell 数据，不能准备时保持整组内部操作。 */
export const prepareDesktopItemDrag = async (itemIds: string[]): Promise<ShellDragPreparation | null> =>
    parseShellDragPreparation(await invokeRuntime<unknown>("prepare_desktop_item_drag", { itemIds }));

/** 释放单击、取消或过期指针会话对应的原生准备。 */
export const cancelDesktopItemDragPreparation = (token: number): Promise<void> => invokeRuntime<void>("cancel_desktop_item_drag_preparation", { token });

/** 订阅 Rust 发布的桌面项目快照并校验 event 载荷。 */
export const listenDesktopItemSnapshots = (onSnapshot: (snapshot: DesktopItemSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseDesktopItemSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
