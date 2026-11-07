import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createServer } from "vite";

const server = await createServer({
    server: { middlewareMode: true },
    appType: "custom",
    optimizeDeps: { noDiscovery: true, include: [] }, // 不扫仓库里的 html，避免和开发服务抢依赖优化。
});
const page = await server.ssrLoadModule("/src/features/settings/PluginSettingsPage.tsx");
const pluginContract = await server.ssrLoadModule("/src/contracts/plugin.ts");
const scripts = await server.ssrLoadModule("/src/bridge/pluginScripts.ts");
test.after(async () => {
    await server.close();
});

test("插件行展示元素穿透整行按钮，卸载与开关保留独立命中", () => {
    const css = readFileSync(new URL("./PluginSettingsPage.css", import.meta.url), "utf8"); // 生产行命中样式。
    const rules = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body })); // 仅匹配声明块。
    const row = rules.find(({ selector }) => selector === ".plugin-settings__row--link > :not(.plugin-settings__hit)"); // 同层排列。
    const pass = rules.find(({ selector, body }) => /\.plugin-settings__row--link\s*>/.test(selector) && /pointer-events:\s*none/.test(body));
    assert.match(row?.body ?? "", /position:\s*relative/);
    assert.match(row?.body ?? "", /z-index:\s*1/);
    assert.match(pass?.selector ?? "", /\.plugin-settings__copy/);
    assert.match(pass?.selector ?? "", /\.plugin-settings__chevron/);
    assert.doesNotMatch(pass?.selector ?? "", /\.plugin-settings__uninstall|\.settings-switch/);
    assert.match(pass?.body ?? "", /pointer-events:\s*none/);
    assert.match(rules.find(({ selector }) => selector === ".plugin-settings__row--link > .plugin-settings__hit")?.body ?? "", /position:\s*absolute/);
});

test("设置条目上下文带上桌面实例与主动拉取入口", () => {
    const source = readFileSync(new URL("./PluginSettingsPage.tsx", import.meta.url), "utf8");
    assert.match(source, /const desktopInstanceId = components\?\.find\(\(item\) => item\.componentId === selected\.id\)\?\.instanceId/);
    assert.match(source, /instanceId=\{desktopInstanceId\}/);
    assert.match(source, /requestLoopback: instanceId \?/);
    assert.match(source, /requestLoopback\(instanceId, channelId, method, path\)/);
    assert.doesNotMatch(source, /peek/i);
});

