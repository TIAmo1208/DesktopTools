import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

/** 正式 Windowed WebView2 初始化脚本提供的最小运行时桥。 */
interface DesktopToolsNativeBridge {
    isRefreshPaused?: () => boolean; // 当前宿主是否主动暂停非必要刷新。
    invoke: (command: string, payload?: Record<string, unknown>) => Promise<unknown>;
    listen: (eventName: string, handler: (payload: unknown) => void) => Promise<UnlistenFn>;
}

declare global {
    /** 暴露正式 Windowed WebView2 注入的本地运行时桥。 */
    interface Window {
        __DESKTOP_TOOLS_NATIVE__?: DesktopToolsNativeBridge;
    }
}

/** 通过当前页面所属运行时调用 Rust 命令。 */
export const invokeRuntime = <T>(command: string, payload?: Record<string, unknown>): Promise<T> => {
    const nativeBridge = window.__DESKTOP_TOOLS_NATIVE__; // 正式 Windowed WebView2 的本地桥。
    if (nativeBridge) return nativeBridge.invoke(command, payload) as Promise<T>;
    return tauriInvoke<T>(command, payload);
};

/** 通过当前页面所属运行时订阅 Rust 快照事件。 */
export const listenRuntime = <T>(eventName: string, handler: (payload: T) => void): Promise<UnlistenFn> => {
    const nativeBridge = window.__DESKTOP_TOOLS_NATIVE__; // 正式 Windowed WebView2 的本地桥。
    if (nativeBridge) return nativeBridge.listen(eventName, (payload) => handler(payload as T));
    return tauriListen<T>(eventName, (event) => handler(event.payload));
};
