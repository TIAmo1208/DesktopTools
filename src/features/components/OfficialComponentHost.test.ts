import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createServer } from "vite";

const server = await createServer({
    server: { middlewareMode: true },
    appType: "custom",
    plugins: [
        {
            name: "plugin-host-test-tables",
            transform(code, id) {
                if (id.endsWith("/features/components/OfficialComponentHost.tsx"))
                    return `${code}\nexport { pluginVisibility, instanceVisible, pluginRenderers, syncPluginScripts, mountPluginTile, releasePluginTile, syncPluginTileSettings };`;
            },
        },
    ],
}); // 不启动监听端口，沿用生产入口及依赖。
const host = await server.ssrLoadModule("/src/features/components/OfficialComponentHost.tsx");
const bridge = await server.ssrLoadModule("/src/bridge/pluginSettings.ts");
const scripts = await server.ssrLoadModule("/src/bridge/pluginScripts.ts");
test.after(async () => {
    await server.close();
});

const slotProps = (instanceId, componentId) => ({ instance: { instanceId, componentId }, registration: { entry: "plugin:slot" } });

test("插件宿主保留官方显隐、入口别名和未知入口兜底", async () => {
    const { pluginVisibility, instanceVisible, pluginRenderers } = host;
    for (const value of [undefined, false, true]) {
        for (const debugRegistered of [false, true]) {
            const context = {
                preferences: { clockEnabled: value, debugEnabled: value, musicEnabled: value, collectEnabled: value, weatherEnabled: value },
                runtime: { manifests: debugRegistered ? [{ id: "official.debug-test" }] : [] },
            }; // 每个开关与 Debug 注册是独立条件。
            for (const [id, expected] of [
                ["official.local-clock", value !== false],
                ["official.debug-test", value !== false && debugRegistered],
                ["official.music-control", value === true],
                ["official.collect-box", value !== false],
                ["official.weather", value === true],
            ])
                assert.equal(pluginVisibility.get(id)(context), expected);
            assert.equal(instanceVisible("plugin.sample", context), true);
        }
    }
    assert.equal(pluginVisibility.get("official.debug-test")({}), false);
    assert.equal(pluginVisibility.get("official.local-clock")({}), true);
    assert.equal(pluginVisibility.get("official.collect-box")({}), true);
    assert.equal(pluginVisibility.get("official.music-control")({}), false);
    assert.equal(pluginVisibility.get("official.weather")({}), false);

    const props = slotProps("sample", "plugin.waiting"); // 内容分派只传递原有实例契约。
    for (const name of ["local-clock", "debug-test", "music-control", "collect-box", "weather"]) {
        const render = pluginRenderers.get(`official:${name}`); // 官方清单使用冒号入口。
        assert.equal(render, pluginRenderers.get(`official.${name}`));
        const tile = render(props, { debugContent: "debug content" }); // 同一 renderer 接收原有 props。
        assert.equal(tile.props.instance, props.instance);
        assert.equal(tile.props.registration, props.registration);
        assert.equal(tile.key, "sample");
        if (name === "debug-test") assert.equal(tile.props.children, "debug content");
    }
    assert.equal(pluginRenderers.get("unknown:entry"), undefined);
    assert.equal(pluginRenderers.get("constructor"), undefined);
    const pluginTile = pluginRenderers.get("plugin:slot")(props, {}); // 插件只取得容器，没有新尺寸和命中策略。
    assert.equal(pluginTile.props.minimum, undefined);
    assert.equal(pluginTile.props["data-hit-priority"], undefined);
    assert.equal(typeof pluginTile.props.children, "function");
    const waiting = pluginTile.props.children(false); // 折叠状态交给作者，容器本身不带样式。
    assert.equal(waiting.props.instanceId, "sample");
    assert.equal(waiting.props.pluginId, "plugin.waiting");
    assert.equal(waiting.props.collapsed, false);

    const source = await readFile(new URL("./OfficialComponentHost.tsx", import.meta.url), "utf8"); // 保证入口优先级与原有兜底原样保留。
    assert.match(source, /props\.registration\?\.entry \?\? instance\.componentId/);
    assert.match(source, /title="本地时间" minimum=\{\{ width: 164, height: 90 \}\}/);
    assert.match(source, /<p className="official-component__error">组件前端入口不存在<\/p>/);
    assert.match(source, /<div ref=\{containerRef\} data-plugin-container=\{instanceId\} \/>/);
    assert.match(source, /releasePluginTile\(instanceId\)/);
    assert.match(source, /removeEventListener\("desktop-refresh-changed", handleRefresh\)/);
});

