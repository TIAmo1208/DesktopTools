import assert from "node:assert/strict";
import test from "node:test";
import { parseIslandMessages } from "../../contracts/island.ts";
import { ISLAND_NOTIFICATION_ANCHOR_ID, islandCapsuleGlyph, islandDismissNotification, islandHoldMessages, islandKeepNotifications, islandLocalNotificationArrival, islandMergeMessages, islandMergeNotifications, islandMessageArrival, islandMessageFocusOrder, islandMessagePanelView, islandMessageTime, islandNotificationAnchor, islandNotificationContentDeadline, islandNotificationContentVisible, islandNotificationCountLabel, islandNotificationDeadline, islandNotificationTone, islandNotificationVisual, type IslandNotification } from "./islandMessages.ts";
import { islandHoverContent } from "./islandContent.ts";
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

test("通知正文先展示一段、图标再保留一段，失败确认与有效进度不超时；阅读期冻结过期条目", () => {
    const pending = islandMessageArrival({ ...message(1), receivedAtMs: 1_000 });
    const failed = islandMessageArrival({ ...message(2), type: "error", loading: true, receivedAtMs: 0 });
    const awaiting: IslandNotification = { ...pending, key: "local:confirm", status: "awaiting" };
    const fresh: IslandNotification = { ...pending, key: "local:weather", source: "weather", receivedAtMs: 1_001 };
    const running: IslandNotification = { ...pending, key: "local:progress", loading: true, progress: 0.4 };
    const items = [fresh, pending, failed, awaiting, running];
    // 两段时长各自从到达时刻起算、互不串接：正文 5s 截止在 6000，寿命取两者较长的 30s，到 31000 移除。
    assert.equal(islandNotificationContentDeadline(pending), 6_000);
    assert.equal(islandNotificationDeadline(pending), 31_000);
    assert.equal(islandNotificationDeadline(failed), undefined);
    assert.equal(islandNotificationDeadline(awaiting), undefined);
    assert.equal(islandNotificationDeadline(running), undefined, "有明确且未走满的进度不自动移除");
    assert.equal(islandNotificationContentVisible(pending, 5_999), true);
    assert.equal(islandNotificationContentVisible(pending, 6_000), false);
    assert.equal(islandKeepNotifications(items, 30_999, "collapsed"), items);
    assert.deepEqual(islandKeepNotifications(items, 31_000, "collapsed"), [fresh, failed, awaiting, running], "同批到达的末尾条目仍差 1ms 到期");
    assert.deepEqual(islandKeepNotifications(items, 31_001, "collapsed"), [failed, awaiting, running]);
    for (const mode of ["hover", "expanded"] as const) assert.equal(islandKeepNotifications(items, 90_000, mode), items);
});

test("两段时长独立起算：正文长于锚点时一起结束，正文短于锚点时锚点继续留", () => {
    const notice = islandMessageArrival({ ...message(1), receivedAtMs: 1_000 });
    // 正文 40s、锚点 30s：正文更长，寿命跟正文一起结束，没有只留图标的阶段。
    const longContent = { contentMs: 40_000, anchorMs: 30_000 };
    assert.equal(islandNotificationDeadline(notice, longContent), 41_000);
    assert.equal(islandNotificationContentVisible(notice, 40_999, longContent), true);
    assert.equal(islandNotificationContentVisible(notice, 41_000, longContent), false);
    // 正文 5s、锚点 30s：正文收起后锚点再留 25s。
    const shortContent = { contentMs: 5_000, anchorMs: 30_000 };
    assert.equal(islandNotificationDeadline(notice, shortContent), 31_000);
    assert.equal(islandNotificationContentVisible(notice, 5_999, shortContent), true);
    assert.equal(islandNotificationContentVisible(notice, 6_000, shortContent), false);
});

