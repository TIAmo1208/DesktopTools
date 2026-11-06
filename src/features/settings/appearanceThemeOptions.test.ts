import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import { parsePreferences } from "../../contracts/settings.ts";
import type { ComponentManifest } from "../../contracts/component.ts";
import { appearanceThemeOptions } from "./appearanceThemeOptions.ts";

/** 最小清单保持契约完整，不依赖磁盘插件。 */
const theme = (id: string, name: string): ComponentManifest => ({
    id,
    name,
    role: "theme",
    entry: "plugin:slot",
    script: "",
    requiredCapabilities: [],
    creatable: false,
    removable: true,
    maxTiles: 0,
    persistenceVersion: 1,
});

test("默认固定首项，主题按中文排序并过滤格子与停用项", () => {
    const manifests = [
        theme("plugin.b", "紫夜"),
        theme("plugin.a", "白昼"),
        theme("plugin.off", "停用"),
        { ...theme("plugin.tile", "格子"), role: "tile" as const },
    ];
    const before = structuredClone(manifests);
    const result = appearanceThemeOptions(manifests, ["plugin.off"]);
    assert.deepEqual(result, [
        { id: "", name: "默认" },
        ...manifests
            .slice(0, 2)
            .sort((a, b) => a.name.localeCompare(b.name, "zh"))
            .map(({ id, name }) => ({ id, name })),
    ]);
    assert.deepEqual(manifests, before);
});

test("读取中或失败未取得清单、未安装与全部停用均保留默认项", () => {
    const defaults = [{ id: "", name: "默认" }];
    assert.deepEqual(appearanceThemeOptions(undefined, []), defaults);
    assert.deepEqual(appearanceThemeOptions([], []), defaults);
    assert.deepEqual(appearanceThemeOptions([theme("plugin.off", "停用")], ["plugin.off"]), defaults);
});

test("没有主题清单时仍渲染可用下拉并选中默认", async () => {
    const server = await createServer({ server: { middlewareMode: true }, appType: "custom" }); // 复用现有组件测试转换，不启动端口。
    try {
        const { AppearanceSettings } = await server.ssrLoadModule("/src/features/settings/AppearanceSettings.tsx");
        const preferences = parsePreferences({ theme: "light", doubleClickToOpen: true, themePluginId: "plugin.missing" }); // 清单未知时也不得呈现空值。
        const markup = renderToStaticMarkup(
            createElement(AppearanceSettings, { preferences, disabled: false, onPreferences: () => {}, onThemeSelection: () => {} }),
        );
        const select = markup.match(/<select[^>]*id="interface-theme-plugin"[^>]*>([\s\S]*?)<\/select>/)?.[0];
        assert.ok(select);
        assert.match(select, /class="appearance-settings__theme"/);
        assert.match(select, /title="默认"/);
        assert.match(select, /<option[^>]*value=""[^>]*selected=""[^>]*>默认<\/option>/);
        assert.equal((select.match(/<option/g) ?? []).length, 1);
        assert.doesNotMatch(select, /disabled/);
        assert.doesNotMatch(markup, /appearance-settings__note|未安装主题插件|正在读取主题/);
    } finally {
        await server.close();
    }
});

test("长主题名保留完整文本用于 title", () => {
    const name = "非常长的主题名称".repeat(20);
    assert.equal(appearanceThemeOptions([theme("plugin.long", name)], [])[1].name, name);
});