test("按插件 id 去重只导入一次模块，失败不重试并进入错误态", async () => {
    const imported = []; // 实际请求过的模块地址。
    const previous = globalThis.window;
    scripts.pluginModules.load = async (specifier) => {
        imported.push(specifier);
        if (specifier === "/plugins/a/main.js") throw new Error("模块加载失败");
    };
    globalThis.window = { __DESKTOP_TOOLS_NATIVE__: { invoke: () => ({ theme: "system", doubleClickToOpen: true, disabledPlugins: [] }) } };
    const manifests = [
        { id: "plugin.b", script: "b/main.js" },
        { id: "plugin.a", script: "a/main.js" },
        { id: "plugin.c", script: "c/main.js" },
        { id: "official.local-clock", script: "" },
    ];
    await host.syncPluginScripts(manifests, ["plugin.a", "plugin.a", "plugin.b", "official.local-clock"]);
    assert.deepEqual(imported, ["/plugins/a/main.js", "/plugins/b/main.js"]);
    await host.syncPluginScripts(manifests, ["plugin.a", "plugin.b"]);
    assert.equal(imported.length, 2);
    const failed = host.pluginRenderers.get("plugin:slot")(slotProps("broken-script", "plugin.a"), {});
    assert.equal(failed.props.children.props.className, "official-component__error");
    assert.equal(failed.props.children.props.children, "组件渲染失败");
    globalThis.window = previous;
});

test("缺 renderTile 与未知或重复注册不参与渲染", async () => {
    const warnings = [];
    const original = console.warn;
    console.warn = (...parts) => warnings.push(parts.map(String).join(" "));
    try {
        scripts.adoptPluginManifests([{ id: "plugin.sample" }]);
        const second = { renderTile() {} };
        scripts.pluginShell["plugin.sample"] = {};
        scripts.pluginShell["plugin.sample"] = second;
        assert.equal(scripts.pluginShell["plugin.sample"].renderTile, undefined);
        assert.notEqual(scripts.pluginShell["plugin.sample"], second);
        assert.equal(
            warnings.some((message) => message.includes("重复注册") && message.includes("plugin.sample")),
            true,
        );
        const broken = host.pluginRenderers.get("plugin:slot")(slotProps("missing-tile", "plugin.sample"), {});
        assert.equal(broken.props.children.type, "p");
        assert.equal(broken.props.children.props.className, "official-component__error");
        assert.equal(broken.props.children.props.children, "组件渲染失败");
    } finally {
        console.warn = original;
    }
});

test("实例移除时调用最近一次 renderTile 的清理函数", () => {
    scripts.adoptPluginManifests([{ id: "plugin.shared" }, { id: "plugin.redraw" }]);
    const cleaned = [];
    let seen;
    scripts.pluginShell["plugin.shared"] = {
        renderTile(_container, context) {
            seen = { instanceId: context.instanceId, collapsed: context.collapsed, settings: context.settings };
            return () => cleaned.push(context.instanceId);
        },
    };
    const context = (instanceId) => ({ pluginId: "plugin.shared", instanceId, collapsed: true, settings: { on: true }, saveSettings: async () => undefined });
    host.mountPluginTile({}, context("inst-1"));
    host.mountPluginTile({}, context("inst-2"));
    assert.equal(seen.instanceId, "inst-2");
    assert.equal(seen.collapsed, true);
    assert.deepEqual(seen.settings, { on: true });
    assert.deepEqual(cleaned, []);
    host.releasePluginTile("inst-1");
    assert.deepEqual(cleaned, ["inst-1"]);
    host.releasePluginTile("inst-1");
    assert.deepEqual(cleaned, ["inst-1"]);
    host.releasePluginTile("inst-2");
    assert.deepEqual(cleaned, ["inst-1", "inst-2"]);

    const order = [];
    let generation = 0;
    scripts.pluginShell["plugin.redraw"] = {
        renderTile() {
            const current = ++generation;
            return () => order.push(current);
        },
    };
    const redraw = { pluginId: "plugin.redraw", instanceId: "redraw-1", collapsed: false, settings: null, saveSettings: async () => undefined };
    host.mountPluginTile({}, redraw);
    host.mountPluginTile({}, redraw);
    assert.deepEqual(order, [1]);
    host.releasePluginTile("redraw-1");
    assert.deepEqual(order, [1, 2]);
});

