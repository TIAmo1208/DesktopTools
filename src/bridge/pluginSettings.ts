import type { UnlistenFn } from "@tauri-apps/api/event";

import { invokeRuntime, listenRuntime } from "./runtime";

/** 与 Rust `PLUGIN_SETTINGS_CHANGED_EVENT` 相同，载荷只有 pluginId。 */
export const PLUGIN_SETTINGS_CHANGED_EVENT = "plugin-settings-changed";

interface PluginSettingsListener {
    onSettings: (pluginId: string) => void; // 某个插件的设置已变化，调用方再读命令。
    onError: (error: unknown) => void; // 载荷无法识别。
}

const desktopListeners = new Set<PluginSettingsListener>(); // 逐屏页面上的设置订阅。
let desktopDispatchWrapped = false; // 初始化脚本的 dispatch 只包一层。

/** 从事件载荷取出插件标识。 */
const readPluginId = (payload: unknown): string | undefined => {
    if (typeof payload !== "object" || payload === null || !("pluginId" in payload)) return undefined;
    const pluginId = (payload as { pluginId: unknown }).pluginId; // 等待类型确认的标识。
    return typeof pluginId === "string" && pluginId.length > 0 ? pluginId : undefined;
};

/** 逐屏 listen 只登记带快照命令或固定名单的事件，此事件只能挂在已转发的 dispatch 上。 */
const ensureDesktopSettingsDispatch = () => {
    const native = globalThis.window?.__DESKTOP_TOOLS_NATIVE__; // 正式逐屏宿主桥。
    if (!native?.dispatch || desktopDispatchWrapped) return;
    const dispatch = native.dispatch; // 初始化脚本原来的投递。
    native.dispatch = (eventName, payload) => {
        dispatch(eventName, payload);
        if (eventName !== PLUGIN_SETTINGS_CHANGED_EVENT) return;
        const pluginId = readPluginId(payload); // 只带标识，设置正文要重新读取。
        for (const listener of desktopListeners) {
            try {
                if (!pluginId) listener.onError(new Error("插件设置事件无效"));
                else listener.onSettings(pluginId);
            } catch (error) {
                console.error("插件设置事件处理失败", error);
            }
        }
    };
    desktopDispatchWrapped = true;
};

/** 读取一份插件设置；没有文件时为 null。 */
export const getPluginSettings = (pluginId: string) => invokeRuntime<unknown>("get_plugin_settings", { pluginId });

/** 保存一份插件设置；成功后由事件通知各屏幕重读。 */
export const savePluginSettings = (pluginId: string, value: unknown) => invokeRuntime<void>("save_plugin_settings", { pluginId, value });

/** 订阅插件设置变更。逐屏走 dispatch，设置窗口走普通事件。调用方必须执行返回的退订。 */
export const listenPluginSettings = (onSettings: (pluginId: string) => void, onError: (error: unknown) => void): Promise<UnlistenFn> => {
    if (globalThis.window?.__DESKTOP_TOOLS_NATIVE__?.dispatch) {
        ensureDesktopSettingsDispatch();
        const listener = { onSettings, onError }; // 这一份订阅。
        desktopListeners.add(listener);
        return Promise.resolve(() => {
            desktopListeners.delete(listener);
        });
    }
    return listenRuntime<unknown>(PLUGIN_SETTINGS_CHANGED_EVENT, (payload) => {
        const pluginId = readPluginId(payload); // 设置窗口收到的同一载荷。
        if (!pluginId) onError(new Error("插件设置事件无效"));
        else onSettings(pluginId);
    });
};
