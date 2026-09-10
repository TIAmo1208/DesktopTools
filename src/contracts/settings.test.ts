import assert from "node:assert/strict";
import test from "node:test";

import { parsePreferences, parseSettingsSnapshot } from "./settings.ts";

test("界面动画使用默认值并严格校验布尔设置", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 缺失字段使用当前默认值。
    assert.equal(defaults.interfaceAnimations, true);
    const disabled = parsePreferences({ ...defaults, interfaceAnimations: false }); // 关闭值必须经过保存回读保留。
    assert.equal(parsePreferences(JSON.parse(JSON.stringify(disabled))).interfaceAnimations, false);
    for (const value of [null, 0, "false"]) assert.throws(() => parsePreferences({ ...defaults, interfaceAnimations: value }));
});

// 桌面主题缺失时使用固定默认值，后续修改、系统色开关及序列化均不能串组。
test("keeps desktop appearance independent from interface and menu appearance", () => {
    const legacy = parsePreferences({ theme: "dark", doubleClickToOpen: true, accentColor: "#aabbcc", useWindowsAccentColor: true }); // 用户原有的非默认主题。
    assert.deepEqual(legacy.desktopAppearance, { theme: "system", accentColor: "#2563eb", useWindowsAccentColor: false });
    const changed = parsePreferences({ ...legacy, theme: "light", accentColor: "#087f5b", useWindowsAccentColor: false }); // 只修改界面与菜单。
    assert.deepEqual(changed.desktopAppearance, legacy.desktopAppearance);
    const desktopChanged = parsePreferences({ ...changed, desktopAppearance: { theme: "system", accentColor: "#b42318", useWindowsAccentColor: false } }); // 只修改桌面。
    assert.equal(desktopChanged.theme, changed.theme);
    assert.equal(desktopChanged.accentColor, changed.accentColor);
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify(desktopChanged))), desktopChanged);
    for (const desktopAppearance of [
        null,
        {},
        { ...legacy.desktopAppearance, theme: "invalid" },
        { ...legacy.desktopAppearance, accentColor: "red" },
        { ...legacy.desktopAppearance, useWindowsAccentColor: "true" },
    ]) {
        assert.throws(() => parsePreferences({ ...legacy, desktopAppearance }));
    }
});

// 偏好和启动状态必须通过实际边界校验，不能把字符串当作开关状态。
test("validates application preferences and settings snapshots", () => {
    for (const theme of ["system", "light", "dark"]) {
        const preferences = { theme, doubleClickToOpen: false, accentColor: "#2563eb", cornerRadius: 6, fontFamily: "Segoe UI Variable", fontSize: 13 }; // 当前被校验的有效主题偏好。
        const migrated = parsePreferences(preferences); // 兼容默认值与完整契约。
        assert.equal(migrated.theme, theme);
        assert.deepEqual(parseSettingsSnapshot({ preferences, autostart: true, version: "0.1.0", debugBuild: true }), {
            preferences: migrated,
            autostart: true,
            version: "0.1.0",
            debugBuild: true,
        });
    }
    for (const invalid of [null, {}, { theme: "unknown", doubleClickToOpen: true }, { theme: "light", doubleClickToOpen: "false" }])
        assert.throws(() => parsePreferences(invalid));
    assert.throws(() => parseSettingsSnapshot({ preferences: { theme: "system", doubleClickToOpen: true }, autostart: "true", version: "0.1.0" }));
});

// 网格分组互不覆盖，拒绝无效数字及伪布尔值。
test("validates independent grids and feature preferences", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 旧配置迁移结果。
    const desktopGrid = { width: 100, height: 110, columnGap: 12, rowGap: 16 }; // 独立桌面格子。
    const changed = parsePreferences({ ...defaults, desktopGrid, clockEnabled: false, debugEnabled: false }); // 隐藏不删除实例的设置意图。
    assert.deepEqual(changed.desktopGrid, desktopGrid);
    assert.deepEqual(changed.fenceGrid, defaults.fenceGrid);
    assert.equal(changed.clockEnabled, false);
    assert.equal(defaults.clockShowSeconds, true);
    for (const patch of [{ width: NaN }, { width: 63 }, { height: 89 }, { rowGap: -1 }, { columnGap: 49 }, { width: 100.5 }]) {
        assert.throws(() => parsePreferences({ ...defaults, desktopGrid: { ...desktopGrid, ...patch } }));
    }
    assert.throws(() => parsePreferences({ ...defaults, clockEnabled: "false" }));
    assert.throws(() => parsePreferences({ ...defaults, fenceGrid: null }));
    assert.throws(() => parseSettingsSnapshot({ preferences: defaults, autostart: false, version: "1", debugBuild: "true" }));
});

// 旧配置采用明确默认值，越界设置和 CSS 注入片段不能进入主题应用路径。
test("migrates old preferences and rejects invalid appearance values", () => {
    const legacy = { theme: "light", doubleClickToOpen: true }; // 旧版配置的完整载荷。
    assert.equal(parsePreferences(legacy).fontSize, 13);
    assert.equal(parsePreferences(legacy).configDirectory, "%APPDATA%\\TIAmo\\DesktopTools");
    assert.equal(parsePreferences({ ...legacy, configDirectory: "D:\\DesktopTools" }).configDirectory, "D:\\DesktopTools");
    for (const configDirectory of [null, false, "", " "]) assert.throws(() => parsePreferences({ ...legacy, configDirectory }));
    assert.equal(parsePreferences(legacy).cornerRadius, 6);
    assert.equal(parsePreferences(legacy).useWindowsAccentColor, false);
    const systemAccent = parsePreferences({ ...legacy, useWindowsAccentColor: true, accentColor: "#AABBCC" }); // 跟随系统时仍保存自定义色。
    assert.equal(systemAccent.useWindowsAccentColor, true);
    assert.equal(systemAccent.accentColor, "#AABBCC");
    assert.equal(parsePreferences({ ...systemAccent, useWindowsAccentColor: false }).accentColor, "#AABBCC");
    for (const patch of [
        { accentColor: "red" },
        { accentColor: null },
        { useWindowsAccentColor: "true" },
        { useWindowsAccentColor: null },
        { cornerRadius: -1 },
        { cornerRadius: 13 },
        { fontSize: 19 },
        { fontSize: 12.5 },
        { fontFamily: "Arial; color:red" },
    ]) {
        assert.throws(() => parsePreferences({ ...legacy, ...patch }));
    }
    assert.equal(parsePreferences({ ...legacy, accentColor: "#AABBCC", cornerRadius: 0, fontSize: 18 }).cornerRadius, 0);
});
