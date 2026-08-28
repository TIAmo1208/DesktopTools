import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import { parseDesktopItemSnapshot, type DesktopItemSnapshot } from "../contracts/desktopItem";

// Rust 发布的桌面项目快照事件名称。
export const DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT = "desktop-item-snapshot-changed";

/** 枚举 Shell 桌面项目并获取新的不可变快照。 */
export const getDesktopItemSnapshot = async (): Promise<DesktopItemSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invoke<unknown>("get_desktop_item_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });

    return parseDesktopItemSnapshot(response);
};

/** 订阅 Rust 发布的桌面项目快照并校验 event 载荷。 */
export const listenDesktopItemSnapshots = (onSnapshot: (snapshot: DesktopItemSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listen<unknown>(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, (event) => {
        try {
            onSnapshot(parseDesktopItemSnapshot(event.payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
