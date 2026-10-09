import type Vditor from "vditor";

/** Vditor 放大只读取 src；只读视图提供已解码地址，正文节点保持原始引用。 */
export const previewCollectImage = (image: HTMLImageElement, preview: typeof Vditor.previewImage, theme: string) => {
    if (!image.complete || !image.naturalWidth || !image.currentSrc) return;
    const source = image.currentSrc.replace(/&/g, "&amp;").replace(/"/g, "&quot;"); // Vditor 将地址插入 HTML 属性，避免地址字符改变标记。
    const displayed = new Proxy(image, {
        get(target, key) {
            if (key === "getAttribute") return (name: string) => (name === "src" ? source : target.getAttribute(name));
            const value = Reflect.get(target, key, target); // DOM 原生访问器和方法必须使用真实节点作为接收者。
            return typeof value === "function" ? value.bind(target) : value;
        },
    }); // 保留正文图像的尺寸和位置，不创建额外图片或修改 src。
    preview(displayed, "zh_CN", theme);
};

/** 修正 Lute 图片 alt 中的转义节点，防止属性破损及编辑后图片变成正文。 */
export const configureCollectImageMarkdown = (lute: Vditor["vditor"]["lute"]) => {
    const renderBackslash: ILuteRenderCallback = (node, entering) => {
        let parent = node.__internal_object__.Parent as { Type: number; LinkType: number; Parent: typeof parent }; // Lute AST 的父链，0 为文档、34 为图片。
        while (parent.Type !== 0) {
            if (parent.Type === 34) return ["", Lute.WalkContinue]; // alt 只输出转义后的文本，不嵌入编辑标记。
            parent = parent.Parent;
        }
        return [entering ? '<span data-type="backslash"><span>\\</span>' : "</span>", Lute.WalkContinue];
    }; // 普通正文保留原有反斜杠编辑标记。
    lute.SetJSRenderers({ renderers: { Md2VditorDOM: { renderBackslash }, HTML2VditorDOM: { renderBackslash } } });

    const parseImages = (html: string) => {
        const document = new DOMParser().parseFromString(html, "text/html"); // 仅转换副本，不修改编辑器 DOM 或资源地址。
        for (const image of document.querySelectorAll("img[src]")) {
            image.setAttribute("src", (image.getAttribute("src") ?? "").replace(/[\s()[\]<>"\\]/g, encodeURIComponent));
        }
        return document;
    }; // Vditor 会解码 src；序列化前重新编码会破坏 Markdown 目标语法的字符，不重复编码已有百分号。
    const escapeImageLabels = (html: string) => {
        if (!html.includes("<img")) return html;
        const document = parseImages(html); // 图片目标和说明都在转换副本上规范化。
        for (const image of document.querySelectorAll("img[alt]")) {
            image.setAttribute("alt", (image.getAttribute("alt") ?? "").replace(/[\\[\]*_`"&<>]/g, "\\$&"));
        }
        return document.body.innerHTML;
    }; // Lute 默认不转义 alt；非成对方括号也必须能保存并重新加载。
    const serialize = lute.VditorDOM2Md.bind(lute); // 保存使用转义后的说明和合法目标地址。
    lute.VditorDOM2Md = (html: string) => serialize(escapeImageLabels(html));
    for (const method of ["SpinVditorDOM", "HTML2VditorDOM"] as const) {
        const original = lute[method].bind(lute); // 这些转换不完整沿用 Md2VditorDOM 自定义渲染器。
        lute[method] = (html: string) => {
            if (!html.includes("<img")) return original(html);
            const document = parseImages(html); // 重排和粘贴前隔离图片说明，避免 Lute 再次解释为 Markdown。
            const labels = new Map<string, string>(); // 仅在本次同步转换期间保存原始说明。
            for (const image of document.querySelectorAll("img[alt]")) {
                const marker = `collect-image-label-${labels.size}`; // 不含 Markdown 标记的临时说明。
                labels.set(marker, image.getAttribute("alt") ?? "");
                image.setAttribute("alt", marker);
            }
            const rendered = new DOMParser().parseFromString(original(document.body.innerHTML), "text/html"); // 其余正文与光标仍由 Lute 原路径处理。
            for (const image of rendered.querySelectorAll("img[alt]")) {
                const label = labels.get(image.getAttribute("alt") ?? ""); // 按临时身份恢复，不能按 DOM 顺序猜测。
                if (label !== undefined) image.setAttribute("alt", label);
            }
            return rendered.body.innerHTML;
        };
    }
};

/** 仅替换图片显示候选；Vditor 序列化继续读取原始 src，保留相对路径。 */
export const observeCollectImages = (
    host: HTMLElement,
    name: string,
    readImage: (name: string, reference: string) => Promise<string>,
    onError: (error: unknown) => void,
) => {
    let active = true; // 卸载或改名后丢弃迟到结果。
    const requests = new WeakMap<HTMLImageElement, string>(); // 同一节点的同一引用只读取一次。
    const update = () => {
        for (const image of host.querySelectorAll<HTMLImageElement>(".vditor-wysiwyg img")) {
            const source = image.getAttribute("src") ?? ""; // 不使用浏览器自动补全的页面地址。
            if (requests.get(image) === source) continue;
            requests.set(image, source);
            image.removeAttribute("srcset");
            if (!source || /^(https?:|data:|blob:)/i.test(source)) continue;
            let reference = source; // Markdown URL 编码只在文件读取边界解码。
            try {
                if (!/^file:/i.test(source)) reference = decodeURIComponent(source);
            } catch {
                /* 原始百分号文件名按原文读取。 */
            }
            void readImage(name, reference)
                .then((url) => {
                    if (active && host.contains(image) && requests.get(image) === source) image.setAttribute("srcset", url);
                })
                .catch((error: unknown) => {
                    if (active && host.contains(image) && requests.get(image) === source) onError(error);
                });
        }
    }; // 编辑、粘贴与撤销重建节点时被动加载，无轮询。
    const observer = new MutationObserver(update); // 不观察 srcset，避免自身显示更新形成循环。
    observer.observe(host, { subtree: true, childList: true, attributes: true, attributeFilter: ["src"] });
    update();
    return () => {
        active = false;
        observer.disconnect();
    };
};
