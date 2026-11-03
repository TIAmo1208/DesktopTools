import assert from "node:assert/strict";
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
