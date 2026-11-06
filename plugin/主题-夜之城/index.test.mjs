import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { applyThemeOptions, clearThemeOptions, nightCityReadout, normalizeBevelPixels, normalizeOptions } from "./index.js";

test("读出格式固定为 HH:MM，秒可选", () => {
    const stamp = new Date(2026, 10, 3, 23, 47, 5).getTime(); // 本机时间的固定时刻。
    assert.equal(nightCityReadout(stamp).clock, "23:47");
    assert.equal(nightCityReadout(stamp, true).clock, "23:47:05");
    assert.match(nightCityReadout(stamp).hex, /^0x[0-9A-F]{6}$/); // 固定宽度，走秒不抖动。
});

test("十六进制读出由秒数派生，同一秒稳定、跨秒变化", () => {
    const stamp = new Date(2026, 10, 3, 23, 47, 5, 100).getTime();
    assert.equal(nightCityReadout(stamp).hex, nightCityReadout(stamp + 800).hex); // 同一秒内的另一毫秒。
    assert.notEqual(nightCityReadout(stamp).hex, nightCityReadout(stamp + 1000).hex); // 下一秒。
});

test("特效开关默认全开，只有显式 false 才关闭", () => {
    assert.deepEqual(normalizeOptions(null), { scan: true, brandScan: true, tileScan: true, bevel: true, glow: true, mono: true });
    assert.deepEqual(normalizeOptions({ scan: false, glow: false }), { scan: false, brandScan: true, tileScan: true, bevel: true, glow: false, mono: true }); // 缺键按默认开启。
    assert.deepEqual(normalizeOptions({ scan: "off", brandScan: false, tileScan: false }), { scan: true, brandScan: false, tileScan: false, bevel: true, glow: true, mono: true }); // 非布尔值不当作关闭。
});

test("切角像素非法回落、合法值夹紧且允许零", () => {
    assert.equal(normalizeBevelPixels(undefined), 6);
    assert.equal(normalizeBevelPixels("8"), 6);
    assert.equal(normalizeBevelPixels(Number.NaN), 6);
    assert.equal(normalizeBevelPixels(-4), 0);
    assert.equal(normalizeBevelPixels(0), 0);
    assert.equal(normalizeBevelPixels(8.5), 8.5);
    assert.equal(normalizeBevelPixels(20), 12);
});

test("开关与切角变量写入文档根，清理后不留残留", () => {
    const properties = new Map();
    const root = {
        dataset: {},
        style: {
            setProperty: (name, value) => properties.set(name, value),
            removeProperty: (name) => properties.delete(name),
        },
    };
    applyThemeOptions(root, { bevel: false, bevelPixels: 9 });
    assert.deepEqual(root.dataset, { nightcityScan: "on", nightcityBrandScan: "on", nightcityTileScan: "on", nightcityBevel: "off", nightcityGlow: "on", nightcityMono: "on" });
    assert.equal(properties.get("--nightcity-bevel"), "9px");
    clearThemeOptions(root);
    assert.deepEqual(root.dataset, {});
    assert.equal(properties.has("--nightcity-bevel"), false);
});

test("设置品牌与时钟扫描遵守开关和减少动效", () => {
    const themeCss = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    const tileCss = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /key:\s*"brandScan",\s*label:\s*"设置标题扫描线"/);
    assert.match(source, /key:\s*"tileScan",\s*label:\s*"时钟扫描线"/);
    assert.match(themeCss, /\.settings__brand span\s*\{[^}]*color:\s*var\(--desktop-accent,\s*#fce300\)/s);
    assert.match(themeCss, /\.settings__brand::after\s*\{[^}]*repeating-linear-gradient[^}]*linear-gradient[^}]*mask-image:\s*linear-gradient\(90deg[^}]*animation:\s*nightcity-brand-scan 4200ms/s);
    assert.match(themeCss, /:root\[data-nightcity-brand-scan="off"\] \.settings__brand::after,[^{]*:root\[data-interface-animations="off"\] \.settings__brand::after\s*\{[^}]*display:\s*none/s);
    assert.match(themeCss, /@media\s*\(prefers-reduced-motion:\s*reduce\)[\s\S]*?\.settings__brand::after\s*\{[^}]*display:\s*none/s);
    assert.doesNotMatch(themeCss, /data-nightcity-scan="off"[^{}]*settings__brand span\s*\{[^}]*display:\s*none/);
    assert.match(tileCss, /\.nightcity-clock\s*\{[^}]*repeating-linear-gradient[^}]*linear-gradient[^}]*mask-image:\s*linear-gradient\(90deg[^}]*animation:\s*nightcity-clock-scan 4800ms/s);
    assert.match(tileCss, /:root\[data-nightcity-tile-scan="off"\] \.nightcity-clock,[^{]*:root\[data-nightcity-scan="off"\] \.nightcity-clock,[^{]*:root\[data-interface-animations="off"\] \.nightcity-clock\s*\{[^}]*animation:\s*none;[^}]*background:\s*none/s);
    assert.match(tileCss, /@media\s*\(prefers-reduced-motion:\s*reduce\)[\s\S]*?\.nightcity-clock\s*\{[^}]*animation:\s*none;[^}]*background:\s*none/s);
});

