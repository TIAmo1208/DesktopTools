import assert from "node:assert/strict";
import test from "node:test";

test("主题字段缺省为 null，基线完整校验且不检查插件目录", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 无主题的当前缺省配置。
    assert.equal(defaults.themePluginId, null);
    assert.equal(defaults.themeBaseline, null);
    const baseline = { accentColor: "#AABBCC", desktopAccentColor: "#123456", cornerRadius: 0, islandRadius: 12, fontFamily: "Arial", fontSize: 18 };
    const selected = parsePreferences({ ...defaults, themePluginId: "plugin.not-installed", themeBaseline: baseline });
    assert.deepEqual(selected.themeBaseline, baseline);
    assert.equal(selected.themePluginId, "plugin.not-installed");
    for (const themePluginId of [1, false, "", " "]) assert.throws(() => parsePreferences({ ...defaults, themePluginId }));
    for (const patch of [
        { accentColor: "red" },
        { desktopAccentColor: "#12345g" },
        { cornerRadius: 13 },
        { cornerRadius: -1 },
        { islandRadius: 17 },
        { islandRadius: 1.5 },
        { fontFamily: "Arial; color:red" },
        { fontSize: 11 },
        { fontSize: 19 },
        { fontSize: 14.5 },
    ])
        assert.throws(() => parsePreferences({ ...defaults, themeBaseline: { ...baseline, ...patch } }));
    for (const themeBaseline of [{}, [], "bad", 1]) assert.throws(() => parsePreferences({ ...defaults, themeBaseline }));
});