test("正文期结束后锚点只留身份与计数，胶囊不再有可展示正文", () => {
    const notice = islandMessageArrival({ ...message(1), receivedAtMs: 1_000 });
    assert.equal(islandNotificationAnchor([notice], { nowMs: 5_999 })?.unit?.primary, "消息1");
    const anchor = islandNotificationAnchor([notice], { nowMs: 6_000 });
    assert.equal(anchor?.id, ISLAND_NOTIFICATION_ANCHOR_ID);
    assert.equal(anchor?.unit, undefined, "正文期结束不再提供内容单元");
    assert.equal(anchor?.count, 1, "锚点身份与计数仍保留");
    // hover 仍能从计数得到可读行，不显示空内容。
    assert.equal(islandHoverContent(anchor).lines[0], "1 条消息");
});

test("胶囊图标按最新条目类型：普通铃铛、成功对勾、警告三角、错误叉、加载进度环", () => {
    const glyph = (type: IslandMessage["type"], loading = false, progress?: number) => islandCapsuleGlyph({ ...islandMessageArrival({ ...message(1), type, loading }), progress });
    assert.deepEqual(glyph("info"), { kind: "bell" });
    assert.deepEqual(glyph("success"), { kind: "type", type: "success" });
    assert.deepEqual(glyph("warning"), { kind: "type", type: "warning" });
    assert.deepEqual(glyph("error"), { kind: "type", type: "error" });
    assert.deepEqual(glyph("info", true, 0.42), { kind: "progress", progress: 0.42 });
    assert.deepEqual(glyph("info", true), { kind: "loading" });
    assert.deepEqual(glyph("info", true, 1.2), { kind: "loading" }, "非法进度回落不定加载");
    assert.deepEqual(islandCapsuleGlyph(undefined), { kind: "bell" }, "没有条目时仍用铃铛");
    const weather = islandLocalNotificationArrival(1, { source: "weather", content: "多云", weatherCode: "302" }, 1_000);
    assert.deepEqual(islandCapsuleGlyph(weather), { kind: "bell" }, "天气本地条目在胶囊用铃铛");
});

test("铃铛显示真实原生类型，加载优先；进度和天气只认投影事实", () => {
    for (const type of ["info", "success", "warning", "error"] as const) {
        const notice = islandMessageArrival({ ...message(1), type });
        assert.equal(islandNotificationTone(notice), type);
        assert.deepEqual(islandNotificationVisual(notice), { kind: "type", type });
    }
    const loading = islandMessageArrival({ ...message(1, "已完成 45%"), type: "error", loading: true });
    assert.equal(islandNotificationTone(loading), "loading");
    assert.equal(loading.status, "failed"); // 原生到期语义不随视觉覆写。
    assert.deepEqual(islandNotificationVisual(loading), { kind: "loading" });
    for (const progress of [0, 0.45, 1]) assert.deepEqual(islandNotificationVisual({ ...loading, progress }), { kind: "progress", progress });
    for (const progress of [-1, 1.01, NaN, Infinity]) assert.deepEqual(islandNotificationVisual({ ...loading, progress }), { kind: "loading" });
    const weather = islandLocalNotificationArrival(1, { source: "weather", content: "天气变化", weatherCode: "302" }, 1000);
    assert.deepEqual(islandNotificationVisual(weather), { kind: "weather", code: "302" });
    assert.equal(islandNotificationVisual(islandLocalNotificationArrival(2, { source: "collect", content: "收录内容" }, 2000)), undefined);
});

test("天气展望事实附在本地条目，不更改摘要或到期时刻", () => {
    const facts = { current: { condition: { code: "100" } }, hourly: { hours: [{ condition: { code: "302" } }] } };
    const weather = islandLocalNotificationArrival(1, { source: "weather", content: "晴", weatherCode: "100", weatherOutlookFacts: facts }, 1_000);
    assert.equal(weather.weatherOutlookFacts, facts);
    assert.equal(islandNotificationAnchor([weather])?.unit?.primary, "晴");
    assert.equal(islandNotificationDeadline(weather), 31_000);
    assert.equal(islandMessageArrival(message(1)).weatherOutlookFacts, undefined);
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
