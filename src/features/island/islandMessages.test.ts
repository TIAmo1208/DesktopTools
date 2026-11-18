import assert from "node:assert/strict";
import test from "node:test";
import { parseIslandMessages } from "../../contracts/island.ts";
import { ISLAND_NOTIFICATION_ANCHOR_ID, islandDismissNotification, islandHoldMessages, islandKeepNotifications, islandLocalNotificationArrival, islandMergeMessages, islandMergeNotifications, islandMessageArrival, islandMessageFocusOrder, islandMessagePanelView, islandMessageTime, islandNotificationAnchor, islandNotificationCountLabel, islandNotificationDeadline, type IslandNotification } from "./islandMessages.ts";
import type { IslandMessage } from "../../contracts/island.ts";

/** 构造与后端投影同形的消息，只覆盖断言需要的字段。 */
const message = (id: number, content = `消息${id}`): IslandMessage => ({ id, type: "info", content, source: "test", loading: false, receivedAtMs: id });

test("合并消息按身份去重并保持到达顺序", () => {
    const merged = islandMergeMessages([message(1), message(2)], [message(2, "覆盖"), message(3)]);
    assert.deepEqual(merged.map((item) => item.id), [1, 2, 3]);
    assert.equal(merged[1]?.content, "覆盖"); // 同身份以后到内容为准。
});

test("阅读期暂存只在 hover 与展开生效，回到常态立即清空", () => {
    const held = [message(1), message(2)];
    assert.deepEqual(islandHoldMessages(held, [], "collapsed"), []);
    assert.deepEqual(islandHoldMessages(held, [message(3)], "hover").map((item) => item.id), [1, 2, 3]);
    assert.deepEqual(islandHoldMessages([], [message(4)], "expanded").map((item) => item.id), [4]);
});

test("展开消息卡仍保留最近五条原生消息且最新在前", () => {
    const incoming = [message(1), message(2), message(3), message(4), message(5), message(6)];
    assert.deepEqual(islandMessagePanelView(incoming, []).map((item) => item.id), [6, 5, 4, 3, 2]);
});

test("阅读期消失的消息仍留在展开视图，清空后回落到岛卡", () => {
    const held = [message(7), message(8)];
    assert.deepEqual(islandMessagePanelView([], held).map((item) => item.id), [8, 7]); // 后端已清理，页面仍在阅读。
    assert.deepEqual(islandMessagePanelView([], islandHoldMessages(held, [], "collapsed")), []);
});

test("键盘关闭后优先选后方最近的旧身份，再反向回退", () => {
    const ids = [5, 4, 3, 2, 1];
    assert.deepEqual(islandMessageFocusOrder(ids, 5), [4, 3, 2, 1]);
    assert.deepEqual(islandMessageFocusOrder(ids, 3), [2, 1, 4, 5]);
    assert.deepEqual(islandMessageFocusOrder(ids, 1), [2, 3, 4, 5]);
    assert.deepEqual(islandMessageFocusOrder([5], 5), []);
    assert.deepEqual(islandMessageFocusOrder(ids, 9), []);
    assert.deepEqual(ids, [5, 4, 3, 2, 1]);
});

test("消息时间当天只给时分，跨天补月日", () => {
    const now = new Date(2026, 10, 11, 12, 30).getTime();
    assert.equal(islandMessageTime(new Date(2026, 10, 11, 9, 5).getTime(), now), "09:05");
    assert.equal(islandMessageTime(new Date(2026, 10, 9, 23, 59).getTime(), now), "11/9 23:59");
});

test("消息契约拒绝未知字段、重复身份与空正文", () => {
    const valid = { id: 1, type: "warning", content: "正文", source: "来源", loading: false, receivedAtMs: 1 };
    assert.deepEqual(parseIslandMessages([valid]), [valid]);
    assert.throws(() => parseIslandMessages([{ ...valid, extra: 1 }]), /岛消息字段无效/);
    assert.throws(() => parseIslandMessages([valid, valid]), /岛消息身份无效/);
    assert.throws(() => parseIslandMessages([{ ...valid, content: "  " }]), /岛消息正文或来源无效/);
    assert.throws(() => parseIslandMessages([{ ...valid, type: "unknown" }]), /岛消息类型无效/);
    assert.throws(() => parseIslandMessages("not-an-array"), /岛消息集合无效/);
});

