import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

/** 正式 Windowed WebView2 初始化脚本提供的最小运行时桥。 */
interface DesktopToolsNativeBridge {
    isRefreshPaused?: () => boolean; // 当前宿主是否主动暂停非必要刷新。
    invoke: (command: string, payload?: Record<string, unknown>) => Promise<unknown>;
    listen: (eventName: string, handler: (payload: unknown) => void) => Promise<UnlistenFn>;
    dispatch?: (eventName: string, payload: unknown) => void; // 逐屏白名单事件的页面入口。
}

declare global {
    /** 暴露正式 Windowed WebView2 注入的本地运行时桥。 */
    interface Window {
        __DESKTOP_TOOLS_NATIVE__?: DesktopToolsNativeBridge;
    }
}

/** 通过当前页面所属运行时调用 Rust 命令。 */
export const invokeRuntime = <T>(command: string, payload?: Record<string, unknown>): Promise<T> => {
    const nativeBridge = globalThis.window?.__DESKTOP_TOOLS_NATIVE__; // 正式 Windowed WebView2 的本地桥。
    if (nativeBridge) return nativeBridge.invoke(command, payload) as Promise<T>;
    return invokeWindow<T>(command, payload);
};

/** 通过当前页面所属运行时订阅 Rust 快照事件。 */
export const listenRuntime = <T>(eventName: string, handler: (payload: T) => void): Promise<UnlistenFn> => {
    const nativeBridge = globalThis.window?.__DESKTOP_TOOLS_NATIVE__; // 正式 Windowed WebView2 的本地桥。
    if (nativeBridge) return nativeBridge.listen(eventName, (payload) => handler(payload as T));
    return listenWindow<T>(eventName, handler);
};

/** 普通页面只关心同一权威快照中的锁屏状态。 */
interface SessionSnapshot {
    revision: number; // 拒绝迟到的旧读取。
    locked: boolean | null; // 未知不伪装成已解锁。
}

let sessionReady: Promise<void> | undefined; // 一个页面只注册一份锁屏监听。
let locked = false; // 普通前台策略不暂停设置页面。
let sessionRevision = -1; // 最近应用的修订。
const resumeWindow = new Set<() => void>(); // 仅保留最新状态读取入口，不积压事件。
const sessionAnimations = new Set<Animation>(); // 仅恢复本策略暂停的动画。

/** 先订阅再读取，锁屏事件优先于迟到的初始化快照。 */
const ensureSession = () => {
    sessionReady ??= (async () => {
        const apply = (snapshot: SessionSnapshot) => {
            if (!Number.isSafeInteger(snapshot?.revision) || snapshot.revision < sessionRevision) return;
            sessionRevision = snapshot.revision;
            if (snapshot.locked === null || locked === snapshot.locked) return;
            locked = snapshot.locked;
            document.documentElement.dataset.sessionLocked = String(locked);
            if (locked) {
                for (const animation of document.getAnimations()) {
                    if (animation.playState === "running") {
                        sessionAnimations.add(animation);
                        animation.pause();
                    }
                }
            } else {
                for (const animation of sessionAnimations) animation.play();
                sessionAnimations.clear();
                for (const resume of [...resumeWindow]) resume();
            }
        }; // 所有普通页面共享相同会话语义。
        const stop = await tauriListen<SessionSnapshot>("desktop-refresh-policy-changed", (event) => apply(event.payload)); // 先建立通知接收。
        window.addEventListener("pagehide", stop, { once: true });
        apply(await tauriInvoke<SessionSnapshot>("get_foreground_snapshot"));
    })();
    return sessionReady;
};

/** 锁屏期间停止页面读取，后端原有监听和命令通道不受影响。 */
const waitForUnlock = async () => {
    if (!locked) return;
    await new Promise<void>((resolve) => {
        const resume = () => {
            resumeWindow.delete(resume);
            resolve();
        }; // 一次性等待，不使用计时器。
        resumeWindow.add(resume);
    });
};

/** 普通页面读取不在锁屏时提交结果，恢复后重新读取当前事实。 */
const invokeWindow = async <T>(command: string, payload?: Record<string, unknown>): Promise<T> => {
    await ensureSession();
    const read = command.startsWith("get_"); // 只读命令可安全在解锁后重读，写命令不重放。
    await waitForUnlock();
    let result = await tauriInvoke<T>(command, payload); // 后端执行继续使用原接口。
    while (locked) {
        await waitForUnlock();
        if (read) result = await tauriInvoke<T>(command, payload);
    }
    return result;
};

/** 锁屏时停止视觉事件交付，解锁时每个订阅仅交付最终状态。 */
const listenWindow = async <T>(eventName: string, handler: (payload: T) => void): Promise<UnlistenFn> => {
    await ensureSession();
    let active = true; // 退订后拒绝迟到交付。
    let pending: T | undefined; // 最新完整事件，不保存历史序列。
    const resume = () => {
        if (active && pending !== undefined) {
            const latest = pending;
            pending = undefined;
            handler(latest);
        }
    }; // 当前非桌面页面订阅均为完整状态事件。
    resumeWindow.add(resume);
    const stop = await tauriListen<T>(eventName, (event) => {
        if (!active) return;
        if (locked) pending = event.payload;
        else handler(event.payload);
    });
    return () => {
        active = false;
        pending = undefined;
        resumeWindow.delete(resume);
        stop();
    };
};
