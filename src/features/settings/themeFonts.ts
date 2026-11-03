import type { ComponentManifest } from "../../contracts/component.ts";
import { APPLICATION_FONTS } from "../../contracts/settings.ts";

// 选中字体之后的系统回退顺序；缺字逐字符落到这些字体，不整段替换。
const INTERFACE_FONT_FALLBACK = ["Segoe UI Variable", "Segoe UI", "sans-serif"];

/** 拼出写入 --interface-font 的字体栈；具体字体一律加引号，通用族名不加。 */
export const interfaceFontStack = (family: string) =>
    [family, ...INTERFACE_FONT_FALLBACK].map((font) => (font === "sans-serif" ? font : `"${font}"`)).join(", ");

/** 外观页可选字体：系统名单在前，主题自带字体按名称排序，同名只保留一项。 */
export const themeFontOptions = (manifests: readonly ComponentManifest[] | undefined) => {
    const names = new Set<string>(APPLICATION_FONTS);
    const options: string[] = [...APPLICATION_FONTS];
    const pluginFonts = (manifests ?? []).flatMap((manifest) => manifest.themeFonts ?? []).sort((left, right) => left.family.localeCompare(right.family, "zh")); // 复制后再排序，不改快照。
    for (const font of pluginFonts) {
        if (names.has(font.family)) continue;
        names.add(font.family);
        options.push(font.family);
    }
    return options;
};

// 已注册的字体面板，键为族名、文件和版本，用于快照变化时增删。
const registeredFonts = new Map<string, FontFace>();

/** 把清单里的主题字体注册进当前文档；加载失败按回退栈显示，不做提示。 */
export const registerThemeFonts = (manifests: readonly ComponentManifest[] | undefined) => {
    const fonts = globalThis.document?.fonts; // 每份文档各自注册，桌面宿主与设置窗共用同一实现。
    if (!fonts) return;
    const wanted = new Set<string>();
    for (const manifest of manifests ?? []) {
        for (const font of manifest.themeFonts ?? []) {
            const version = manifest.version ?? "";
            const key = `${font.family}|${font.file}|${version}`; // 版本变化时重新注册，避免沿用旧文件。
            wanted.add(key);
            if (registeredFonts.has(key)) continue;
            const face = new FontFace(font.family, `url("/plugins/${font.file}?v=${version}")`, { weight: String(font.weight), style: font.style });
            fonts.add(face);
            registeredFonts.set(key, face);
        }
    }
    for (const [key, face] of registeredFonts) {
        if (wanted.has(key)) continue;
        fonts.delete(face); // 插件停用、卸载或改了字体文件时摘掉旧面板。
        registeredFonts.delete(key);
    }
};