test("各来源通知按身份去重并投影为一枚通知锚点，逐条关闭后重新计数", () => {
    const first = islandMessageArrival({ ...message(1), source: "设置", receivedAtMs: 1_000 });
    const second = islandMessageArrival({ ...message(2), source: "插件", receivedAtMs: 2_000 });
    const collect: IslandNotification = { key: "local:1", source: "collect", content: "新收录", status: "pending", receivedAtMs: 3_000 };
    const weather: IslandNotification = { ...collect, key: "local:2", source: "weather", content: "多云", receivedAtMs: 2_500 };
    const merged = islandMergeNotifications([first], [collect, second, weather, { ...first, content: "已更新" }]);
    assert.deepEqual(merged.map((notice) => notice.source), ["collect", "weather", "插件", "设置"]);
    assert.equal(merged.at(-1)?.content, "已更新");
    assert.equal(islandNotificationAnchor(merged)?.id, ISLAND_NOTIFICATION_ANCHOR_ID);
    assert.equal(islandNotificationAnchor(merged)?.unit?.primary, "新收录");
    assert.equal(islandNotificationAnchor(merged)?.count, 4);
    const remaining = islandDismissNotification(merged, collect.key);
    assert.equal(islandNotificationAnchor(remaining)?.count, 3);
    assert.equal(islandNotificationAnchor(remaining)?.unit?.primary, "多云");
    assert.equal(islandNotificationAnchor(remaining.reduce((rows, item) => islandDismissNotification(rows, item.key), remaining)), undefined);
});

test("通知计数封顶但不丢原条目，恰好 99 条不显示加号", () => {
    const many = Array.from({ length: 103 }, (_, index) => islandMessageArrival(message(index + 1)));
    assert.equal(islandNotificationCountLabel(islandNotificationAnchor(many)?.count ?? 0), "99+");
    assert.equal(islandNotificationCountLabel(islandNotificationAnchor(many.slice(0, 99))?.count ?? 0), "99");
    assert.equal(many.length, 103);
});

test("通知逐条保留 30s，失败和确认不超时；阅读期冻结过期条目", () => {
    const pending = islandMessageArrival({ ...message(1), receivedAtMs: 1_000 });
    const failed = islandMessageArrival({ ...message(2), type: "error", loading: true, receivedAtMs: 0 });
    const awaiting: IslandNotification = { ...pending, key: "local:confirm", status: "awaiting" };
    const fresh: IslandNotification = { ...pending, key: "local:weather", source: "weather", receivedAtMs: 1_001 };
    const items = [fresh, pending, failed, awaiting];
    assert.equal(islandNotificationDeadline(pending), 31_000);
    assert.equal(islandNotificationDeadline(failed), undefined);
    assert.equal(islandNotificationDeadline(awaiting), undefined);
    assert.equal(islandKeepNotifications(items, 30_999, "collapsed"), items);
    assert.deepEqual(islandKeepNotifications(items, 31_000, "collapsed"), [fresh, failed, awaiting]);
    assert.deepEqual(islandKeepNotifications(items, 31_001, "collapsed"), [failed, awaiting]);
    for (const mode of ["hover", "expanded"] as const) assert.equal(islandKeepNotifications(items, 90_000, mode), items);
});

test("真实天气与收录走本地投影、关闭及过期，不混淆原生身份", () => {
    const weather = islandLocalNotificationArrival(1, { source: "weather", content: "阵雨" }, 1_000);
    const collect = islandLocalNotificationArrival(2, { source: "collect", content: "新收录" }, 2_000);
    assert.deepEqual(weather, { key: "local:1", source: "weather", content: "阵雨", receivedAtMs: 1_000, status: "pending" });
    assert.equal(collect.status, "pending");
    assert.equal(collect.nativeId, undefined);
    const native = islandMessageArrival({ ...message(1), type: "error" });
    const merged = islandMergeNotifications([native], [weather, collect]);
    assert.equal(islandNotificationAnchor(merged)?.count, 3);
    const remaining = islandDismissNotification(merged, collect.key);
    assert.deepEqual(remaining, [weather, native]);
    assert.deepEqual(islandKeepNotifications(remaining, 31_000, "collapsed"), [native]);
});

test("阅读期暂存只保留最近五条，不无界累积", () => {
    const many = Array.from({ length: 8 }, (_, index) => message(index + 1));
    assert.deepEqual(islandHoldMessages([], many, "expanded").map((item) => item.id), [4, 5, 6, 7, 8]);
    assert.deepEqual(islandHoldMessages(many, [message(9)], "expanded").map((item) => item.id), [5, 6, 7, 8, 9]);
});
