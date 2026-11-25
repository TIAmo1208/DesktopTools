import assert from "node:assert/strict";
import test from "node:test";

import { ISLAND_PLUGIN_MESSAGE_LIMITS, parseIslandPluginMessage, type IslandPluginMessageKind } from "../../contracts/island.ts";
import {
    ISLAND_MESSAGE_TIMING,
    islandDismissNotification,
    islandLocalNotificationArrival,
    islandMergeNotifications,
    islandNotificationDeadline,
    islandNotificationTone,
} from "./islandMessages.ts";

/** 一条插件消息的本地条目；与岛页面收到的载荷同一形状。 */
const pluginNotice = (source: string, key: string, kind: IslandPluginMessageKind, text: string, receivedAtMs: number) =>
    islandLocalNotificationArrival(1, { source: "plugin", pluginSource: source, pluginKey: key, pluginKind: kind, content: text }, receivedAtMs);

test("插件消息载荷严格校验：来源、控制字符与长度", () => {
    assert.deepEqual(parseIslandPluginMessage({ source: "plugin.demo", key: "pending:p1", kind: "waiting", text: "待批准\n重构", unixMillis: 1000 }), {
        source: "plugin.demo",
        key: "pending:p1",
        kind: "waiting",
        text: "待批准 重构", // 控制字符与换行折成单个空格。
        unixMillis: 1000,
    });
    for (const invalid of [
        { key: "k", kind: "info", text: "x", unixMillis: 1 }, // 缺来源。
        { source: "", key: "k", kind: "info", text: "x", unixMillis: 1 },
        { source: "plugin.demo", key: "", kind: "info", text: "x", unixMillis: 1 },
        { source: "plugin.demo", key: "\u0000", kind: "info", text: "x", unixMillis: 1 },
        { source: "plugin.demo", key: "k", kind: "unknown", text: "x", unixMillis: 1 },
        { source: "plugin.demo", key: "k", kind: "info", text: " \n\u0007 ", unixMillis: 1 }, // 剥完控制字符为空。
        { source: "plugin.demo", key: "k", kind: "info", text: "x", unixMillis: -1 },
        { source: "plugin.demo", key: "k", kind: "info", text: "x" },
        { source: "plugin.demo", key: "k", kind: "info", text: "x", unixMillis: 1, extra: true },
        null,
    ])
        assert.throws(() => parseIslandPluginMessage(invalid));
    // 上限以 Unicode 字符计，emoji 不被代理对长度误判；超限拒收而不是截断。
    const emoji = "😀".repeat(ISLAND_PLUGIN_MESSAGE_LIMITS.text);
    assert.equal(parseIslandPluginMessage({ source: "plugin.demo", key: "k", kind: "info", text: emoji, unixMillis: 1 }).text, emoji);
    assert.throws(() =>
        parseIslandPluginMessage({ source: "plugin.demo", key: "k", kind: "info", text: `${emoji}😀`, unixMillis: 1 }),
    );
    assert.throws(() => parseIslandPluginMessage({ source: "plugin.demo", key: "k".repeat(ISLAND_PLUGIN_MESSAGE_LIMITS.key + 1), kind: "info", text: "x", unixMillis: 1 }));
});

test("插件条目沿用本地条目的身份与语气，不占用原生身份", () => {
    const waiting = pluginNotice("plugin.demo", "pending:p1", "waiting", "待批准：重构登录", 500);
    assert.equal(waiting.key, "plugin:plugin.demo:pending:p1"); // 来源 + key 构成稳定身份。
    assert.equal(waiting.nativeId, undefined); // 关闭只影响浮岛，不退原生队列。
    assert.equal(waiting.status, "awaiting");
    assert.equal(islandNotificationTone(waiting), "warning");

    for (const [kind, status, type] of [
        ["info", "pending", "info"],
        ["done", "completed", "success"],
        ["error", "failed", "error"],
    ] as const) {
        const notice = pluginNotice("plugin.demo", `k-${kind}`, kind, kind, 1);
        assert.equal(notice.status, status);
        assert.equal(notice.type, type);
    }
});