test("岛偏好默认值、整份保存往返与 Rust 一致的边界", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 从读取到整份提交使用同一个解析结果。
    assert.deepEqual(
        [
            defaults.islandEnabled,
            defaults.islandForm,
            defaults.islandEdge,
            defaults.islandOffset,
            defaults.islandHideOnFullscreen,
            defaults.islandCardOrder,
            defaults.islandWeatherSource,
            defaults.islandWeatherCityId,
            defaults.islandRadius,
        ],
        [false, "capsule", "", 0, true, [], "", "", 12],
    );
    const selected = parsePreferences({
        ...defaults,
        islandEnabled: true,
        islandForm: "notch",
        islandEdge: "left",
        islandOffset: -12.5,
        islandHideOnFullscreen: false,
        islandCardOrder: ["official.music", "plugin.clock"],
        islandWeatherSource: "qweather",
        islandWeatherCityId: "101010100",
        islandRadius: 16,
    });
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify({ ...selected, fontSize: 14 }))), { ...selected, fontSize: 14 });
    // 上浮岛显示：缺字段为空映射，缺键按各自默认值读取；显式布尔往返，未知键与非布尔值拒绝。旧字段不迁移。
    // 收起记忆与默认收起：都按逐卡映射解析，缺字段为空表（记忆缺失视为开启、默认收起缺失视为不收起，t98）。
    assert.deepEqual(defaults.islandCardCollapseMemory, {});
    assert.deepEqual(defaults.islandCardDefaultCollapsed, {});
    const collapsed = parsePreferences({ ...defaults, islandCardDefaultCollapsed: { "official.collect-box": true }, islandCardCollapseMemory: { "official.music-control": false } });
    assert.equal(collapsed.islandCardDefaultCollapsed["official.collect-box"], true);
    assert.equal(collapsed.islandCardCollapseMemory["official.music-control"], false);
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify(collapsed))).islandCardDefaultCollapsed, collapsed.islandCardDefaultCollapsed);
    // 该映射按「保留合法布尔项」解析：非法类型/未知键被丢弃，不是抛错（与 collapseMemory 的既有口径一致）。
    assert.deepEqual(parsePreferences({ ...defaults, islandCardDefaultCollapsed: null }).islandCardDefaultCollapsed, {});
    assert.deepEqual(parsePreferences({ ...defaults, islandCardDefaultCollapsed: { "official.collect-box": "yes", unknown: true } }).islandCardDefaultCollapsed, {});
    assert.deepEqual(defaults.islandAnchorVisible, {});
    assert.deepEqual(defaults.islandPluginAnchorVisible, {}); // 逐插件卡映射缺省为空，全部显示。
    assert.equal(ISLAND_ANCHOR_MUSIC_VISIBLE_DEFAULT, true);
    assert.equal(ISLAND_ANCHOR_MESSAGE_VISIBLE_DEFAULT, true);
    for (const key of ["music", "message"] as const) assert.equal(islandAnchorVisible(defaults, key), true);
    assert.equal(islandAnchorVisible(undefined, "music"), ISLAND_ANCHOR_MUSIC_VISIBLE_DEFAULT); // 偏好尚未加载时也不抛。
    const anchors = parsePreferences({ ...defaults, islandAnchorVisible: { music: false, message: true } });
    assert.deepEqual(anchors.islandAnchorVisible, { music: false, message: true });
    assert.equal(islandAnchorVisible(anchors, "music"), false);
    assert.equal(islandAnchorVisible(anchors, "message"), true);
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify(anchors))).islandAnchorVisible, anchors.islandAnchorVisible);
    assert.deepEqual(parsePreferences({ ...defaults, islandAnchorVisible: {} }).islandAnchorVisible, {});
    const legacyAnchor = parsePreferences({ ...defaults, islandCapsuleMessages: false }); // 旧开关不迁移，也不再作为字段存在。
    assert.equal("islandCapsuleMessages" in legacyAnchor, false);
    assert.deepEqual(legacyAnchor.islandAnchorVisible, {});
    assert.equal(islandAnchorVisible(legacyAnchor, "message"), true);
    // 旧配置里的全局 plugin 键仍可解析并原样保留，但没有任何读取方再按它生效。
    const legacyPluginKey = parsePreferences({ ...defaults, islandAnchorVisible: { plugin: false } });
    assert.deepEqual(legacyPluginKey.islandAnchorVisible, { plugin: false });
    assert.equal(islandPluginAnchorVisible(legacyPluginKey, "plugin.demo"), true); // 旧全局键不再作为生效来源。
    for (const islandAnchorVisible of [null, 0, 1, "true", [], { music: "true" }, { music: 1 }, { weather: true }, { music: true, extra: false }])
        assert.throws(() => parsePreferences({ ...defaults, islandAnchorVisible }));
    for (const islandEdge of ["", "top", "bottom", "left", "right"]) assert.equal(parsePreferences({ ...defaults, islandEdge }).islandEdge, islandEdge);
    for (const islandForm of ["capsule", "notch"]) assert.equal(parsePreferences({ ...defaults, islandForm }).islandForm, islandForm);
    for (const islandRadius of [0, 12, 16]) assert.equal(parsePreferences({ ...defaults, islandRadius }).islandRadius, islandRadius);
    for (const patch of [
        { islandEnabled: null },
        { islandHideOnFullscreen: 1 },
        { islandForm: "Notch" },
        { islandForm: null },
        { islandEdge: "Top" },
        { islandEdge: null },
        { islandOffset: NaN },
        { islandOffset: Infinity },
        { islandOffset: "0" },
        { islandRadius: -1 },
        { islandRadius: 17 },
        { islandRadius: 1.5 },
        { islandRadius: null },
        { islandCardOrder: null },
        { islandCardOrder: [1] },
        { islandWeatherSource: "owm" },
        { islandWeatherSource: null },
        { islandWeatherCityId: 101010100 },
        { islandWeatherCityId: "line\nbreak" },
        { islandWeatherCityId: "city" },
    ])
        assert.throws(() => parsePreferences({ ...defaults, ...patch }));
    const cards = ["official.music", "official.music", "plugin.", "other.card", ...Array.from({ length: 40 }, (_, index) => `plugin.card${index}`)]; // 同 Rust：丢弃非法标识、去重、截断。
    assert.deepEqual(parsePreferences({ ...defaults, islandCardOrder: cards }).islandCardOrder, [
        "official.music",
        ...Array.from({ length: 31 }, (_, index) => `plugin.card${index}`),
    ]);
});

