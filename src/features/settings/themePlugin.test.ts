import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { activeThemeStylesheet, applyThemeStylesheet, canSavePreferences, readThemePluginId, recoverUnavailableTheme, resolveTheme, startThemeRecovery } from "./themePlugin.ts";
import { parsePreferences } from "../../contracts/settings.ts";
import { clearTheme } from "./applyThemeSelection.ts";

test("三类失效各提交一次并还原五项，广播无主题后不形成保存循环", async () => {
    const base = parsePreferences({ theme: "dark", doubleClickToOpen: true }); // 与真实保存相同的完整偏好。
    const selected = {
        ...base,
        themePluginId: "plugin.theme",
        fontSize: 18,
        themeBaseline: { accentColor: "#123456", desktopAccentColor: "#654321", cornerRadius: 4, fontFamily: "Arial", fontSize: 14 },
    };
    const valid = { manifests: [{ id: "plugin.theme", role: "theme", stylesheet: "theme.css" }] };
    for (const [reason, snapshot, preferences, message] of [
        ["disabled", valid, { ...selected, disabledPlugins: ["plugin.theme"] }, "主题已停用"],
        ["removed", { manifests: [] }, selected, "主题已移除"],
        ["unavailable", { manifests: [{ id: "plugin.theme", role: "tile" }] }, selected, "主题不可用"],
        ["unavailable", { manifests: [{ id: "plugin.theme", role: "theme" }] }, selected, "主题不可用"],
    ] as const) {
        let calls = 0;
        const state = { key: "" }; // 每份文档自己的去重记录。
        const save = async (next: typeof base) => {
            calls++;
            return next;
        };
        assert.equal(resolveTheme(snapshot, preferences).reason, reason);
        const first = recoverUnavailableTheme(state, snapshot, preferences, save);
        assert.equal(await recoverUnavailableTheme(state, snapshot, preferences, save), null); // 保存中重复推送不重复提交。
        const result = await first;
        assert.equal(calls, 1);
        assert.ok(result?.message.startsWith(message));
        assert.deepEqual(result?.preferences, clearTheme(preferences));
        assert.equal(result?.preferences.themePluginId, null);
        assert.equal(result?.preferences.themeBaseline, null);
        assert.equal(result?.preferences.fontFamily, "Arial");
        assert.equal(await recoverUnavailableTheme(state, snapshot, preferences, save), null);
        assert.equal(await recoverUnavailableTheme(state, snapshot, result!.preferences, save), null);
        assert.equal(calls, 1);
        await recoverUnavailableTheme(state, valid, selected, save); // 再次恢复有效主题，重置连续失效记录。
        await recoverUnavailableTheme(state, snapshot, preferences, save);
        assert.equal(calls, 2);
    }
});

test("回退保存失败不乐观更新也不无限重试，未知快照不认作移除", async () => {
    const selected = parsePreferences({ theme: "light", doubleClickToOpen: true, themePluginId: "plugin.theme" });
    const original = structuredClone(selected);
    const state = { key: "" };
    let calls = 0;
    const fail = async () => {
        calls++;
        throw new Error("保存失败");
    };
    assert.equal(await recoverUnavailableTheme(state, undefined, selected, fail), null);
    await assert.rejects(recoverUnavailableTheme(state, { manifests: [] }, selected, fail), /保存失败/);
    assert.equal(await recoverUnavailableTheme(state, { manifests: [] }, selected, fail), null);
    assert.equal(calls, 1);
    assert.deepEqual(selected, original);
});

