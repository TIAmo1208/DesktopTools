import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import "./styles/global.css";

// React 应用挂载节点。
const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("缺少 React 应用挂载节点");
// 正式透明桌面宿主页面标记。
const isDesktopHost = new URLSearchParams(window.location.search).get("desktop-host") === "1";
document.documentElement.dataset.desktopHost = String(isDesktopHost);

createRoot(rootElement).render(
    <StrictMode>
        <App />
    </StrictMode>,
);
