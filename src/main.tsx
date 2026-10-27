import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { PreferencesProvider } from "./features/settings/PreferencesProvider";
import { SettingsPage } from "./features/settings/SettingsPage";
import "./styles/global.css";

// 编辑器及其样式只在按需编辑窗加载，桌面宿主不承担其初始化成本。
const CollectEditor = lazy(() => import("./features/tile-content/collect-box/CollectEditor").then((module) => ({ default: module.CollectEditor })));

// React 应用挂载节点。
const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("缺少 React 应用挂载节点");
// 正式透明桌面宿主页面标记。
const isDesktopHost = new URLSearchParams(window.location.search).get("desktop-host") === "1";
const isCollectEditor = new URLSearchParams(window.location.search).get("collect-editor") === "1";
document.documentElement.dataset.desktopHost = String(isDesktopHost);
document.documentElement.dataset.collectEditor = String(isCollectEditor);
document.documentElement.dataset.theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
document.documentElement.dataset.desktopTheme = document.documentElement.dataset.theme;

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
