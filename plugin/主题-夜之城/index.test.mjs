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
    assert.deepEqual(normalizeOptions(null), { scan: true, bevel: true, glow: true, mono: true });
    assert.deepEqual(normalizeOptions({ scan: false, glow: false }), { scan: false, bevel: true, glow: false, mono: true }); // 缺键按默认开启。
    assert.deepEqual(normalizeOptions({ scan: "off" }), { scan: true, bevel: true, glow: true, mono: true }); // 非布尔值不当作关闭。
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
    assert.deepEqual(root.dataset, { nightcityScan: "on", nightcityBevel: "off", nightcityGlow: "on", nightcityMono: "on" });
    assert.equal(properties.get("--nightcity-bevel"), "9px");
    clearThemeOptions(root);
    assert.deepEqual(root.dataset, {});
    assert.equal(properties.has("--nightcity-bevel"), false);
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
