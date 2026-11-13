import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { PreferencesProvider } from "./features/settings/PreferencesProvider";
import { SettingsPage } from "./features/settings/SettingsPage";
import "./styles/global.css";
import "./styles/bootSplash.css";

// 编辑器及其样式只在按需编辑窗加载，桌面宿主不承担其初始化成本。
const CollectEditor = lazy(() => import("./features/tile-content/collect-box/CollectEditor").then((module) => ({ default: module.CollectEditor })));
// 置顶弹层只在独立弹层窗加载，桌面宿主与设置窗都不装配它。
const PopupLayerPage = lazy(() => import("./features/popup-layer/PopupLayerPage").then((module) => ({ default: module.PopupLayerPage })));

// React 应用挂载节点。
const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("缺少 React 应用挂载节点");
// 正式透明桌面宿主页面标记。
const query = new URLSearchParams(window.location.search);
const isDesktopHost = query.get("desktop-host") === "1";
const isIsland = query.get("island") === "1"; // 独立岛不装配设置页或桌面工作区。
const isCollectEditor = query.get("collect-editor") === "1";
const isPopupLayer = query.get("popup-layer") === "1"; // 置顶弹层窗口：菜单、确认与天气弹窗。
document.documentElement.dataset.desktopHost = String(isDesktopHost);
document.documentElement.dataset.island = String(isIsland);
document.documentElement.dataset.collectEditor = String(isCollectEditor);
document.documentElement.dataset.popupLayer = String(isPopupLayer);
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

/** 启动覆盖层的内置时长：主题没有声明 boot.durationMs 时用它。 */
const DEFAULT_BOOT_MS = 900;
/** 主题能声明的时长上限，与宿主校验保持同一个数。 */
const MAX_BOOT_MS = 10_000;

/**
 * 启动覆盖层：只在桌面宿主首屏、主显示器、本进程启动后两秒内且未要求减少动效时播放。
 * 多显示器各建一个窗口，只有 monitor=0 播；显示器拓扑变化重开窗口时 boot 已过期，不再播。
 * 时长由宿主按主题的 boot.durationMs 拼进地址，主题写 0 表示不播。
 */
const playBootSplash = () => {
    if (!isDesktopHost || query.get("monitor") !== "0") return;
    const boot = Number(query.get("boot") ?? "");
    if (!Number.isFinite(boot) || boot <= 0 || Date.now() - boot > 2000) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const declared = query.get("theme-boot-ms");
    const parsed = declared === null ? Number.NaN : Number(declared);
    const duration = Number.isFinite(parsed) ? Math.min(Math.max(parsed, 0), MAX_BOOT_MS) : DEFAULT_BOOT_MS;
    if (duration <= 0) return; // 主题显式关掉了启动覆盖层。
    const splash = document.createElement("div");
    splash.id = "boot-splash";
    splash.setAttribute("aria-hidden", "true");
    splash.style.setProperty("--boot-splash-duration", `${duration}ms`); // CSSOM 赋值不受内联样式策略限制。
    const mark = document.createElement("span");
    mark.className = "boot-splash__mark";
    mark.textContent = "DESKTOP TOOLS";
    splash.append(mark);
    document.body.append(splash);
    const done = () => splash.remove();
    splash.addEventListener("animationend", (event) => {
        if (event.animationName === "boot-splash-fade") done();
    });
    window.setTimeout(done, duration + 200); // 动画被系统关掉时的兜底，避免覆盖层留在屏上。
};

mountFirstFrameTheme();
playBootSplash();

createRoot(rootElement).render(
    <StrictMode>
        <PreferencesProvider>
            {isPopupLayer ? (
                <Suspense fallback={null}>
                    <PopupLayerPage />
                </Suspense>
            ) : isDesktopHost || isIsland ? (
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
