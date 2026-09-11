import type { DesktopItemViewModel } from "./desktopItemViewModel";

/** 为本地拖动、跨屏拖动和重排副本提供同一项目内容与样式规则。 */
export const createDesktopItemPreview = (
    source: HTMLElement | DesktopItemViewModel,
    className = "desktop-item--unclassified",
    selected = false,
): HTMLElement => {
    const original = source instanceof HTMLElement ? source : undefined; // 本地呈现可读取项目自身的视觉状态。
    const item = original
        ? {
              key: original.dataset.itemId ?? "",
              label: original.querySelector(".desktop-item__label")?.textContent ?? "",
              imageSource: original.querySelector<HTMLImageElement>(".desktop-item__image img")?.src ?? null,
              isAvailable: original.dataset.availability !== "unavailable",
          }
        : (source as DesktopItemViewModel); // 跨屏仅使用权威数据，不接收 HTML。
    const node = document.createElement("li"); // 预览没有身份、命中或键盘焦点属性。
    node.className = original?.className ?? `desktop-item ${className}`;
    node.classList.add("desktop-item--drag-preview");
    node.dataset.availability = item.isAvailable ? "available" : "unavailable";
    if (original?.dataset.selected === "true" || selected) node.dataset.selected = "true";
    node.setAttribute("aria-hidden", "true");
    const container = document.createElement("div"); // 图像尺寸由项目 CSS 和列表变体决定。
    container.className = "desktop-item__image";
    if (item.imageSource) {
        const image = document.createElement("img"); // 预览不注册到桌面接管门槛。
        image.src = item.imageSource;
        image.alt = "";
        image.draggable = false;
        container.append(image);
    }
    const label = document.createElement("span"); // textContent 保留名称特殊字符。
    label.className = "desktop-item__label";
    label.textContent = item.label;
    node.append(container, label);
    if (!item.isAvailable) {
        const status = document.createElement("span"); // 与实际项目相同的失效说明。
        status.className = "desktop-item__status";
        status.textContent = "等待恢复";
        node.append(status);
    }
    if (original) {
        const appearance = getComputedStyle(original); // 脱离围栏后保留继承外观与当前网格尺寸规则。
        for (const property of [
            "--fence-text",
            "--fence-item-text-shadow",
            "color",
            "text-shadow",
            "font-size",
            "grid-template-rows",
            "grid-template-columns",
            "padding",
        ])
            node.style.setProperty(property, appearance.getPropertyValue(property));
    }
    Object.assign(node.style, { position: "fixed", pointerEvents: "none", zIndex: "20000" });
    return node;
};
