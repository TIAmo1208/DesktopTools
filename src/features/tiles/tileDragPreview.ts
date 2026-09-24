/** 跨屏格子的只读视觉快照，不传输 HTML、事件或业务身份。 */
export interface TileDragVisual {
    tag: string; // 仅允许固定的呈现标签。
    text?: string; // 文本节点原文。
    style?: Record<string, string>; // 固定白名单内的计算样式。
    attributes?: Record<string, string>; // 图像及 SVG 的呈现属性。
    children?: TileDragVisual[]; // 当前已渲染的内容。
    scrollTop?: number; // 当前内容视口的滚动位置。
    scrollLeft?: number; // 当前横向滚动位置。
}

const tags = new Set([
    "div",
    "span",
    "p",
    "time",
    "section",
    "header",
    "aside",
    "ul",
    "li",
    "button",
    "img",
    "svg",
    "path",
    "circle",
    "rect",
    "line",
    "polyline",
    "polygon",
    "g",
]); // 预览不会创建脚本、表单、嵌入页面或外链。
const styles = [
    "left",
    "top",
    "right",
    "bottom",
    "visibility",
    "z-index",
    "transform",
    "-webkit-line-clamp",
    "-webkit-box-orient",
    "display",
    "position",
    "box-sizing",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "padding",
    "margin",
    "gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column",
    "grid-row",
    "flex-direction",
    "align-items",
    "align-self",
    "align-content",
    "justify-self",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "justify-content",
    "justify-items",
    "overflow",
    "white-space",
    "text-overflow",
    "color",
    "background-color",
    "border",
    "border-radius",
    "font-family",
    "font-size",
    "font-weight",
    "font-variant-numeric",
    "letter-spacing",
    "backdrop-filter",
    "line-height",
    "text-align",
    "text-shadow",
    "opacity",
    "object-fit",
    "fill",
    "stroke",
    "stroke-width",
]; // 只复制格子现有外观，不复制 URL 样式或业务选择器。
const attributes = [
    "src",
    "viewBox",
    "d",
    "cx",
    "cy",
    "r",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "width",
    "height",
    "points",
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
]; // SVG 与系统图像的静态内容。

/** 在源格子仍可见时捕获整个表面及内容，取消或跨屏无需重建业务实例。 */
export const captureTileDragVisual = (element: HTMLElement): TileDragVisual => {
    /** 递归读取已呈现节点，未知包装只保留其内容。 */
    const capture = (node: Node): TileDragVisual => {
        if (node.nodeType === Node.TEXT_NODE) return { tag: "text", text: node.textContent ?? "" };
        if (!(node instanceof Element)) return { tag: "text", text: "" };
        const computed = getComputedStyle(node); // 源屏当前实际样式。
        return {
            tag: tags.has(node.localName) ? node.localName : "div",
            style: Object.fromEntries(styles.map((property) => [property, computed.getPropertyValue(property)])),
            attributes: Object.fromEntries(attributes.flatMap((name) => (node.hasAttribute(name) ? [[name, node.getAttribute(name)!]] : []))),
            children: [...node.childNodes].map(capture),
            scrollTop: node.scrollTop || undefined,
            scrollLeft: node.scrollLeft || undefined,
        };
    };
    return capture(element);
};

/** 根据有界视觉数据创建惰性副本，接收端不解析或执行 HTML。 */
export const createTileDragPreview = (visual: TileDragVisual): HTMLElement => {
    let remaining = 512; // 单格子的节点上限，避免异常数据无限递归。
    const scrolls: { node: Element; top: number; left: number }[] = []; // 节点挂载后恢复视口，不能在无布局时写入滚动。
    /** 白名单同时约束标签、属性和样式，源窗口数据不能变成可执行内容。 */
    const create = (value: TileDragVisual, depth = 0): Node => {
        if (!value || --remaining < 0 || depth > 32) throw new Error("跨屏格子预览内容超出限制");
        if (value.tag === "text") return document.createTextNode(typeof value.text === "string" ? value.text : "");
        if (!tags.has(value.tag)) throw new Error("跨屏格子预览标签无效");
        const node = ["svg", "path", "circle", "rect", "line", "polyline", "polygon", "g"].includes(value.tag)
            ? document.createElementNS("http://www.w3.org/2000/svg", value.tag)
            : document.createElement(value.tag); // SVG 保留原图标几何。
        for (const [property, content] of Object.entries(value.style ?? {})) {
            if (styles.includes(property) && typeof content === "string" && !/url\s*\(/i.test(content)) node.style.setProperty(property, property === "overflow" && /auto|scroll/i.test(content) ? "hidden" : content);
        }
        for (const [name, content] of Object.entries(value.attributes ?? {})) {
            if (!attributes.includes(name) || typeof content !== "string") continue;
            if (name === "src" && (value.tag !== "img" || !/^data:image\/(png|jpeg|webp|gif);base64,/i.test(content))) continue;
            if ((name === "fill" || name === "stroke") && /url\s*\(/i.test(content)) continue;
            node.setAttribute(name, content);
        }
        for (const child of value.children ?? []) node.appendChild(create(child, depth + 1));
        if (Number.isFinite(value.scrollTop) || Number.isFinite(value.scrollLeft))
            scrolls.push({ node, top: value.scrollTop ?? 0, left: value.scrollLeft ?? 0 });
        return node;
    };
    const root = create(visual); // 单一惰性表面，不包含 data-hit 或项目映射身份。
    if (!(root instanceof HTMLElement)) throw new Error("跨屏格子预览根节点无效");
    root.inert = true;
    root.setAttribute("aria-hidden", "true");
    queueMicrotask(() =>
        scrolls.forEach(({ node, top, left }) => {
            if (node.isConnected) {
                node.scrollTop = top;
                node.scrollLeft = left;
            }
        }),
    );
    return root;
};