test("CRT 恢复为全屏叠加且不再裁字形", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    assert.match(css, /#root::after\s*\{[^}]*position:\s*fixed;[^}]*inset:\s*0;[^}]*z-index:\s*9999;[^}]*pointer-events:\s*none;[^}]*background:\s*var\(--nightcity-crt,\s*none\)/s);
    assert.match(css, /:root\[data-theme="dark"\]\s*\{[^}]*--nightcity-crt:[^}]*20%[^}]*84px[^}]*84px/s);
    assert.match(css, /:root\[data-theme="light"\]\s*\{[^}]*--nightcity-crt:[^}]*96px[^}]*96px/s);
    assert.match(css, /html\[data-desktop-host="true"\]\[data-desktop-theme="light"\]\s*\{[^}]*--nightcity-crt:[^}]*96px/s);
    assert.match(css, /:root\[data-nightcity-scan="off"\] #root::after\s*\{[^}]*display:\s*none/s);
    assert.match(css, /@media\s*\(max-width:\s*768px\)[\s\S]*?#root::after\s*\{[^}]*display:\s*none/s);
    assert.doesNotMatch(css, /nightcity-text-scan|background-clip:\s*text|-webkit-text-fill-color|desktop-item--drag-preview/);
});

test("主题装饰只落菜单，禁用控件不发悬停冷光", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8"); // 主题效果层。
    const checkboxCss = readFileSync(new URL("./style.css", import.meta.url), "utf8"); // 插件设置裸复选。
    const glowRules = [...css.matchAll(/(?:^|\n)\s*(?::root\[[^\n]+\]\s+)?button:not\(\[data-button="row-hit"\]\):hover:not\(:disabled\)\s*\{/g)];
    assert.equal(glowRules.length, 3); // 深色、浅色及 glow=off 三条都排除禁用。
    assert.match(css, /\.desktop-tile-menu,\s*\.folder-portal__breadcrumb-menu\s*\{[^}]*background-image:[^}]*background-size: 12px 1px, 1px 12px,[^}]*background-repeat: no-repeat;/s);
    assert.doesNotMatch(css, /(?:button|input|textarea|select)::(?:before|after)[^{]*background-image/);
    assert.match(checkboxCss, /\.nightcity-options \.nightcity-options__input\s*\{[^}]*border: 1px solid var\(--color-muted\);/s);
    assert.match(checkboxCss, /\.nightcity-options \.nightcity-options__input:disabled\s*\{[^}]*border-color: var\(--color-border\);/s);
    assert.doesNotMatch(checkboxCss, /input\[type="radio"\]/);
});

