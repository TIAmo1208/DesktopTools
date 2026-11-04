import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { applyThemeOptions, clearThemeOptions, nightCityReadout, normalizeOptions } from "./index.js";

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

test("开关写成 data-nightcity-* 属性，清理后不留残留", () => {
    const root = { dataset: {} }; // 只需 dataset 的最小替身。
    applyThemeOptions(root, { bevel: false });
    assert.deepEqual(root.dataset, { nightcityScan: "on", nightcityBevel: "off", nightcityGlow: "on", nightcityMono: "on" });
    clearThemeOptions(root);
    assert.deepEqual(root.dataset, {});
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