test("逐屏设置事件走 dispatch，退订后不再投递", async () => {
    const calls = [];
    const errors = [];
    const native = {
        dispatch() {},
        listen() {
            throw new Error("逐屏不应使用 listen 订阅插件设置");
        },
    };
    const previous = globalThis.window;
    globalThis.window = { __DESKTOP_TOOLS_NATIVE__: native };
    try {
        const stop = await bridge.listenPluginSettings(
            (pluginId) => calls.push(pluginId),
            (error) => errors.push(error),
        );
        globalThis.window.__DESKTOP_TOOLS_NATIVE__.dispatch(bridge.PLUGIN_SETTINGS_CHANGED_EVENT, { pluginId: "plugin.clock" });
        globalThis.window.__DESKTOP_TOOLS_NATIVE__.dispatch(bridge.PLUGIN_SETTINGS_CHANGED_EVENT, {});
        assert.deepEqual(calls, ["plugin.clock"]);
        assert.equal(errors.length, 1);
        stop();
        globalThis.window.__DESKTOP_TOOLS_NATIVE__.dispatch(bridge.PLUGIN_SETTINGS_CHANGED_EVENT, { pluginId: "plugin.clock" });
        assert.deepEqual(calls, ["plugin.clock"]);
    } finally {
        globalThis.window = previous;
    }
    const source = await readFile(new URL("../../bridge/pluginSettings.ts", import.meta.url), "utf8");
    assert.match(source, /get_plugin_settings/);
    assert.match(source, /save_plugin_settings/);
    assert.match(source, /onSettings/);
    assert.match(source, /plugin-settings-changed/);
});

test("收到 plugin-settings-changed 后用新设置重绘，并先执行上一轮清理", async () => {
    const order = [];
    const seen = [];
    scripts.adoptPluginManifests([{ id: "plugin.greet" }]);
    scripts.pluginShell["plugin.greet"] = {
        renderTile(_container, context) {
            seen.push(context.settings);
            const generation = seen.length;
            return () => order.push(generation);
        },
    };
    const context = (settings) => ({ pluginId: "plugin.greet", instanceId: "tile-1", collapsed: false, settings, saveSettings: async () => undefined });
    host.mountPluginTile({}, context(null));
    const native = {
        invoke: async (command, args) => {
            assert.equal(command, "get_plugin_settings");
            assert.equal(args.pluginId, "plugin.greet");
            return { greeting: "TEST" };
        },
        dispatch() {},
        listen() {
            throw new Error("逐屏不应使用 listen 订阅插件设置");
        },
    };
    const previous = globalThis.window;
    globalThis.window = { __DESKTOP_TOOLS_NATIVE__: native };
    try {
        const stop = await bridge.listenPluginSettings(
            (pluginId) => {
                void host.syncPluginTileSettings(pluginId, (settings) => host.mountPluginTile({}, context(settings)));
            },
            () => {},
        );
        globalThis.window.__DESKTOP_TOOLS_NATIVE__.dispatch(bridge.PLUGIN_SETTINGS_CHANGED_EVENT, '{"pluginId":"plugin.greet"}');
        await new Promise((resolve) => setTimeout(resolve, 0));
        stop();
    } finally {
        globalThis.window = previous;
    }
    assert.deepEqual(seen, [null, { greeting: "TEST" }]);
    assert.deepEqual(order, [1]);
});

test("插件格子菜单最后一项是可按清单置灰的移除", async () => {
    const tile = await server.ssrLoadModule("/src/features/tiles/DesktopTileBase.tsx");
    const blocked = tile.tileRemoveMenuItem(false, () => undefined);
    assert.equal(blocked.label, "移除");
    assert.equal(blocked.separatorBefore, true);
    assert.equal(blocked.danger, true);
    assert.equal(blocked.disabled, true);
    assert.equal(blocked.description, "该组件不支持移除");
    const allowed = tile.tileRemoveMenuItem(true, () => undefined);
    assert.equal(allowed.disabled, false);
    assert.equal(allowed.description, undefined);
    const hostSource = await readFile(new URL("./OfficialComponentHost.tsx", import.meta.url), "utf8");
    assert.match(hostSource, /tileRemoveMenuItem/);
    assert.match(hostSource, /remove-component/);
    assert.match(hostSource, /plugin:slot/);
});

test("设置条目缺字段或重复时丢弃该条，不影响 renderTile", () => {
    scripts.adoptPluginManifests([{ id: "plugin.entries" }, { id: "plugin.bad-entries" }]);
    const renderTile = () => {};
    scripts.pluginShell["plugin.entries"] = {
        renderTile,
        settingsEntries: [
            { id: "a", name: "甲", render() {} },
            { name: "缺标识" },
            { id: "a", name: "重复" },
            { id: "b", name: "乙", render() {} },
            { id: "", name: "空" },
        ],
    };
    const api = scripts.pluginShell["plugin.entries"];
    assert.equal(api.renderTile, renderTile);
    assert.deepEqual(
        api.settingsEntries.map((entry) => entry.id),
        ["a", "b"],
    );
    scripts.pluginShell["plugin.bad-entries"] = { renderTile, settingsEntries: { id: "nope" } };
    assert.equal(scripts.pluginShell["plugin.bad-entries"].renderTile, renderTile);
    assert.equal(scripts.pluginShell["plugin.bad-entries"].settingsEntries, undefined);
});

