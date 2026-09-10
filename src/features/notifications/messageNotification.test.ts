import assert from "node:assert/strict";
import { test } from "node:test";
import { dismissMessage, getMessages, publishMessage, subscribeMessages } from "./messageNotification.ts";

test("消息挂载前保留、来源隔离去重、按顺序关闭及订阅释放", () => {
    const message = { type: "error" as const, content: "连接失败", source: "music", dedupeKey: "connection" }; // 当前故障消息。
    publishMessage(message);
    message.content = "不应影响快照";
    assert.equal(getMessages()[0].content, "连接失败");
    let updates = 0; // 当前有效订阅收到的更新数。
    const unsubscribe = subscribeMessages(() => updates++); // 当前测试订阅的释放入口。
    publishMessage(message);
    publishMessage({ ...message, source: "workspace" });
    assert.equal(getMessages().length, 2);
    assert.equal(updates, 1);
    const first = getMessages()[0]; // 关闭时绑定的具体消息，防止误删下一条。
    dismissMessage(first);
    dismissMessage(first);
    assert.equal(getMessages()[0].source, "workspace");
    unsubscribe();
    const count = updates; // 释放订阅后的通知计数。
    publishMessage({ ...message, content: " " });
    assert.equal(getMessages().length, 1);
    publishMessage(message);
    assert.equal(updates, count);
    assert.equal(getMessages().length, 2);
    for (const item of getMessages()) dismissMessage(item);
});
