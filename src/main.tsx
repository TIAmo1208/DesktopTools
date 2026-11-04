import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { PreferencesProvider } from "./features/settings/PreferencesProvider";
import { SettingsPage } from "./features/settings/SettingsPage";
import "./styles/global.css";
import "./styles/bootSplash.css";

// 编辑器及其样式只在按需编辑窗加载，桌面宿主不承担其初始化成本。
const CollectEditor = lazy(() => import("./features/tile-content/collect-box/CollectEditor").then((module) => ({ default: module.CollectEditor })));

// React 应用挂载节点。
const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("缺少 React 应用挂载节点");
// 正式透明桌面宿主页面标记。
const query = new URLSearchParams(window.location.search);
const isDesktopHost = query.get("desktop-host") === "1";
const isCollectEditor = query.get("collect-editor") === "1";
document.documentElement.dataset.desktopHost = String(isDesktopHost);
document.documentElement.dataset.collectEditor = String(isCollectEditor);
document.documentElement.dataset.theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
document.documentElement.dataset.desktopTheme = document.documentElement.dataset.theme;

/**
 * 首帧主题：宿主建窗口时已算好样式表地址，先挂上再渲染，别让用户先看见默认桌面再换皮。
 * 地址与偏好侧装配同一份格式，Provider 之后接管时判定为同一张表，不会重复加载。
 */
const mountFirstFrameTheme = () => {
    const href = query.get("theme-css");
    if (!href) return;
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.setAttribute("href", href);
    link.setAttribute("data-theme-plugin", "");
    document.head.append(link);
};

/**
 * 启动覆盖层：只在桌面宿主首屏、主显示器、本进程启动后两秒内且未要求减少动效时播放。
 * 多显示器各建一个窗口，只有 monitor=0 播；显示器拓扑变化重开窗口时 boot 已过期，不再播。
 */
const playBootSplash = () => {
    if (!isDesktopHost || query.get("monitor") !== "0") return;
    const boot = Number(query.get("boot") ?? "");
    if (!Number.isFinite(boot) || boot <= 0 || Date.now() - boot > 2000) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const splash = document.createElement("div");
    splash.id = "boot-splash";
    splash.setAttribute("aria-hidden", "true");
    const mark = document.createElement("span");
    mark.className = "boot-splash__mark";
    mark.textContent = "DESKTOP TOOLS";
    splash.append(mark);
    document.body.append(splash);
    const done = () => splash.remove();
    splash.addEventListener("animationend", (event) => {
        if (event.animationName === "boot-splash-fade") done();
    });
    window.setTimeout(done, 1600); // 动画被系统关掉时的兜底，避免覆盖层留在屏上。
};

mountFirstFrameTheme();
playBootSplash();

createRoot(rootElement).render(
    <StrictMode>
        <PreferencesProvider>
            {isDesktopHost ? (
                <App />
            ) : isCollectEditor ? (
                <Suspense fallback={<p>正在加载编辑器…</p>}>
                    <CollectEditor />
                </Suspense>
            ) : (
                <SettingsPage />
            )}
        </PreferencesProvider>
    </StrictMode>,
);