test("图片预览宽度使用默认值并验证整数边界", () => {
    const base = { theme: "light", doubleClickToOpen: true };
    assert.equal(parsePreferences(base).collectImagePreviewWidth, 78);
    for (const width of [48, 78, 156]) assert.equal(parsePreferences({ ...base, collectImagePreviewWidth: width }).collectImagePreviewWidth, width);
    for (const width of [47, 157, 78.5, null, "78", NaN]) assert.throws(() => parsePreferences({ ...base, collectImagePreviewWidth: width }));
});

import {
    insertIslandMessageCard,
    ISLAND_MESSAGE_CARD_ID,
    ISLAND_ANCHOR_MESSAGE_VISIBLE_DEFAULT,
    islandMessageCardEnabled,
    islandMessageCardInitialization,
    ISLAND_ANCHOR_MUSIC_VISIBLE_DEFAULT,
    islandAnchorVisible,
    islandPluginAnchorVisible,
    isPluginComponentId,
    ISLAND_PLUGIN_ANCHOR_LIMIT,
    parsePreferences,
    parseSettingsSnapshot,
    resolveAnimationsEnabled,
} from "./settings.ts";

test("天气显示偏好默认关闭，往返保留并拒绝非布尔值", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 缺失字段采用当前默认值。
    assert.equal(defaults.weatherEnabled, false);
    for (const weatherEnabled of [true, false])
        assert.equal(parsePreferences(JSON.parse(JSON.stringify({ ...defaults, weatherEnabled }))).weatherEnabled, weatherEnabled);
    for (const weatherEnabled of [null, 1, "true"]) assert.throws(() => parsePreferences({ ...defaults, weatherEnabled }));
});

test("刷新策略默认关闭，保存往返保留开关并拒绝错误类型", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 当前缺省策略。
    for (const key of ["pauseOnFullscreen", "pauseOnFocus"] as const) {
        assert.equal(defaults[key], false);
        assert.equal(parsePreferences(JSON.parse(JSON.stringify({ ...defaults, [key]: true })))[key], true);
        for (const value of [null, 1, "true"]) assert.throws(() => parsePreferences({ ...defaults, [key]: value }));
    }
});

test("界面动画使用默认值并严格校验布尔设置", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 缺失字段使用当前默认值。
    assert.equal(defaults.followSystemReducedMotion, true);
    assert.equal(defaults.interfaceAnimations, true);
    const disabled = parsePreferences({ ...defaults, followSystemReducedMotion: false, interfaceAnimations: false }); // 两个开关必须独立保存。
    assert.equal(parsePreferences(JSON.parse(JSON.stringify(disabled))).followSystemReducedMotion, false);
    assert.equal(parsePreferences(JSON.parse(JSON.stringify(disabled))).interfaceAnimations, false);
    for (const key of ["followSystemReducedMotion", "interfaceAnimations"] as const)
        for (const value of [null, 0, "false"]) assert.throws(() => parsePreferences({ ...defaults, [key]: value }));
    assert.equal(resolveAnimationsEnabled({ ...defaults, interfaceAnimations: false }, false), true); // 跟随系统时保留但忽略手动值。
    assert.equal(resolveAnimationsEnabled(defaults, true), false);
    assert.equal(resolveAnimationsEnabled({ ...defaults, followSystemReducedMotion: false, interfaceAnimations: false }, false), false);
    assert.equal(resolveAnimationsEnabled({ ...defaults, followSystemReducedMotion: false, interfaceAnimations: true }, true), true);
});