test("同一来源同一 key 去重到最新一条，跨插件同 key 不冲突并按到达时间排序", () => {
    const first = pluginNotice("plugin.dsh", "pending:p1", "waiting", "待批准：重构登录", 100);
    const second = pluginNotice("plugin.dsh", "pending:p1", "done", "已处理：重构登录", 300);
    const sameKeyOtherPlugin = pluginNotice("plugin.other", "pending:p1", "info", "另一个插件的提醒", 200);
    const merged = islandMergeNotifications([first], [second, sameKeyOtherPlugin]);
    assert.equal(merged.length, 2); // 同一来源同一 key 合并；不同来源同名 key 各自保留。
    assert.deepEqual(
        merged.map((notice) => [notice.key, notice.content, notice.receivedAtMs]),
        [
            ["plugin:plugin.dsh:pending:p1", "已处理：重构登录", 300],
            ["plugin:plugin.other:pending:p1", "另一个插件的提醒", 200],
        ],
    );
    assert.equal(merged[0].status, "completed"); // 后到的语气覆盖旧条目。

    const weather = islandLocalNotificationArrival(2, { source: "weather", content: "晴" }, 250);
    assert.equal(islandMergeNotifications([first, weather], [second]).length, 2); // 本地天气条目不被插件 key 影响。
});

test("暂停期间不接收新提醒", async () => {
    const { islandPluginMessageDeliverable } = await import("./islandPluginCard.ts");
    assert.equal(islandPluginMessageDeliverable(false), true);
    assert.equal(islandPluginMessageDeliverable(true), false); // 锁屏、浮岛隐藏、全屏隐藏时不投递。
});

test("关闭与过期沿用本地条目规则", () => {
    const waiting = pluginNotice("plugin.demo", "pending:p1", "waiting", "待批准", 1_000);
    const done = pluginNotice("plugin.demo", "pending:p2", "done", "已完成", 1_000);
    assert.equal(islandNotificationDeadline(waiting, ISLAND_MESSAGE_TIMING), undefined); // 待确认只豁免锚点期。
    assert.equal(islandNotificationDeadline(done, ISLAND_MESSAGE_TIMING), 1_000 + Math.max(ISLAND_MESSAGE_TIMING.contentMs, ISLAND_MESSAGE_TIMING.anchorMs));

    const remaining = islandDismissNotification([waiting, done], waiting.key);
    assert.deepEqual(remaining.map((notice) => notice.key), [done.key]); // 关闭只撤下这一条。
});

test("插件消息可选 uri：带 uri 的条目可导航，缺省保持只读", () => {
    const withUri = parseIslandPluginMessage({ source: "plugin.dsh-status", key: "p1", kind: "waiting", text: "待批准", unixMillis: 1, uri: "dsh://open" });
    assert.equal(withUri.uri, "dsh://open");
    const without = parseIslandPluginMessage({ source: "plugin.dsh-status", key: "p2", kind: "info", text: "普通提醒", unixMillis: 1 });
    assert.equal(without.uri, undefined);
    for (const bad of ["", "   ", 1, "x".repeat(513), "file:///C:/secret", "http://example.com", "dsh:/\u0085bad"])
        assert.throws(() => parseIslandPluginMessage({ source: "plugin.dsh-status", key: "p3", kind: "info", text: "x", unixMillis: 1, uri: bad }));
    const notice = islandLocalNotificationArrival(1, {
        source: "plugin",
        pluginSource: "plugin.dsh-status",
        pluginKey: "p1",
        pluginKind: "waiting",
        content: "待批准",
        pluginUri: "dsh://open",
    }, 2);
    assert.equal(notice.uri, "dsh://open");
    assert.equal(notice.componentId, "plugin.dsh-status"); // 打开时据此取岛实例。
    const readOnly = islandLocalNotificationArrival(2, {
        source: "plugin",
        pluginSource: "plugin.dsh-status",
        pluginKey: "p2",
        pluginKind: "info",
        content: "普通提醒",
    }, 3);
    assert.equal(readOnly.uri, undefined); // 无 uri 的条目保持只读。
});