test("设置窗口按插件 id 幂等加载，停用不加载，失败可查询", async () => {
    const calls = [];
    const imported = []; // 实际导入过的模块。
    let disabled = ["plugin.off"];
    const loaded = [];
    const previous = globalThis.window;
    scripts.pluginModules.load = async (specifier) => {
        imported.push(specifier);
        if (specifier === "/plugins/fail/main.js") throw new Error("插件脚本执行失败");
        return { default: { renderTile() {} } }; // 默认导出，标识由宿主绑定。
    };
    globalThis.window = {
        __DESKTOP_TOOLS_NATIVE__: {
            invoke(command, payload) {
                calls.push([command, payload?.pluginId]);
                if (command === "get_preferences") return { theme: "system", doubleClickToOpen: true, disabledPlugins: disabled };
                if (command === "uninstall_plugin") return { removed: ["plugin.ok.json", "plugin.ok.js"] };
                if (command === "open_plugin_directory") return null;
                throw new Error(command);
            },
        },
    };
    const stop = scripts.listenPluginScripts((pluginId) => loaded.push(pluginId));
    try {
        await scripts.loadPluginScript("plugin.off", "off/main.js");
        assert.equal(imported.length, 0);
        disabled = [];
        await scripts.loadPluginScript("plugin.off", "off/main.js");
        assert.deepEqual(imported, ["/plugins/off/main.js"]);
        await scripts.loadPluginScript("plugin.off", "off/main.js");
        assert.deepEqual(imported, ["/plugins/off/main.js"]);
        assert.deepEqual(loaded, ["plugin.off"]);
        assert.equal(typeof scripts.readPluginApi("plugin.off")?.renderTile, "function");
        const failure = await scripts.loadPluginScript("plugin.fail", "fail/main.js");
        assert.equal(failure, "插件模块加载失败：插件脚本执行失败");
        assert.equal(scripts.pluginScriptFailure("plugin.fail"), "插件模块加载失败：插件脚本执行失败");
        const before = imported.length;
        assert.equal(await scripts.loadPluginScript("plugin.fail", "fail/main.js"), "插件模块加载失败：插件脚本执行失败");
        assert.equal(imported.length, before);
        scripts.pluginModules.load = async (specifier) => {
            imported.push(specifier);
            return specifier.endsWith("shape/main.js")
                ? { default: 1 }
                : {
                      renderTile() {
                          return "ns";
                      },
                  };
        };
        assert.equal(await scripts.loadPluginScript("plugin.shape", "shape/main.js"), "插件导出不是对象");
        assert.equal(await scripts.loadPluginScript("plugin.ns", "ns/main.js"), undefined);
        assert.equal(scripts.readPluginApi("plugin.ns").renderTile(), "ns");
        assert.deepEqual(await scripts.uninstallPlugin("plugin.ok"), ["plugin.ok.json", "plugin.ok.js"]);
        await scripts.openPluginDirectory("plugin.ok");
        assert.equal(
            calls.some((call) => call[0] === "open_plugin_directory" && call[1] === "plugin.ok"),
            true,
        );
    } finally {
        stop();
        globalThis.window = previous;
    }
});

test("停用插件不进入可见列表，启用后恢复作者内容", async () => {
    const off = { preferences: { disabledPlugins: ["plugin.off"] } }; // 名单里的插件。
    const on = { preferences: { disabledPlugins: [] } }; // 重新启用。
    assert.equal(host.instanceVisible("plugin.off", off), false);
    assert.equal(host.instanceVisible("plugin.other", off), true);
    assert.equal(host.instanceVisible("plugin.off", on), true);
    const restored = host.pluginRenderers.get("plugin:slot")(slotProps("back-1", "plugin.restore"), on);
    assert.equal(typeof restored.props.children, "function");
    const broken = host.pluginRenderers.get("plugin:slot")(slotProps("still-broken", "plugin.sample"), on);
    const brokenBody = typeof broken.props.children === "function" ? broken.props.children(false) : broken.props.children;
    assert.equal(brokenBody.props.children, "组件渲染失败");
    const source = await readFile(new URL("./OfficialComponentHost.tsx", import.meta.url), "utf8");
    assert.equal(source.includes("插件已停用"), false);
    assert.equal(source.includes("component-runtime-failure"), false);
});

test("运行时失败只通知一次，命令失败同样通知", () => {
    const seen: string[] = [];
    const notify = (content: string) => seen.push(content);
    const item = { componentId: "plugin.item2", message: "插件清单身份重复" };
    host.reportRuntimeFailures(undefined, [item], notify);
    host.reportRuntimeFailures(undefined, [item], notify);
    host.reportRuntimeFailures("读取组件快照失败", undefined, notify);
    assert.deepEqual(seen, ["plugin.item2：插件清单身份重复", "读取组件快照失败"]);
});