test("插件目录缺省跟随配置目录，保存往返保留且拒绝错误类型与纯空白", () => {
    const base = { theme: "light", doubleClickToOpen: true }; // 当前最小偏好载荷。
    assert.equal(parsePreferences(base).pluginDirectory, "");
    for (const pluginDirectory of ["", "D:\\DesktopToolsPlugins", "%APPDATA%\\TIAmo\\plugin"]) {
        const preferences = parsePreferences({ ...base, pluginDirectory }); // 已保存的自定义路径不可因前端解析丢失。
        assert.equal(preferences.pluginDirectory, pluginDirectory);
        assert.equal(parsePreferences(JSON.parse(JSON.stringify(preferences))).pluginDirectory, pluginDirectory);
    }
    for (const pluginDirectory of [null, false, 1, " ", "\t"]) assert.throws(() => parsePreferences({ ...base, pluginDirectory }));
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

test("停用插件缺失按空列表，非数组和非字符串条目丢弃", () => {
    const base = { theme: "system", doubleClickToOpen: true }; // 旧配置没有停用名单。
    assert.deepEqual(parsePreferences(base).disabledPlugins, []);
    assert.deepEqual(parsePreferences({ ...base, disabledPlugins: "plugin.clock" }).disabledPlugins, []);
    assert.deepEqual(
        parsePreferences({ ...base, disabledPlugins: ["plugin.clock", "official.weather", "plugin.clock", "plugin.", "plugin.b", 1, null] }).disabledPlugins,
        ["plugin.clock", "plugin.b"],
    );
    const overflow = Array.from({ length: 70 }, (_item, index) => `plugin.n${index}`); // 超过后端上限的合法标识。
    assert.equal(parsePreferences({ ...base, disabledPlugins: overflow }).disabledPlugins.length, 64);
    assert.equal(parsePreferences({ ...base, disabledPlugins: overflow }).disabledPlugins[0], "plugin.n0");
});

test("音乐显示歌词默认开启并严格校验布尔设置", () => {
    const base = { theme: "system", doubleClickToOpen: true }; // 旧配置没有歌词开关。
    assert.equal(parsePreferences(base).musicLyricsEnabled, true);
    assert.equal(parsePreferences({ ...base, musicLyricsEnabled: false }).musicLyricsEnabled, false);
    for (const musicLyricsEnabled of [null, 1, "false"]) assert.throws(() => parsePreferences({ ...base, musicLyricsEnabled }));
});

test("逐插件卡锚点：缺省显示、按卡关闭、未知键保留但不生效、非法键与超限拒绝", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true });
    assert.equal(islandPluginAnchorVisible(undefined, "plugin.demo"), true); // 偏好未加载也按显示。
    assert.equal(islandPluginAnchorVisible(defaults, "plugin.demo"), true); // 缺键显示。
    const selected = parsePreferences({ ...defaults, islandPluginAnchorVisible: { "plugin.demo": false, "plugin.not-installed": true } });
    assert.equal(islandPluginAnchorVisible(selected, "plugin.demo"), false); // 按卡关闭。
    assert.equal(islandPluginAnchorVisible(selected, "plugin.other"), true); // 映射里没有的卡按缺省。
    assert.equal(islandPluginAnchorVisible(selected, "plugin.not-installed"), true); // 未知键保留但渲染侧忽略。
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify(selected))).islandPluginAnchorVisible, {
        "plugin.demo": false,
        "plugin.not-installed": true,
    }); // 保存往返不丢映射。
    assert.equal(isPluginComponentId("plugin.dsh-status"), true);
    for (const key of ["dsh-status", "official.clock", "plugin.", "plugin.DSH", "plugin.a b", `plugin.${"x".repeat(130)}`])
        assert.equal(isPluginComponentId(key), false);
    for (const islandPluginAnchorVisible of [null, 0, 1, "true", [], { "plugin.demo": "false" }, { "plugin.demo": 1 }, { "official.clock": true }, { "plugin.": true }, { "plugin.DSH": true }])
        assert.throws(() => parsePreferences({ ...defaults, islandPluginAnchorVisible }));
    const full = Object.fromEntries(Array.from({ length: ISLAND_PLUGIN_ANCHOR_LIMIT }, (_, index) => [`plugin.card-${index}`, true]));
    assert.equal(Object.keys(parsePreferences({ ...defaults, islandPluginAnchorVisible: full }).islandPluginAnchorVisible).length, ISLAND_PLUGIN_ANCHOR_LIMIT);
    assert.throws(() => parsePreferences({ ...defaults, islandPluginAnchorVisible: { ...full, "plugin.one-more": true } }));
});

