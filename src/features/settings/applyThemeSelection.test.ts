import assert from "node:assert/strict";
import test from "node:test";
import { parsePreferences } from "../../contracts/settings.ts";
import type { ComponentManifest, ThemeAppearance } from "../../contracts/component.ts";
import { applyThemeSelection, clearTheme, selectTheme } from "./applyThemeSelection.ts";

/** 只构造本测试需要的合法主题清单。 */
const theme = (id: string, appearance: Partial<ThemeAppearance>): ComponentManifest => ({
    id, name: id, role: "theme", entry: "plugin:slot", script: "", stylesheet: "theme/style.css",
    requiredCapabilities: [], creatable: false, removable: true, maxTiles: 0, persistenceVersion: 1,
    themeAppearance: { accentColor: null, desktopAccentColor: null, cornerRadius: null, fontFamily: null, fontSize: null, ...appearance },
});

const original = parsePreferences({ theme: "dark", doubleClickToOpen: true, accentColor: "#123456", fontFamily: "Arial", fontSize: 14, cornerRadius: 4, useWindowsAccentColor: true }); // 非默认用户外观。

test("首次选择捕获原值且一次保存，换主题不更新基线并清除旧覆盖", async () => {
    let calls = 0; // 真实调用次数，而非只断言构造结果。
    const first = await applyThemeSelection(original, theme("plugin.first", { accentColor: "#abcdef", fontFamily: "SimSun", cornerRadius: 12 }), async (next) => {
        calls++;
        assert.equal(original.themeBaseline, null);
        return next;
    });
    assert.equal(calls, 1);
    assert.deepEqual(first.themeBaseline, { accentColor: "#123456", desktopAccentColor: "#2563eb", fontFamily: "Arial", fontSize: 14, cornerRadius: 4 });
    assert.equal(first.fontFamily, "SimSun");
    assert.equal(first.useWindowsAccentColor, true);
    const second = selectTheme(first, theme("plugin.second", { fontSize: 18, desktopAccentColor: "#445566" })); // 不声明字体与界面色。
    assert.deepEqual(second.themeBaseline, first.themeBaseline);
    assert.equal(second.fontFamily, "Arial");
    assert.equal(second.accentColor, "#123456");
    assert.equal(second.cornerRadius, 4);
    assert.equal(second.fontSize, 18);
    assert.equal(second.desktopAppearance.accentColor, "#445566");
    assert.deepEqual(clearTheme(second), original);
});

test("切回默认、停用、卸载和缺失均可单次提交还原，无基线保留当前值", async () => {
    const selected = selectTheme(original, theme("plugin.first", { fontSize: 18 }));
    let calls = 0;
    assert.deepEqual(await applyThemeSelection(selected, null, async (next) => { calls++; return next; }), original);
    assert.equal(calls, 1);
    assert.deepEqual(clearTheme({ ...original, themePluginId: "plugin.missing" }), original);
    assert.deepEqual(clearTheme({ ...selected, disabledPlugins: ["plugin.first"] }), { ...original, disabledPlugins: ["plugin.first"] });
});

test("非法旧基线按字段回退默认值且还原不抛错", () => {
    const selected = selectTheme(original, theme("plugin.first", {}));
    const cleared = clearTheme({ ...selected, themeBaseline: { ...selected.themeBaseline!, fontFamily: "Unknown", fontSize: 100, cornerRadius: -1, accentColor: "red" } });
    assert.equal(cleared.fontFamily, "Segoe UI Variable");
    assert.equal(cleared.fontSize, 13);
    assert.equal(cleared.cornerRadius, 6);
    assert.equal(cleared.accentColor, "#2563eb");
    assert.equal(cleared.themeBaseline, null);
});

test("保存失败不修改原偏好，不允许选择停用或非主题清单", async () => {
    const snapshot = structuredClone(original);
    await assert.rejects(applyThemeSelection(original, theme("plugin.first", {}), async () => { throw new Error("保存失败"); }));
    assert.deepEqual(original, snapshot);
    assert.throws(() => selectTheme({ ...original, disabledPlugins: ["plugin.first"] }, theme("plugin.first", {})));
    assert.throws(() => selectTheme(original, { ...theme("plugin.first", {}), role: "tile" }));
});
