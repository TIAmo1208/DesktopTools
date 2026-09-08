import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

import { getPreferences, getWindowsAccentColor, listenPreferences, listenWindowsAccentColor, settingsErrorMessage } from "../../bridge/settings";
import type { ApplicationPreferences } from "../../contracts/settings";

/** 当前窗口内共享的偏好加载状态。 */
interface PreferencesContextValue {
    preferences?: ApplicationPreferences; // 仅在真实后端加载成功后提供的偏好。
    failure?: string; // 加载或订阅错误说明。
    onPreferences: (preferences: ApplicationPreferences) => void; // 接收保存成功的权威偏好。
}

// 每个独立窗口只订阅一次偏好状态。
const PreferencesContext = createContext<PreferencesContextValue>({ onPreferences: () => undefined });

/** 读取设置窗口或逐屏宿主共用的偏好。 */
export const usePreferences = () => useContext(PreferencesContext);

/** 同步后端偏好和系统主题，不改变透明宿主背景。 */
export const PreferencesProvider = ({ children }: { children: ReactNode }) => {
    const [preferences, setPreferences] = useState<ApplicationPreferences>(); // 当前窗口收到的权威偏好。
    const [failure, setFailure] = useState<string>(); // 当前偏好加载错误。
    const [windowsAccentColor, setWindowsAccentColor] = useState<string>(); // 系统颜色独立于持久化的自定义颜色。
    const [accentFailure, setAccentFailure] = useState<string>(); // 系统强调色读取或订阅错误。
    const revision = useRef(0); // 当前窗口已接收偏好的顺序，防止启动读取回退已保存值。
    /** 接收命令保存或重试读取成功的偏好。 */
    const handlePreferences = useCallback((next: ApplicationPreferences) => {
        revision.current++;
        setPreferences(next);
        setFailure(undefined);
    }, []);
    useEffect(() => {
        let active = true; // 当前订阅生命周期是否仍有效。
        const initialRevision = revision.current; // 当前启动读取开始前的偏好序号。
        /** 接收当前订阅的已保存偏好。 */
        const handlePreferencesEvent = (next: ApplicationPreferences) => {
            if (!active) return;
            handlePreferences(next);
        };
        /** 在设置页呈现偏好读取错误，桌面保留原有双击行为。 */
        const handleFailure = (error: unknown) => {
            if (active) setFailure(settingsErrorMessage(error));
        };
        const subscription = listenPreferences(handlePreferencesEvent, handleFailure); // Tauri 或原生宿主的单个偏好订阅。
        void subscription
            .then(() => getPreferences())
            .then((next) => {
                if (active && initialRevision === revision.current) handlePreferences(next);
            })
            .catch(handleFailure);
        return () => {
            active = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [handlePreferences]);

    useEffect(() => {
        if (!preferences?.useWindowsAccentColor) return;
        let active = true; // 当前系统颜色订阅是否有效。
        let received = false; // 防止启动读取覆盖较新的系统事件。
        /** 接收系统颜色，不写入自定义颜色配置。 */
        const handleColor = (color: string) => {
            if (!active) return;
            received = true;
            setWindowsAccentColor(color);
            setAccentFailure(undefined);
        };
        /** 呈现系统颜色读取失败，保留最近有效颜色。 */
        const handleError = (error: unknown) => {
            if (active) setAccentFailure(settingsErrorMessage(error));
        };
        const subscription = listenWindowsAccentColor(handleColor, handleError); // 当前窗口独立的系统颜色订阅。
        void subscription
            .then(() => getWindowsAccentColor())
            .then((color) => {
                if (!received) handleColor(color);
            })
            .catch(handleError);
        return () => {
            active = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [preferences?.useWindowsAccentColor]);

    useEffect(() => {
        const systemTheme = window.matchMedia("(prefers-color-scheme: dark)"); // 当前 Windows 系统主题媒体查询。
        /** 将用户选择解析为实际浅色或深色，供各组件复用。 */
        const handleTheme = () => {
            const theme = preferences?.theme ?? "system"; // 加载期间沿用系统主题。
            document.documentElement.dataset.theme = theme === "system" ? (systemTheme.matches ? "dark" : "light") : theme;
        };
        handleTheme();
        systemTheme.addEventListener("change", handleTheme);
        return () => systemTheme.removeEventListener("change", handleTheme);
    }, [preferences?.theme]);

    useEffect(() => {
        if (!preferences) return;
        const root = document.documentElement; // 当前独立窗口的主题根节点。
        const accent = preferences.useWindowsAccentColor ? (windowsAccentColor ?? preferences.accentColor) : preferences.accentColor; // 系统读取失败时保留有效自定义颜色。
        root.style.setProperty("--color-focus", accent);
        root.style.setProperty("--color-accent", accent);
        root.style.setProperty("--surface-radius", `${preferences.cornerRadius}px`);
        root.style.setProperty("--interface-font", `"${preferences.fontFamily}", "Segoe UI", sans-serif`);
        root.style.setProperty("--interface-font-size", `${preferences.fontSize}px`);
        for (const [scope, grid] of [
            ["desktop", preferences.desktopGrid],
            ["fence", preferences.fenceGrid],
        ] as const) {
            root.style.setProperty(`--${scope}-grid-width`, `${grid.width}px`);
            root.style.setProperty(`--${scope}-grid-height`, `${grid.height}px`);
            root.style.setProperty(`--${scope}-grid-column-gap`, `${grid.columnGap}px`);
            root.style.setProperty(`--${scope}-grid-row-gap`, `${grid.rowGap}px`);
        }
    }, [preferences, windowsAccentColor]);

    return (
        <PreferencesContext.Provider
            value={{ preferences, failure: failure ?? (preferences?.useWindowsAccentColor ? accentFailure : undefined), onPreferences: handlePreferences }}
        >
            {children}
        </PreferencesContext.Provider>
    );
};
