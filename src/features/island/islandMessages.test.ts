import assert from "node:assert/strict";
import test from "node:test";
import { parseIslandMessages } from "../../contracts/island.ts";
import { islandHoldMessages, islandMergeMessages, islandMessageTime, islandMessageView } from "./islandMessages.ts";
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

test("三态投影：常态与 hover 只留最新一条，展开保留最近五条且最新在前", () => {
    const incoming = [message(1), message(2), message(3), message(4), message(5), message(6)];
    assert.deepEqual(islandMessageView(incoming, [], "collapsed").map((item) => item.id), [6]);
    assert.deepEqual(islandMessageView(incoming, [], "hover").map((item) => item.id), [6]);
    assert.deepEqual(islandMessageView(incoming, [], "expanded").map((item) => item.id), [6, 5, 4, 3, 2]);
});

test("阅读期消失的消息仍留在展开视图，清空后回落到岛卡", () => {
    const held = [message(7), message(8)];
    assert.deepEqual(islandMessageView([], held, "expanded").map((item) => item.id), [8, 7]); // 后端已清理，页面仍在阅读。
    assert.deepEqual(islandMessageView([], [], "expanded"), []); // 离开阅读态后回落。
    assert.deepEqual(islandMessageView([], held, "collapsed"), []); // 常态不展示暂存消息。
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

test("阅读期暂存只保留最近五条，不无界累积", () => {
    const many = Array.from({ length: 8 }, (_, index) => message(index + 1));
    assert.deepEqual(islandHoldMessages([], many, "expanded").map((item) => item.id), [4, 5, 6, 7, 8]);
    assert.deepEqual(islandHoldMessages(many, [message(9)], "expanded").map((item) => item.id), [5, 6, 7, 8, 9]);
});
