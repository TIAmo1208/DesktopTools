import assert from "node:assert/strict";
import test from "node:test";

import { applyAppearanceToDocument, resolveAppearance, type AppearancePreferences } from "./appearanceProjection.ts";

/** 只带外观字段的偏好，其他字段与应用外观无关。 */
const preferences = (overrides: Partial<AppearancePreferences> = {}): AppearancePreferences => ({
    theme: "system",
    desktopAppearance: { theme: "system", accentColor: "#111111", useWindowsAccentColor: false },
    accentColor: "#22c55e",
    useWindowsAccentColor: false,
    cornerRadius: 8,
    islandRadius: 20,
    fontFamily: "宋律",
    fontSize: 14,
    desktopGrid: { width: 68, height: 90, columnGap: 10, rowGap: 8 },
    fenceGrid: { width: 68, height: 90, columnGap: 6, rowGap: 8 },
    ...overrides,
});

test("偏好未加载时只按系统深浅模式给出主题，不写变量", () => {
    assert.deepEqual(resolveAppearance(undefined, true, undefined), { theme: "dark", desktopTheme: "dark", variables: {} });
    assert.deepEqual(resolveAppearance(undefined, false, undefined), { theme: "light", desktopTheme: "light", variables: {} });
});

test("界面与桌面各自解析系统主题", () => {
    const projection = resolveAppearance(preferences({ theme: "light" }), true, undefined);
    assert.equal(projection.theme, "light");
    assert.equal(projection.desktopTheme, "dark");
    assert.equal(resolveAppearance(preferences({ desktopAppearance: { theme: "dark", accentColor: "#111111", useWindowsAccentColor: false } }), false, undefined).desktopTheme, "dark");
});

test("强调色按界面与桌面各自决定是否跟随系统，读不到系统色时回退自定义色", () => {
    const followed = resolveAppearance(preferences({ useWindowsAccentColor: true, desktopAppearance: { theme: "system", accentColor: "#333333", useWindowsAccentColor: true } }), false, "#0ea5e9");
    assert.equal(followed.variables["--color-accent"], "#0ea5e9");
    assert.equal(followed.variables["--desktop-accent"], "#0ea5e9");
    const missing = resolveAppearance(preferences({ useWindowsAccentColor: true }), false, undefined);
    assert.equal(missing.variables["--color-accent"], "#22c55e");
    const custom = resolveAppearance(preferences(), false, "#0ea5e9");
    assert.equal(custom.variables["--color-accent"], "#22c55e");
    assert.equal(custom.variables["--desktop-accent"], "#111111");
});

test("圆角、字体与两组网格写到对应变量", () => {
    const { variables } = resolveAppearance(preferences(), false, undefined);
    assert.equal(variables["--surface-radius"], "8px");
    assert.equal(variables["--island-radius"], "20px");
    assert.equal(variables["--interface-font-size"], "14px");
    assert.match(variables["--interface-font"], /宋律/);
    assert.equal(variables["--desktop-grid-width"], "68px");
    assert.equal(variables["--desktop-grid-column-gap"], "10px");
    assert.equal(variables["--fence-grid-row-gap"], "8px");
});

test("把解析结果写到文档根的数据集与样式属性", () => {
    const written: Record<string, string> = {};
    const root = {
        documentElement: {
            dataset: {} as Record<string, string>,
            style: {
                setProperty: (name: string, value: string) => {
                    written[name] = value;
                },
            },
        },
    };
    applyAppearanceToDocument(root, { theme: "dark", desktopTheme: "light", variables: { "--color-accent": "#0ea5e9" } });
    assert.equal(root.documentElement.dataset.theme, "dark");
    assert.equal(root.documentElement.dataset.desktopTheme, "light");
    assert.deepEqual(written, { "--color-accent": "#0ea5e9" });
});
