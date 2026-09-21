import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

import { getPreferences, getWindowsAccentColor, listenPreferences, listenWindowsAccentColor, settingsErrorMessage } from "../../bridge/settings";
import { resolveAnimationsEnabled, type ApplicationPreferences } from "../../contracts/settings";
import { invokeRuntime } from "../../bridge/runtime";

/** 当前窗口内共享的偏好加载状态。 */
interface PreferencesContextValue {
    preferences?: ApplicationPreferences; // 仅在真实后端加载成功后提供的偏好。
    failure?: string; // 加载或订阅错误说明。
    reducedMotion: boolean; // 当前系统是否要求减少动态效果。
    animationsEnabled: boolean; // 用户偏好与系统限制合并后的有效状态。
    onPreferences: (preferences: ApplicationPreferences) => void; // 接收保存成功的权威偏好。
}

// 每个独立窗口只订阅一次偏好状态。
const PreferencesContext = createContext<PreferencesContextValue>({ onPreferences: () => undefined, reducedMotion: false, animationsEnabled: false });

/** 读取设置窗口或逐屏宿主共用的偏好。 */
export const usePreferences = () => useContext(PreferencesContext);

/** 同步后端偏好和系统主题，不改变透明宿主背景。 */
export const PreferencesProvider = ({ children }: { children: ReactNode }) => {
    const [reducedMotion, setReducedMotion] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches); // 首次渲染即遵守系统偏好。
    const [preferences, setPreferences] = useState<ApplicationPreferences>(); // 当前窗口收到的权威偏好。
    const [failure, setFailure] = useState<string>(); // 当前偏好加载错误。
    const [windowsAccentColor, setWindowsAccentColor] = useState<string>(); // 系统颜色独立于持久化的自定义颜色。
    const [accentFailure, setAccentFailure] = useState<string>(); // 系统强调色读取或订阅错误。
    const revision = useRef(0); // 当前窗口已接收偏好的顺序，防止启动读取回退已保存值。
    const animationsEnabled = resolveAnimationsEnabled(preferences, reducedMotion); // 跟随系统与手动动画偏好只由一个入口解析。
    useEffect(() => {
        const query = window.matchMedia("(prefers-reduced-motion: reduce)"); // 持续监听系统设置。
        const handleMotion = () => setReducedMotion(query.matches); // 系统改变后立即更新有效状态。
        handleMotion();
        query.addEventListener("change", handleMotion);
        return () => query.removeEventListener("change", handleMotion);
    }, []);
    useEffect(() => {
        document.documentElement.dataset.interfaceAnimations = animationsEnabled ? "on" : "off";
    }, [animationsEnabled]);
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
        if (!preferences?.useWindowsAccentColor && !preferences?.desktopAppearance.useWindowsAccentColor) return;
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
    }, [preferences?.useWindowsAccentColor, preferences?.desktopAppearance.useWindowsAccentColor]);

    useEffect(() => {
        const systemTheme = window.matchMedia("(prefers-color-scheme: dark)"); // 当前 Windows 系统主题媒体查询。
        /** 将用户选择解析为实际浅色或深色，供各组件复用。 */
        const handleTheme = () => {
            const theme = preferences?.theme ?? "system"; // 加载期间沿用系统主题。
            document.documentElement.dataset.theme = theme === "system" ? (systemTheme.matches ? "dark" : "light") : theme;
            const desktopTheme = preferences?.desktopAppearance.theme ?? "system"; // 桌面独立解析系统深浅模式。
            document.documentElement.dataset.desktopTheme = desktopTheme === "system" ? (systemTheme.matches ? "dark" : "light") : desktopTheme;
            if (preferences)
                void invokeRuntime("sync_message_appearance", {
                    preferences,
                    dark: document.documentElement.dataset.theme === "dark",
                    animationsEnabled,
                }).catch((error: unknown) => setFailure(settingsErrorMessage(error))); // 原生消息只消费程序已经解析的界面外观，不另行监听系统颜色。
        };
        handleTheme();
        systemTheme.addEventListener("change", handleTheme);
        return () => systemTheme.removeEventListener("change", handleTheme);
    }, [preferences, animationsEnabled]);

    useEffect(() => {
        if (!preferences) return;
        const root = document.documentElement; // 当前独立窗口的主题根节点。
        const accent = preferences.useWindowsAccentColor ? (windowsAccentColor ?? preferences.accentColor) : preferences.accentColor; // 系统读取失败时保留有效自定义颜色。
        root.style.setProperty("--color-focus", accent);
        root.style.setProperty("--color-accent", accent);
        root.style.setProperty("--interface-accent", accent);
        const desktop = preferences.desktopAppearance; // 桌面主题保持独立，系统读取失败时保留该组自定义色。
        root.style.setProperty("--desktop-accent", desktop.useWindowsAccentColor ? (windowsAccentColor ?? desktop.accentColor) : desktop.accentColor);
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
            value={{
                preferences,
                reducedMotion,
                animationsEnabled,
                failure: failure ?? (preferences?.useWindowsAccentColor || preferences?.desktopAppearance.useWindowsAccentColor ? accentFailure : undefined),
                onPreferences: handlePreferences,
            }}
        >
            {children}
        </PreferencesContext.Provider>
    );
};