test("选中四角只由主题覆盖且不画整圈边框", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    const shared = readFileSync(new URL("../../src/features/desktop-items/DesktopItemGrid.css", import.meta.url), "utf8");
    assert.match(css, /\.desktop-item\[data-selected="true"\],[^{]*\.collect-box__item--selected,[^{]*\.desktop-tile:focus,[^{]*\.desktop-tile:focus-visible,[^{]*\.desktop-tile\[data-keyboard-focus="true"\]\s*\{[^}]*border-color:\s*transparent;[^}]*background-image:/s);
    const selection = css.match(/\/\* 桌面项目、收集条目和格子焦点[^}]+\}/s)?.[0] ?? "";
    assert.equal((selection.match(/linear-gradient\(/g) ?? []).length, 8);
    assert.match(selection, /background-size:\s*12px 1px, 1px 12px,[^;]+;/);
    assert.match(selection, /background-repeat:\s*no-repeat/);
    assert.match(css, /\.desktop-item\[data-selected="true"\]:hover\s*\{[^}]*background-color:/s);
    assert.doesNotMatch(shared, /background-image|background-size:\s*12px 1px/);
    assert.match(shared, /\.desktop-item\[data-selected="true"\]\s*\{[^}]*border-color:[^}]*background:/s);
});

test("门户选中、设置导航与下拉由主题覆盖", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    const portal = css.match(/\.folder-portal__items li\.is-selected\s*\{([^}]*)\}/s)?.[1] ?? "";
    assert.match(portal, /border-color:\s*transparent/);
    assert.equal((portal.match(/linear-gradient\(/g) ?? []).length, 8);
    assert.match(portal, /background-size:\s*12px 1px, 1px 12px,[^;]+;/);
    assert.match(css, /\.folder-portal__items li\.is-selected:hover\s*\{[^}]*background-color:/s);
    assert.match(css, /\.settings__navigation-item:hover,[^{]*\.settings__navigation-toggle:hover\s*\{[^}]*var\(--color-ready\)/s);
    assert.match(css, /\.settings__navigation-item\[aria-current="page"\]\s*\{[^}]*color:\s*var\(--color-text\);[^}]*var\(--color-focus\)/s);
    assert.match(css, /\.settings__navigation-item\[aria-current="page"\]::before\s*\{[^}]*var\(--color-focus\)/s);
    assert.match(css, /\.settings__select\s*\{[^}]*border-color:\s*var\(--color-border\);[^}]*color:\s*var\(--color-text\);[^}]*background-color:\s*var\(--color-subtle\)/s);
    assert.match(css, /:root\[data-theme="dark"\]\s*\{[^}]*color-scheme:\s*dark/s);
    assert.match(css, /:root\[data-theme="light"\]\s*\{[^}]*color-scheme:\s*light/s);
});

test("设置行、标题、输入与滚动条使用主题语言", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    const rows = css.match(/\.settings__row,[^{]*\.weather-settings__row\s*\{([^}]*)\}/s)?.[1] ?? "";
    assert.match(rows, /border-color:\s*var\(--color-border\)/);
    assert.match(rows, /border-radius:\s*0/);
    assert.match(rows, /background-color:\s*color-mix/);
    assert.doesNotMatch(rows, /background-image|linear-gradient/);
    assert.match(css, /\.settings__content,[^{]*\.feature-settings\s*\{[^}]*scrollbar-color:\s*var\(--color-border\) transparent/s);
    assert.match(css, /\.plugin-settings__label,[^{]*\.weather-settings__status\s*\{[^}]*font-family:[^}]*Cascadia Mono[^}]*letter-spacing:\s*0\.09375rem;[^}]*text-transform:\s*uppercase/s);
    assert.match(css, /input:not\(\[type="checkbox"\]\):not\(\[type="radio"\]\):hover:not\(:disabled\),[^{]*select:hover:not\(:disabled\),[^{]*textarea:hover:not\(:disabled\)\s*\{[^}]*border-color:/s);
});

