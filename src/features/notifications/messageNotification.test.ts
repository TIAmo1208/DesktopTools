import assert from "node:assert/strict";
import { test } from "node:test";
import { publishMessage } from "./messageNotification.ts";

test("消息提交不在调用方同步抛错", () => {
    assert.doesNotThrow(() => publishMessage({ type: "error", content: "连接失败", source: "music" }));
});

test("消息发送携带毫秒期限，非法期限不进入原生队列", async () => {
    const sent: unknown[] = []; // 捕获真正交给运行时桥的载荷。
    Object.defineProperty(globalThis, "window", { configurable: true, value: { __DESKTOP_TOOLS_NATIVE__: { invoke: async (_command: string, payload: unknown) => { sent.push(payload); } } } });
    try {
        publishMessage({ type: "info", content: "默认", source: "test" });
        publishMessage({ type: "info", content: "短消息", source: "test", durationMs: 1000 });
        for (const durationMs of [0, -1, 1.5, NaN, Infinity, 0x100000000]) publishMessage({ type: "info", content: "非法", source: "test", durationMs });
        await Promise.resolve();
        assert.deepEqual(sent, [
            { message: { type: "info", content: "默认", source: "test", durationMs: 4000 } },
            { message: { type: "info", content: "短消息", source: "test", durationMs: 1000 } },
        ]);
    } finally {
        Reflect.deleteProperty(globalThis, "window");
    }
});
