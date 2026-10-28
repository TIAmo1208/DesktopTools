import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "vite";

const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
const page = await server.ssrLoadModule("/src/features/settings/PluginSettingsPage.tsx");
const scripts = await server.ssrLoadModule("/src/bridge/pluginScripts.ts");
test.after(async () => {
    await server.close();
});

test("插件清单只保留 plugin. 前缀并按显示名排序", () => {
    const rows = page.pluginRows([
        { id: "official.clock", name: "时间" },
        { id: "plugin.b", name: "天气" },
        { id: "plugin.a", name: "时钟" },
    ]);
    assert.deepEqual(
        rows.map((row) => row.id),
        ["plugin.a", "plugin.b"],
    );
    assert.deepEqual(
        rows.map((row) => row.name),
        ["时钟", "天气"].sort((left, right) => left.localeCompare(right, "zh")),
    );
});

test("条目数说明区分停用、失败、加载中、没有条目和条目数", async () => {
    const previous = globalThis.window;
    globalThis.window = {
        __DESKTOP_TOOLS_NATIVE__: {
            invoke(command, payload) {
                if (command === "get_preferences") return { theme: "system", doubleClickToOpen: true, disabledPlugins: [] };
                if (command === "load_plugin_script" && payload.pluginId === "plugin.fail-summary") throw new Error("插件脚本执行失败");
                return null;
            },
        },
    };
    try {
        scripts.adoptPluginManifests([{ id: "plugin.entries" }, { id: "plugin.empty-entries" }]);
        scripts.pluginShell["plugin.entries"] = { renderTile() {}, settingsEntries: [{ id: "a", name: "甲", render() {} }] };
        scripts.pluginShell["plugin.empty-entries"] = { renderTile() {} };
        assert.deepEqual(
            page.entriesFor("plugin.entries").map((entry) => entry.id),
            ["a"],
        );
        assert.deepEqual(page.entriesFor("plugin.empty-entries"), []);
        assert.equal(page.entriesFor("plugin.missing-summary"), undefined);
        assert.equal(page.entrySummary("plugin.entries", false, true), "1 个设置条目");
        assert.equal(page.entrySummary("plugin.empty-entries", false, true), "没有设置条目");
        assert.equal(page.entrySummary("plugin.missing-summary", false, false), "正在加载插件设置…");
        assert.equal(page.entrySummary("plugin.missing-summary", true, false), "已停用");
        assert.equal(await scripts.loadPluginScript("plugin.fail-summary"), "插件脚本执行失败");
        assert.equal(page.entrySummary("plugin.fail-summary", false, false), "设置条目加载失败");
    } finally {
        globalThis.window = previous;
    }
});
