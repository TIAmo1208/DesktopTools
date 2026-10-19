import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import { parseDesktopHostSnapshot, type DesktopHostSnapshot, type FrontendHostReadiness, type HitRegion } from "../contracts/desktopHost";
import { invokeRuntime, listenRuntime } from "./runtime";

// 正式宿主快照变化事件名称。
export const DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT = "desktop-host-snapshot-changed";

/** 返回正式宿主当前权威快照。 */
export const getDesktopHostSnapshot = async (): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的正式宿主 command 返回值。
    const response = await invokeRuntime<unknown>("get_desktop_host_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 从右上角测试面板请求应用正常退出。 */
export const exitDesktopTools = async (): Promise<void> => {
    await invokeRuntime("exit_desktop_tools").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 请求显示设置窗口，桌面格子的“打开设置”入口复用托盘同一显隐语义。 */
export const openSettingsWindow = async (): Promise<void> => {
    await invokeRuntime("open_settings_window").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 上报逐屏 Renderer 存活心跳。 */
export const recordDesktopHostRendererHeartbeat = async (monitorId: string): Promise<void> => {
    await invokeRuntime("record_desktop_host_renderer_heartbeat", { monitorId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 发布页面、系统图像和完整命中图准备证据。 */
export const publishDesktopHostReadiness = async (report: FrontendHostReadiness): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的前端准备 command 返回值。
    const response = await invokeRuntime<unknown>("publish_desktop_host_readiness", { report }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 原子替换页面布局变化后的完整语义命中图。 */
export const updateDesktopHostHitMap = async (monitorId: string, hitRegions: HitRegion[]): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的命中图 command 返回值。
    const response = await invokeRuntime<unknown>("update_desktop_host_hit_map", { monitorId, hitRegions }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 暂停正式宿主命中裁剪以承接连续指针拖动。 */
export const beginDesktopHostInputDrag = async (monitorId: string): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的拖动开始 command 返回值。
    const response = await invokeRuntime<unknown>("begin_desktop_host_input_drag", { monitorId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 结束连续指针拖动并恢复完整命中图。 */
export const endDesktopHostInputDrag = async (monitorId: string): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的拖动结束 command 返回值。
    const response = await invokeRuntime<unknown>("end_desktop_host_input_drag", { monitorId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 通过桌面项目点击处理器回传最小输入探针。 */
export const recordDesktopHostInputProbe = async (monitorId: string, itemId: string): Promise<DesktopHostSnapshot> => {
    // 等待契约校验的输入探针 command 返回值。
    const response = await invokeRuntime<unknown>("record_desktop_host_input_probe", { monitorId, itemId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseDesktopHostSnapshot(response);
};

/** 订阅正式宿主快照并校验事件载荷。 */
export const listenDesktopHostSnapshots = (onSnapshot: (snapshot: DesktopHostSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseDesktopHostSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