test("失效移除与天气删除使用 danger，分段控件仍为 primary", () => {
    const plugin = readFileSync(new URL("./PluginSettingsPage.tsx", import.meta.url), "utf8"); // 插件移除命令。
    const weather = readFileSync(new URL("./WeatherSettings.tsx", import.meta.url), "utf8"); // 天气确认与供应商分段。
    assert.match(plugin, /className="plugin-settings__remove"[\s\S]*?data-button="danger"/);
    assert.match(weather, /className="weather-settings__danger"[\s\S]*?data-button="danger"/);
    assert.match(weather, /className=\{`weather-settings__segment[^}]+\}[\s\S]*?data-button="primary"/);
});

test("安装确认、目录草稿和任务历史", () => {
    assert.equal(page.installConfirmLabel(false), "覆盖安装");
    assert.equal(page.installConfirmLabel(true), "卸载并安装");
    assert.equal(page.pluginDirectoryShown(undefined, ""), "程序所在目录\\plugin");
    assert.equal(page.pluginDirectoryShown("D:\\plugins", ""), "D:\\plugins");
    assert.equal(page.pluginDirectoryDirty(undefined, ""), false);
    assert.equal(page.pluginDirectoryDirty("", "D:\\plugins"), true);
    assert.equal(page.pluginDirectoryDirty("", ""), false);
    const plan = pluginContract.parsePluginInstallPlan({
        planId: "install-1",
        name: "时钟",
        version: "1.0.0",
        targetFolder: "时钟",
        sourceName: "时钟.zip",
        conflict: { installedVersion: "1.2.0", incomingVersion: "1.0.0", downgrade: true },
    });
    assert.equal(plan?.conflict?.downgrade, true);
    assert.equal(page.installConflictIdentity("甲", "plugin.item"), "甲@plugin.item");
    assert.equal(page.installConflictIdentity(undefined, "plugin.item"), "@plugin.item");
    assert.equal(page.installVersionLine("1.0.0", "1.1.0"), "版本：1.0.0 -> 1.1.0");
    const named = pluginContract.parsePluginInstallPlan({
        planId: "install-2",
        name: "时钟",
        version: "1.0.0",
        targetFolder: "时钟",
        sourceName: "时钟.zip",
        author: "甲",
        pluginId: "plugin.item",
    });
    assert.equal(page.installConflictIdentity(named?.author, named?.pluginId), "甲@plugin.item");
    assert.equal(pluginContract.parsePluginInstallPlan(null), undefined);
    assert.throws(() => pluginContract.parsePluginInstallPlan({ planId: "" }));
    const tasks = page.rememberInstallTask(
        Array.from({ length: 20 }, (_, index) => ({ id: String(index), name: "旧", status: "done", stage: "done", percent: 100 })),
        { id: "new", name: "新", status: "running", stage: "copy", percent: 40 },
    );
    assert.equal(tasks.length, 20);
    assert.equal(tasks.at(-1)?.id, "new");
    assert.equal(page.installBoard(tasks).text, "正在安装 20/20");
    for (const reason of [
        "插件清单缺少 name",
        "插件要求的程序版本过高，需要 99.0.0，当前 0.3.2",
        "插件不支持当前程序版本，最高 0.0.1，当前 0.3.2",
        "插件名称无效：a/b",
        "插件入口脚本不存在",
        "../evil.js",
    ]) {
        assert.equal(scripts.commandErrorMessage(reason), reason);
    }
    assert.equal(scripts.commandErrorMessage(new Error("清单校验失败：缺版本")), "清单校验失败：缺版本");
    const batch = scripts.pluginInstallPlans({
        plans: [{ planId: "a", name: "甲", version: "1", targetFolder: "甲", sourceName: "甲" }],
        errors: ["插件要求的程序版本过高，需要 99.0.0，当前 0.3.2", "插件清单缺少 name"],
    });
    assert.equal(batch.plans.length, 1);
    assert.deepEqual(batch.errors, ["插件要求的程序版本过高，需要 99.0.0，当前 0.3.2", "插件清单缺少 name"]);
    assert.deepEqual(scripts.pluginInstallPlans({ plans: [], errors: [] }), { plans: [], errors: [] });
    assert.throws(() => scripts.pluginInstallPlans({ plans: [], errors: "不是数组" }));
    assert.equal(page.taskMenuPlacement({ top: 400, bottom: 700, height: 300 }, { top: 0, bottom: 500 }).flipUp, true);
    assert.equal(page.taskMenuPlacement({ top: 40, bottom: 200, height: 160 }, { top: 0, bottom: 800 }).flipUp, false);
    scripts.bindPluginApi("plugin.kept", { settingsEntries: [{ id: "e", name: "条目", render() {} }] });
    assert.equal(scripts.readPluginApi("plugin.kept")?.settingsEntries?.[0]?.name, "条目");
    scripts.bindPluginApi("plugin.kept", { settingsEntries: [{ id: "z", name: "后写", render() {} }] });
    assert.equal(scripts.readPluginApi("plugin.kept")?.settingsEntries?.[0]?.name, "条目");
    assert.equal(scripts.pluginScriptLoaded("plugin.kept"), false);
});

test("不可创建的插件不显示入口，停用时入口禁用", () => {
    assert.equal(page.showCreatePlugin(false), false);
    assert.equal(page.showCreatePlugin(true), true);
    assert.equal(page.createPluginBlocked(true), true);
    assert.equal(page.createPluginBlocked(false), false);
    const menu = page.pluginContextMenu(
        { creatable: true, maxTiles: 3 },
        { used: 1, pluginDisabled: false, onOpenSettings() {}, onOpenDirectory() {}, onCreate() {}, onToggle() {}, onUninstall() {} },
    );
    assert.deepEqual(
        menu.map((item) => item.label),
        ["打开插件设置", "打开插件目录", "添加到桌面", "停用插件", "卸载插件"],
    );
    assert.equal(menu[2].separatorBefore, true);
    assert.equal(menu[3].separatorBefore, true);
    assert.equal(menu[4].separatorBefore, true);
    assert.equal(menu[4].danger, true);
    assert.equal(menu[2].disabled, false);
    const full = page.pluginContextMenu(
        { creatable: true, maxTiles: 2 },
        { used: 2, pluginDisabled: false, onOpenSettings() {}, onOpenDirectory() {}, onCreate() {}, onToggle() {}, onUninstall() {} },
    );
    assert.equal(full[2].disabled, true);
    assert.equal(full[2].description, "已达上限（2 个）");
    let created = 0;
    const wired = page.pluginContextMenu(
        { creatable: true, maxTiles: 3 },
        {
            used: 1,
            pluginDisabled: false,
            onOpenSettings() {},
            onOpenDirectory() {},
            onCreate() {
                created += 1;
            },
            onToggle() {},
            onUninstall() {},
        },
    );
    wired[2].onSelect?.();
    assert.equal(created, 1);
    const hidden = page.pluginContextMenu(
        { creatable: false, maxTiles: 2 },
        { used: 0, pluginDisabled: false, onOpenSettings() {}, onOpenDirectory() {}, onCreate() {}, onToggle() {}, onUninstall() {} },
    );
    assert.equal(
        hidden.some((item) => item.label === "添加到桌面"),
        false,
    );
    assert.equal(page.addToDesktopHint(3, 1), "还可以添加 2 个格子");
    assert.equal(page.addToDesktopHint(2, 2), "已达上限（2 个）");
    assert.equal(page.addToDesktopHint(0, 4), "添加到桌面");
    assert.equal(page.pluginTileRoom(2, 2).full, true);
    assert.equal(page.pluginIdentity("甲", "plugin.item"), "甲@plugin.item");
    assert.equal(page.pluginIdentity(undefined, "plugin.item"), "plugin.item");
    assert.equal(page.installFailureMessage("清单无效"), "安装失败：清单无效");
    assert.equal(page.installNotice("演示插件·正常", page.installSuccessMessage()), "演示插件·正常\n安装成功");
    assert.equal(page.installNotice("演示插件·正常", page.installFailureMessage("清单无效")), "演示插件·正常\n安装失败：清单无效");
    assert.equal(page.pluginPowerLabel(true), "启动插件");
    assert.equal(page.pluginPowerLabel(false), "停用插件");
});

test("多类型插件的创建入口展开成类型列表，单类型仍是单个命令", () => {
    const tileTypes = [
        { key: "status", name: "连通状态" },
        { key: "messages", name: "消息数" },
    ];
    assert.equal(page.pluginTileTypesExpand(tileTypes), true);
    assert.equal(page.pluginTileTypesExpand(tileTypes.slice(0, 1)), false);
    assert.equal(page.pluginTileTypesExpand([]), false);
    const created: Array<string | undefined> = [];
    const menu = page.pluginContextMenu(
        { creatable: true, maxTiles: 2, tileTypes },
        {
            used: 0,
            pluginDisabled: false,
            onOpenSettings() {},
            onOpenDirectory() {},
            onCreate(tileType) {
                created.push(tileType);
            },
            onToggle() {},
            onUninstall() {},
        },
    );
    const add = menu.find((item) => item.label === "添加到桌面"); // 多类型时不直接创建。
    assert.equal(add?.onSelect, undefined);
    assert.deepEqual(
        add?.children?.map((child) => child.label),
        ["连通状态", "消息数"],
    );
    add?.children?.[1].onSelect?.();
    assert.deepEqual(created, ["messages"]);
    const single = page.pluginContextMenu(
        { creatable: true, maxTiles: 2, tileTypes: tileTypes.slice(0, 1) },
        {
            used: 0,
            pluginDisabled: false,
            onOpenSettings() {},
            onOpenDirectory() {},
            onCreate(tileType) {
                created.push(tileType);
            },
            onToggle() {},
            onUninstall() {},
        },
    );
    assert.equal(single.find((item) => item.label === "添加到桌面")?.children, undefined, "单类型不展开");
    single.find((item) => item.label === "添加到桌面")?.onSelect?.();
    assert.equal(created.at(-1), "status", "单类型直接创建也要带上类型");
    assert.equal(page.pluginSingleTileType([]), undefined);
    assert.equal(page.pluginSingleTileType(tileTypes), undefined, "多类型不走单类型直建");
    assert.equal(page.pluginSingleTileType(tileTypes.slice(0, 1)), "status");
    assert.deepEqual(page.createTileState("messages"), { tileType: "messages" });
    assert.deepEqual(page.createTileState(), {}, "没选类型时不写 state");
});

test("多类型插件的数量上限按类型分别计算", () => {
    const tileTypes = [
        { key: "status", name: "连通状态" },
        { key: "messages", name: "消息数" },
    ];
    const components = [
        { componentId: "plugin.multi", state: { tileType: "status" } }, // 连通状态已用一个。
        { componentId: "plugin.multi", state: {} }, // 旧实例没有类型，落到默认类型（第一节）。
        { componentId: "plugin.other", state: { tileType: "messages" } }, // 别的插件的同类不计入。
    ];
    const status = page.pluginTypeStatus(components, "plugin.multi", tileTypes, 2);
    assert.equal(status?.get("status")?.remaining, 0, "默认类型已满");
    assert.equal(status?.get("messages")?.remaining, 2, "另一类不受影响");
    assert.equal(page.pluginTightestRemaining(status), 0);
    assert.equal(page.pluginTypeStatus(components, "plugin.multi", [], 2), undefined, "隐式单类型不分类");
    assert.equal(page.addToDesktopHint(2, 3, 0), "已达上限（2 个）");
    assert.equal(page.addToDesktopHint(2, 3, 1), "还可以添加 1 个格子");
    const created: Array<string | undefined> = [];
    const menu = page.pluginContextMenu(
        { creatable: true, maxTiles: 2, tileTypes, componentId: "plugin.multi" },
        {
            used: 3,
            components,
            pluginDisabled: false,
            onOpenSettings() {},
            onOpenDirectory() {},
            onCreate(tileType) {
                created.push(tileType);
            },
            onToggle() {},
            onUninstall() {},
        },
    );
    const children = menu.find((item) => item.label === "添加到桌面")?.children ?? [];
    assert.deepEqual([children[0].disabled, children[1].disabled], [true, false], "只禁用已经满了的那一类");
    assert.equal(children[0].description, "已达上限（2 个）");
    children[1].onSelect?.();
    assert.deepEqual(created, ["messages"]);
    const single = page.pluginContextMenu(
        { creatable: true, maxTiles: 1, tileTypes: tileTypes.slice(0, 1), componentId: "plugin.multi" },
        {
            used: 1,
            components: [{ componentId: "plugin.multi", state: { tileType: "status" } }],
            pluginDisabled: false,
            onOpenSettings() {},
            onOpenDirectory() {},
            onCreate() {},
            onToggle() {},
            onUninstall() {},
        },
    );
    assert.equal(single.find((item) => item.label === "添加到桌面")?.disabled, true, "单类型按那一类判断");
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
            invoke(command) {
                if (command === "get_preferences") return { theme: "system", doubleClickToOpen: true, disabledPlugins: [] };
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
        assert.equal(page.entrySummary("plugin.empty-entries", false, true), "插件未提供设置条目");
        scripts.bindPluginApi("plugin.bad-shape", { renderTile() {}, settingsEntries: { id: "nope" } });
        assert.equal(page.entrySummary("plugin.bad-shape", false, true), "插件设置条目格式无效：不是数组");
        const exported = scripts.shellFromModule({ default: { renderTile() {}, settingsEntries: [{ id: "a", name: "甲", render() {} }] } });
        assert.equal(exported.ok && exported.value.settingsEntries[0].id, "a");
        assert.equal(exported.ok && exported.value.default, undefined);
        const named = scripts.shellFromModule({ default: { renderTile() {} }, settingsEntries: [{ id: "b", name: "乙", render() {} }] });
        assert.equal(named.ok && named.value.settingsEntries[0].id, "b");
        assert.equal(scripts.shellFromModule({ default: 1 }).message, "插件导出不是对象");
        assert.equal(scripts.shellFromModule(null).message, "插件模块没有默认导出");
        assert.equal(scripts.parsePluginInstallApplied("plugin.a").snapshot, undefined);
        const snapshot = {
            revision: 2,
            manifests: [{ id: "plugin.a", name: "甲", entry: "plugin:slot", script: "甲/main.js", requiredCapabilities: [], persistenceVersion: 1 }],
            instances: [],
            failures: [],
        };
        assert.equal(scripts.parsePluginInstallApplied({ pluginId: "plugin.a", snapshot }).snapshot.manifests[0].id, "plugin.a");
        assert.equal(page.installSuccessMessage(), "安装成功");
        assert.equal(page.entrySummary("plugin.missing-summary", false, false), "正在加载插件设置…");
        assert.equal(page.entrySummary("plugin.missing-summary", true, false), "已停用");
        scripts.pluginModules.load = async () => {
            throw new Error("插件脚本执行失败");
        };
        assert.equal(await scripts.loadPluginScript("plugin.fail-summary", "fail/main.js"), "插件模块加载失败：插件脚本执行失败");
        assert.equal(page.entrySummary("plugin.fail-summary", false, false), "设置条目加载失败");
        const themeCalls: string[] = []; // 没有入口脚本的主题不加载，带脚本的主题照常加载。
        page.requestPluginScript({ id: "plugin.theme", role: "theme", script: "" }, (id) => themeCalls.push(id));
        page.requestPluginScript({ id: "plugin.theme", role: "theme", script: "主题-示例/main.js" }, (id) => themeCalls.push(id), true);
        assert.deepEqual(themeCalls, ["plugin.theme"]);
        const tileCalls: string[] = []; // 空脚本不能靠“没路径”被放过。
        page.requestPluginScript({ id: "plugin.tile", role: "tile", script: "" }, (id) => tileCalls.push(id));
        page.requestPluginScript({ id: "plugin.tile", script: "" }, (id) => tileCalls.push(id));
        assert.deepEqual(tileCalls, ["plugin.tile", "plugin.tile"]);
        assert.equal(await scripts.loadPluginScript("plugin.empty-script", ""), "插件脚本路径无效");
    } finally {
        globalThis.window = previous;
    }
});

test("共享 token 元数据只在声明齐全时出现", () => {
    scripts.bindPluginApi("plugin.pairing", {
        renderTile() {},
        loopbackCredential: { target: " DesktopTools/astrbot-status ", headerName: "X-Status-Token", title: "共享 token", hint: "粘贴后保存" },
    });
    assert.deepEqual(page.pluginLoopbackCredential("plugin.pairing"), { target: "DesktopTools/astrbot-status", headerName: "X-Status-Token", title: "共享 token", hint: "粘贴后保存" });
    scripts.bindPluginApi("plugin.no-pairing", { renderTile() {} });
    assert.equal(page.pluginLoopbackCredential("plugin.no-pairing"), undefined);
    scripts.bindPluginApi("plugin.bad-pairing", { renderTile() {}, loopbackCredential: { target: "" } });
    assert.equal(page.pluginLoopbackCredential("plugin.bad-pairing"), undefined);
});
