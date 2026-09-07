import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import {
    parseVirtualDesktopSnapshot,
    parseVirtualDesktopSwitchingEvent,
    type VirtualDesktopSnapshot,
    type VirtualDesktopSwitchingEvent,
} from "../contracts/virtualDesktop";
import { invokeRuntime, listenRuntime } from "./runtime";

// 虚拟桌面环境快照变化事件名称。
export const VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT = "virtual-desktop-snapshot-changed";

// 虚拟桌面切换开始事件名称。
export const VIRTUAL_DESKTOP_SWITCHING_EVENT = "virtual-desktop-switching";

/** 返回当前虚拟桌面环境和布局归档摘要。 */
export const getVirtualDesktopSnapshot = async (): Promise<VirtualDesktopSnapshot> => {
    const response = await invokeRuntime<unknown>("get_virtual_desktop_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的虚拟桌面 command 返回值。
    return parseVirtualDesktopSnapshot(response);
};

/** 创建新 Windows 虚拟桌面。 */
export const createVirtualDesktop = async (): Promise<VirtualDesktopSnapshot> => {
    const response = await invokeRuntime<unknown>("create_virtual_desktop").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 创建后的虚拟桌面环境。
    return parseVirtualDesktopSnapshot(response);
};

/** 删除目标 Windows 虚拟桌面并指定窗口回退桌面。 */
export const deleteVirtualDesktop = async (desktopId: string, fallbackId: string): Promise<VirtualDesktopSnapshot> => {
    const response = await invokeRuntime<unknown>("delete_virtual_desktop", { desktopId, fallbackId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 删除后的虚拟桌面环境。
    return parseVirtualDesktopSnapshot(response);
};

/** 创建新桌面并绑定一个删除布局归档。 */
export const restoreVirtualDesktopArchive = async (archiveId: string): Promise<VirtualDesktopSnapshot> => {
    const response = await invokeRuntime<unknown>("restore_virtual_desktop_archive", { archiveId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 归档绑定后的虚拟桌面环境。
    return parseVirtualDesktopSnapshot(response);
};

/** 订阅虚拟桌面环境快照。 */
export const listenVirtualDesktopSnapshots = (onSnapshot: (snapshot: VirtualDesktopSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseVirtualDesktopSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });

/** 订阅桌面切换开始事件以取消活动交互。 */
export const listenVirtualDesktopSwitching = (
    onSwitching: (event: VirtualDesktopSwitchingEvent) => void,
    onError: (error: unknown) => void,
): Promise<UnlistenFn> =>
    listenRuntime<unknown>(VIRTUAL_DESKTOP_SWITCHING_EVENT, (payload) => {
        try {
            onSwitching(parseVirtualDesktopSwitchingEvent(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