test("切角只落左上与右下，关闭后所有参与表面回到直角", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body }));
    const bevel = rules.find(({ selector }) => selector.startsWith('button:not([data-button="row-hit"]),'));
    const disabled = rules.find(({ selector }) => selector.startsWith(':root[data-nightcity-bevel="off"] button:not([data-button="row-hit"]),'));
    assert.ok(bevel && disabled);
    assert.match(bevel.selector, /input:not\(\[type="checkbox"\]\):not\(\[type="radio"\]\),\s*select,\s*textarea$/s);
    assert.doesNotMatch(bevel.selector, /desktop-tile-menu|folder-portal__breadcrumb-menu|settings__row|__dialog|\.desktop-tile|\.desktop-items/);
    assert.match(bevel.body, /border-start-start-radius:\s*var\(--nightcity-bevel,\s*6px\)/);
    assert.match(bevel.body, /border-start-end-radius:\s*0/);
    assert.match(bevel.body, /border-end-start-radius:\s*0/);
    assert.match(bevel.body, /border-end-end-radius:\s*var\(--nightcity-bevel,\s*6px\)/);
    assert.match(bevel.body, /corner-top-left-shape:\s*bevel/);
    assert.match(bevel.body, /corner-bottom-right-shape:\s*bevel/);
    assert.doesNotMatch(bevel.body, /corner-top-right-shape:\s*bevel|corner-bottom-left-shape:\s*bevel/);
    assert.doesNotMatch(bevel.selector, /toast|#root/i);
    assert.doesNotMatch(bevel.selector, /(?:^|,)\s*\[data-button="row-hit"\](?:,|$)/);
    assert.match(disabled.body, /border-radius:\s*0/);
    assert.match(disabled.body, /corner-shape:\s*round/);
    assert.doesNotMatch(disabled.body, /bevel/);
});

test("夜之城独占 G8 切角开关几何与四态", () => {
    const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
    assert.match(css, /\.settings-switch__track\s*\{[^}]*padding:\s*0;[^}]*border-radius:\s*2px;[^}]*background:\s*var\(--color-surface\);/s);
    assert.match(css, /\.settings-switch__track::after\s*\{[^}]*inset-block-start:\s*1px;[^}]*inset-inline-start:\s*1px;[^}]*width:\s*16px;[^}]*height:\s*18px;[^}]*clip-path:\s*polygon\(3px 0,[^}]*55%/s);
    assert.match(css, /input:checked \+ \.settings-switch__track::after,[^{]*input:focus-visible \+ \.settings-switch__track::after\s*\{[^}]*inset-inline-start:\s*21px;[^}]*var\(--color-ready\)[^}]*drop-shadow\(0 0 5px[^}]*28%/s);
    assert.match(css, /input:focus-visible \+ \.settings-switch__track::before\s*\{[^}]*inset:\s*-3px;[^}]*border:\s*2px solid var\(--color-focus\);[^}]*border-radius:\s*4px;/s);
    assert.match(css, /input:disabled \+ \.settings-switch__track::after\s*\{[^}]*background:\s*var\(--color-border\);[^}]*filter:\s*none;/s);
    assert.doesNotMatch(css, /data-nightcity-bevel="off"[^{}]*settings-switch/s);
});

test("设置条目提供独立的切角像素输入", () => {
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /bevelLabel\.textContent = "切角像素"/);
    assert.match(source, /bevelHint\.textContent = "只调整切角量，与界面圆角无关"/);
    assert.match(source, /bevelInput\.type = "number"/);
    assert.match(source, /bevelInput\.min = "0"/);
    assert.match(source, /bevelInput\.max = "12"/);
});

test("主题外观默认直角", () => {
    const theme = JSON.parse(readFileSync(new URL("./theme.json", import.meta.url), "utf8"));
    assert.equal(theme.appearance.cornerRadius, 0);
});

test("复选已选态不换外观源或尺寸，强制色仍由系统绘制", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8"); // 复选框真实样式。
    const rules = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body })); // 去掉注释后逐块检查状态。
    const base = rules.find(({ selector }) => selector === ".nightcity-options .nightcity-options__input");
    const checked = rules.find(({ selector }) => selector === ".nightcity-options .nightcity-options__input:checked");
    const disabled = rules.find(({ selector }) => selector === ".nightcity-options .nightcity-options__input:disabled");
    assert.ok(base && checked && disabled);
    assert.match(base.body, /appearance:\s*none/);
    assert.match(base.body, /box-sizing:\s*border-box/);
    assert.match(base.body, /width:\s*16px/);
    assert.match(base.body, /height:\s*16px/);
    assert.match(checked.body, /background:/); // 勾由背景绘制，不依赖原生 appearance 切换。
    assert.doesNotMatch(checked.body, /appearance:|width:|height:|box-sizing:/);
    assert.doesNotMatch(disabled.body, /appearance:|width:|height:|box-sizing:/);
    assert.match(css, /@media\s*\(forced-colors:\s*active\)[\s\S]*?\.nightcity-options \.nightcity-options__input[^{]*\{[^}]*appearance:\s*auto/);
});
