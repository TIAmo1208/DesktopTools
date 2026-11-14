import assert from "node:assert/strict";
import test from "node:test";
import { parsePreferences } from "../../contracts/settings.ts";
import type { ComponentManifest, ThemeAppearance } from "../../contracts/component.ts";
import { applyThemeSelection, clearTheme, selectTheme } from "./applyThemeSelection.ts";

/** 只构造本测试需要的合法主题清单。 */
const theme = (id: string, appearance: Partial<ThemeAppearance>): ComponentManifest => ({
    id,
    name: id,
    role: "theme",
    entry: "plugin:slot",
    script: "",
    stylesheet: "theme/style.css",
    requiredCapabilities: [],
    creatable: false,
    removable: true,
    maxTiles: 0,
    persistenceVersion: 1,
    themeAppearance: { accentColor: null, desktopAccentColor: null, cornerRadius: null, islandRadius: null, fontFamily: null, fontSize: null, ...appearance },
});

const original = parsePreferences({
    theme: "dark",
    doubleClickToOpen: true,
    accentColor: "#123456",
    fontFamily: "Arial",
    fontSize: 14,
    cornerRadius: 4,
    islandRadius: 8,
    useWindowsAccentColor: true,
}); // 非默认用户外观。

test("首次选择捕获原值且一次保存，换主题不更新基线并清除旧覆盖", async () => {
    let calls = 0; // 真实调用次数，而非只断言构造结果。
    const first = await applyThemeSelection(
        original,
        theme("plugin.first", { accentColor: "#abcdef", fontFamily: "SimSun", cornerRadius: 12, islandRadius: 16 }),
        async (next) => {
            calls++;
            assert.equal(original.themeBaseline, null);
            return next;
        },
    );
    assert.equal(calls, 1);
    assert.deepEqual(first.themeBaseline, {
        accentColor: "#123456",
        desktopAccentColor: "#2563eb",
        fontFamily: "Arial",
        fontSize: 14,
        cornerRadius: 4,
        islandRadius: 8,
    });
    assert.equal(first.fontFamily, "SimSun");
    assert.equal(first.islandRadius, 16);
    assert.equal(first.useWindowsAccentColor, true);
    const second = selectTheme(first, theme("plugin.second", { fontSize: 18, desktopAccentColor: "#445566" })); // 不声明字体与界面色。
    assert.deepEqual(second.themeBaseline, first.themeBaseline);
    assert.equal(second.fontFamily, "Arial");
    assert.equal(second.accentColor, "#123456");
    assert.equal(second.cornerRadius, 4);
    assert.equal(second.islandRadius, 8);
    assert.equal(second.fontSize, 18);
    assert.equal(second.desktopAppearance.accentColor, "#445566");
    assert.deepEqual(clearTheme(second), original);
});

test("切回默认、停用、卸载和缺失均可单次提交还原，无基线保留当前值", async () => {
    const selected = selectTheme(original, theme("plugin.first", { fontSize: 18 }));
    let calls = 0;
    assert.deepEqual(
        await applyThemeSelection(selected, null, async (next) => {
            calls++;
            return next;
        }),
        original,
    );
    assert.equal(calls, 1);
    assert.deepEqual(clearTheme({ ...original, themePluginId: "plugin.missing" }), original);
    assert.deepEqual(clearTheme({ ...selected, disabledPlugins: ["plugin.first"] }), { ...original, disabledPlugins: ["plugin.first"] });
});

test("非法旧基线按字段回退默认值且还原不抛错", () => {
    const selected = selectTheme(original, theme("plugin.first", {}));
    const cleared = clearTheme({
        ...selected,
        themeBaseline: { ...selected.themeBaseline!, fontFamily: "Arial; color:red", fontSize: 100, cornerRadius: -1, islandRadius: 17, accentColor: "red" },
    });
    assert.equal(cleared.fontFamily, "Segoe UI Variable");
    assert.equal(cleared.fontSize, 13);
    assert.equal(cleared.cornerRadius, 6);
    assert.equal(cleared.islandRadius, 12);
    assert.equal(cleared.accentColor, "#2563eb");
    assert.equal(cleared.themeBaseline, null);
});

test("主题自带字体不进入基线，切回默认主题回到系统字体", () => {
    const withThemeFont = { ...original, fontFamily: "宋律宋体" }; // 用户先从字体列表选中主题字体。
    const songlv = {
        ...theme("plugin.songlv", { fontFamily: "宋律宋体" }),
        themeFonts: [{ family: "宋律宋体", file: "主题-宋律/fonts/songti.woff2", weight: 400, style: "normal" as const }],
    };
    const selected = selectTheme(withThemeFont, songlv);
    assert.equal(selected.fontFamily, "宋律宋体");
    assert.equal(selected.themeBaseline?.fontFamily, "Segoe UI Variable");
    assert.equal(clearTheme(selected).fontFamily, "Segoe UI Variable");
});

test("已被主题字体污染的旧基线在停用或切回默认时回到系统字体", () => {
    const polluted = {
        ...original,
        themePluginId: "plugin.songlv",
        fontFamily: "宋律宋体",
        themeBaseline: {
            ...(original.themeBaseline ?? { accentColor: "#123456", desktopAccentColor: "#2563eb", cornerRadius: 4, islandRadius: 8, fontSize: 14 }),
            fontFamily: "宋律宋体",
        },
    };
    assert.equal(clearTheme(polluted).fontFamily, "Segoe UI Variable");
    assert.equal(clearTheme({ ...polluted, disabledPlugins: ["plugin.songlv"] }).fontFamily, "Segoe UI Variable");
});

test("保存失败不修改原偏好，不允许选择停用或非主题清单", async () => {
    const snapshot = structuredClone(original);
    await assert.rejects(
        applyThemeSelection(original, theme("plugin.first", {}), async () => {
            throw new Error("保存失败");
        }),
    );
    assert.deepEqual(original, snapshot);
    assert.throws(() => selectTheme({ ...original, disabledPlugins: ["plugin.first"] }, theme("plugin.first", {})));
    assert.throws(() => selectTheme(original, { ...theme("plugin.first", {}), role: "tile" }));
});