test("消息通知卡偏好：缺省显示、初始化未完成，保存往返不丢", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true });
    assert.equal(defaults.islandMessageCardVisible, true);
    assert.equal(defaults.islandMessageCardInitialized, false);
    assert.equal(islandMessageCardEnabled(defaults), true);
    const saved = parsePreferences({ ...defaults, islandMessageCardVisible: false, islandMessageCardInitialized: true });
    assert.equal(saved.islandMessageCardVisible, false);
    assert.equal(saved.islandMessageCardInitialized, true);
    assert.equal(islandMessageCardEnabled(saved), false);
    assert.deepEqual(parsePreferences(JSON.parse(JSON.stringify(saved))), saved);
    for (const islandMessageCardVisible of ["true", 1, null])
        assert.throws(() => parsePreferences({ ...defaults, islandMessageCardVisible }));
    for (const islandMessageCardInitialized of ["false", 0, null])
        assert.throws(() => parsePreferences({ ...defaults, islandMessageCardInitialized }));
});

test("消息通知卡默认插入位置：天气后、无天气时首个插件前、无插件则末尾，只插一次", () => {
    assert.deepEqual(insertIslandMessageCard(["official.music-control", "official.weather", "plugin.clock"]), [
        "official.music-control",
        "official.weather",
        ISLAND_MESSAGE_CARD_ID,
        "plugin.clock",
    ]); // 天气之后。
    assert.deepEqual(insertIslandMessageCard(["official.weather", "official.collect-box"]), [
        "official.weather",
        ISLAND_MESSAGE_CARD_ID,
        "official.collect-box",
    ]); // 无插件时紧跟天气。
    assert.deepEqual(insertIslandMessageCard(["official.music-control", "plugin.clock"]), [
        "official.music-control",
        ISLAND_MESSAGE_CARD_ID,
        "plugin.clock",
    ]); // 无天气时首个插件之前。
    assert.deepEqual(insertIslandMessageCard(["official.music-control"]), ["official.music-control", ISLAND_MESSAGE_CARD_ID]); // 无插件末尾。
    assert.deepEqual(insertIslandMessageCard([]), [ISLAND_MESSAGE_CARD_ID]);
    assert.deepEqual(insertIslandMessageCard(["official.weather", ISLAND_MESSAGE_CARD_ID]), ["official.weather", ISLAND_MESSAGE_CARD_ID]); // 已有不重复插。
    assert.equal(islandMessageCardInitialization(undefined), undefined);
    assert.equal(islandMessageCardInitialization({ islandCardOrder: [], islandMessageCardInitialized: true }), undefined); // 已初始化不再插。
    assert.deepEqual(islandMessageCardInitialization({ islandCardOrder: ["official.weather"], islandMessageCardInitialized: false }), {
        islandCardOrder: ["official.weather", ISLAND_MESSAGE_CARD_ID],
        islandMessageCardInitialized: true,
    }); // 只写一次补丁。
    const full = Array.from({ length: 32 }, (_, index) => `plugin.card-${index}`);
    assert.deepEqual(insertIslandMessageCard(full), full); // 满额不插，不挤掉既有卡。
    assert.equal(islandMessageCardInitialization({ islandCardOrder: full, islandMessageCardInitialized: false }), undefined); // 满额不置初始化标记，腾出位置后重试。
    const fullWithSlot = [...full.slice(0, 31), ISLAND_MESSAGE_CARD_ID];
    assert.deepEqual(islandMessageCardInitialization({ islandCardOrder: fullWithSlot, islandMessageCardInitialized: false }), {
        islandCardOrder: fullWithSlot,
        islandMessageCardInitialized: true,
    }); // 已存在的槽位不因满额丢失。
});