test("没有偏好写入能力时不保存，有能力时照常还原", async () => {
    const previous = globalThis.window; // 测完还原，避免影响同文件的其他用例。
    const selected = parsePreferences({
        theme: "light",
        doubleClickToOpen: true,
        themePluginId: "plugin.theme",
        fontSize: 18,
        themeBaseline: { accentColor: "#123456", desktopAccentColor: "#654321", cornerRadius: 4, fontFamily: "Arial", fontSize: 14 },
    });
    let calls = 0; // 实际发起的保存次数。
    const save = async (next: typeof selected) => {
        calls++;
        return next;
    };
    try {
        globalThis.window = { __DESKTOP_TOOLS_NATIVE__: { invoke() {} } } as unknown as Window & typeof globalThis;
        assert.equal(canSavePreferences(), false);
        assert.equal(await startThemeRecovery({ key: "" }, { manifests: [] }, selected, save), null);
        assert.equal(calls, 0);
        globalThis.window = {} as unknown as Window & typeof globalThis;
        assert.equal(canSavePreferences(), true);
        const result = await startThemeRecovery({ key: "" }, { manifests: [] }, selected, save);
        assert.equal(calls, 1);
        assert.equal(result?.preferences.themePluginId, null);
        assert.match(result?.message ?? "", /^主题已移除/);
        const source = readFileSync(new URL("./themePlugin.ts", import.meta.url), "utf8"); // 判别只看原生桥，不看窗口标题或地址。
        assert.match(source, /__DESKTOP_TOOLS_NATIVE__/);
        assert.doesNotMatch(source, /document\.title|location\.href/);
        const provider = readFileSync(new URL("./PreferencesProvider.tsx", import.meta.url), "utf8");
        assert.match(provider, /startThemeRecovery\(/);
        assert.doesNotMatch(provider, /recoverUnavailableTheme\(/);
    } finally {
        globalThis.window = previous;
    }
});

test("两个文档并发回退写入相同结果，均不写回旧主题标识", async () => {
    const selected = parsePreferences({
        theme: "light",
        doubleClickToOpen: true,
        themePluginId: "plugin.theme",
        fontSize: 18,
        themeBaseline: { accentColor: "#123456", desktopAccentColor: "#654321", cornerRadius: 4, fontFamily: "Arial", fontSize: 14 },
    });
    const writes: unknown[] = [];
    const save = async (next: typeof selected) => {
        writes.push(next);
        return next;
    };
    await Promise.all([
        recoverUnavailableTheme({ key: "" }, { manifests: [] }, selected, save),
        recoverUnavailableTheme({ key: "" }, { manifests: [] }, selected, save),
    ]);
    assert.equal(writes.length, 2);
    assert.deepEqual(writes[0], writes[1]);
    assert.deepEqual(writes[0], clearTheme(selected));
});

test("未停用的主题清单才提供样式表", () => {
    const snapshot = {
        manifests: [
            { id: "plugin.theme", role: "theme", stylesheet: "示例-主题/theme.css", version: "2", script: "示例-主题/main.js" },
            { id: "plugin.tile", role: "tile", stylesheet: "nope.css", version: "1" },
        ],
    }; // 主题与格子各一份；脚本字段不得进入装配结果。
    assert.equal(readThemePluginId(undefined), undefined);
    assert.equal(readThemePluginId({}), undefined);
    assert.equal(readThemePluginId({ themePluginId: "" }), undefined);
    assert.equal(readThemePluginId({ themePluginId: 1 }), undefined);
    assert.equal(readThemePluginId({ themePluginId: "plugin.theme" }), "plugin.theme");
    assert.deepEqual(activeThemeStylesheet(snapshot, { disabledPlugins: [], themePluginId: "plugin.theme" }), {
        stylesheet: "示例-主题/theme.css",
        version: "2",
    });
    assert.equal(activeThemeStylesheet(snapshot, { disabledPlugins: ["plugin.theme"], themePluginId: "plugin.theme" }), null);
    assert.equal(activeThemeStylesheet(snapshot, { disabledPlugins: [], themePluginId: "plugin.tile" }), null);
    assert.equal(activeThemeStylesheet(snapshot, { disabledPlugins: [], themePluginId: "plugin.missing" }), null);
    assert.equal(activeThemeStylesheet(snapshot, { disabledPlugins: [] }), null);
    assert.equal(activeThemeStylesheet(undefined, { disabledPlugins: [], themePluginId: "plugin.theme" }), null);
    const themeSource = readFileSync(new URL("./themePlugin.ts", import.meta.url), "utf8"); // 确认主题路径不碰脚本加载。
    assert.doesNotMatch(themeSource, /loadPluginScript|import\(/);
    assert.match(themeSource, /disabledPlugins/);
    assert.match(themeSource, /role === "theme"/);
});

test("主题样式挂在偏好变量副作用之后", () => {
    const source = readFileSync(new URL("./PreferencesProvider.tsx", import.meta.url), "utf8"); // 顺序以源码中的 effect 声明为准。
    const variables = source.indexOf('setProperty("--color-focus"');
    const theme = source.indexOf("applyThemeStylesheet(");
    assert.ok(variables > 0 && theme > variables);
    assert.match(source.slice(variables, theme), /useEffect\(/);
    assert.match(source, /activeThemeStylesheet\(/);
    assert.match(source, /getComponentRuntimeSnapshot/);
});

test("主题 link 保持单例，切换摘旧，无主题无残留", () => {
    const events: string[] = []; // append 与 remove 的实际顺序。
    const links: Link[] = []; // 当前还挂在 head 上的主题 link。
    type Link = {
        rel: string;
        attributes: Map<string, string>;
        onerror: (() => void) | null;
        isConnected: boolean;
        setAttribute(name: string, value: string): void;
        getAttribute(name: string): string | null;
        remove(): void;
    };
    const document = {
        head: {
            appendChild(link: Link) {
                events.push(`add ${link.getAttribute("href")}`);
                link.isConnected = true;
                links.push(link);
                return link;
            },
        },
        createElement() {
            const link: Link = {
                rel: "",
                attributes: new Map(),
                onerror: null,
                isConnected: false,
                setAttribute(name: string, value: string) {
                    this.attributes.set(name, value);
                },
                getAttribute(name: string) {
                    return this.attributes.get(name) ?? null;
                },
                remove() {
                    events.push(`remove ${this.getAttribute("href")}`);
                    this.isConnected = false;
                    const index = links.indexOf(link);
                    if (index >= 0) links.splice(index, 1);
                },
            };
            return link;
        },
        querySelectorAll(selector: string) {
            assert.equal(selector, "link[data-theme-plugin]");
            return links.filter((link) => link.isConnected && link.attributes.has("data-theme-plugin"));
        },
    };
    globalThis.document = document as unknown as Document;
    const theme = (stylesheet: string, version: string) => ({ stylesheet, version }); // 测试用的样式表描述。
    applyThemeStylesheet(null);
    assert.equal(links.length, 0);

    applyThemeStylesheet(theme("示例-主题/theme.css", "1"));
    assert.equal(links.length, 1);
    assert.equal(links[0].rel, "stylesheet");
    assert.equal(links[0].getAttribute("data-theme-plugin"), "");
    assert.equal(links[0].getAttribute("href"), "/plugins/示例-主题/theme.css?v=1");
    const first = links[0]; // 同一地址不应再插一张。
    applyThemeStylesheet(theme("示例-主题/theme.css", "1"));
    assert.equal(links.length, 1);
    assert.equal(links[0], first);

    events.length = 0;
    applyThemeStylesheet(theme("另一主题/theme.css", "2"));
    assert.deepEqual(events, ["add /plugins/另一主题/theme.css?v=2", "remove /plugins/示例-主题/theme.css?v=1"]);
    assert.equal(links.length, 1);
    assert.equal(links[0].getAttribute("href"), "/plugins/另一主题/theme.css?v=2");

    applyThemeStylesheet(null);
    assert.equal(links.length, 0);
    applyThemeStylesheet(null);
    assert.equal(links.length, 0);

    const notices: string[] = []; // 失败回退时交给现有通知的文案。
    applyThemeStylesheet(theme("坏主题/theme.css", "3"), (message) => notices.push(message));
    assert.equal(links.length, 1);
    links[0].onerror?.();
    assert.equal(links.length, 0);
    assert.deepEqual(notices, ["主题样式表加载失败，已回退到内置主题。"]);
    applyThemeStylesheet(theme("坏主题/theme.css", "3"), (message) => notices.push(message));
    assert.equal(links.length, 0);
    assert.equal(notices.length, 1);
    applyThemeStylesheet(null);
    applyThemeStylesheet(theme("坏主题/theme.css", "3"));
    assert.equal(links.length, 1);
});
