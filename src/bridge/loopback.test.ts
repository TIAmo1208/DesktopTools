import assert from "node:assert/strict";
import test from "node:test";

import { createLoopbackHub, createRequestLoopback } from "./loopback.ts";
import type { ComponentHostEvent } from "../contracts/component.ts";
import type { LoopbackRequestEntry } from "../contracts/loopback.ts";

// 主动拉取封装发 request_loopback，GET /v1/snapshot 只发一次；非 2xx 仍是完整响应。
test("request command wrapper asks once for the snapshot", async () => {
    const calls: Array<{ command: string; payload: Record<string, unknown> }> = [];
    const request = createRequestLoopback(async (command, payload) => {
        calls.push({ command, payload });
        return { status: 200, headers: [{ name: "Content-Type", value: "application/json" }], body: "{\"schemaVersion\":2}" };
    });
    const response = await request("instance-a", "astrbot-request", "GET", "/v1/snapshot");
    assert.equal(response.status, 200);
    assert.deepEqual(calls, [
        {
            command: "request_loopback",
            payload: { instanceId: "instance-a", channelId: "astrbot-request", method: "GET", path: "/v1/snapshot" },
        },
    ]);
    assert.equal(calls.some((call) => call.command === "read_loopback_channel"), false);
});

/** 假事件源与假读取：验证去重、暂停与退订，不依赖运行时。 */
const createHub = () => {
    const listeners: Array<(event: ComponentHostEvent) => void> = [];
    const stops: number[] = [];
    const reads: Array<[string, string]> = [];
    const hub = createLoopbackHub({
        listen: (_name, handler) => {
            listeners.push(handler);
            return Promise.resolve(() => stops.push(1));
        },
        read: (instanceId, channelId) => {
            reads.push([instanceId, channelId]);
            return Promise.resolve([{ sequence: 1, receivedAt: 1, method: "POST", path: "/v1/snapshot", headers: [], body: "{}" }] as LoopbackRequestEntry[]);
        },
    });
    return { hub, listeners, stops, reads };
};

const event = (kind: string): ComponentHostEvent => ({ sequence: 1, kind: kind as ComponentHostEvent["kind"], unixMillis: 1 });
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

// 同一实例通道的两个读者共用一个底层监听，事件只读一次缓冲。
test("同一实例通道只建立一条监听", async () => {
    const { hub, listeners, reads } = createHub();
    const first: LoopbackRequestEntry[][] = [];
    const second: LoopbackRequestEntry[][] = [];
    const stopFirst = hub.subscribe("instance-a", "astrbot-push", { onEntries: (entries) => first.push(entries), onError: () => undefined });
    const stopSecond = hub.subscribe("instance-a", "astrbot-push", { onEntries: (entries) => second.push(entries), onError: () => undefined });
    await settle();
    assert.equal(listeners.length, 1);

    listeners[0](event("loopback-data:astrbot-push"));
    await settle();
    assert.equal(reads.length, 1);
    assert.equal(first.length, 1);
    assert.equal(second.length, 1);

    stopFirst();
    stopSecond();
    hub.dispose();
});

// 不同通道的通知互不串读；最后一位读者退订才释放监听。
test("按通道过滤并在退订后释放", async () => {
    const { hub, listeners, reads, stops } = createHub();
    const received: LoopbackRequestEntry[][] = [];
    const stop = hub.subscribe("instance-a", "astrbot-push", { onEntries: (entries) => received.push(entries), onError: () => undefined });
    await settle();

    listeners[0](event("loopback-data:astrbot-request"));
    await settle();
    assert.equal(reads.length, 0);

    listeners[0](event("time-tick"));
    await settle();
    assert.equal(reads.length, 0);

    stop();
    await settle();
    assert.equal(stops.length, 1);

    stops.length = 0;
    const stopAgain = hub.subscribe("instance-a", "astrbot-push", { onEntries: () => undefined, onError: () => undefined });
    await settle();
    stopAgain();
    hub.dispose();
});

// 不可见时暂停交付；恢复时补读一次，不重放积压事件。
test("不可见时暂停并在恢复时补读", async () => {
    const { hub, listeners, reads } = createHub();
    const received: LoopbackRequestEntry[][] = [];
    hub.subscribe("instance-a", "astrbot-push", { onEntries: (entries) => received.push(entries), onError: () => undefined });
    await settle();

    hub.setPaused("instance-a", "astrbot-push", true);
    listeners[0](event("loopback-data:astrbot-push"));
    await settle();
    assert.equal(received.length, 0);
    assert.equal(reads.length, 0);

    hub.setPaused("instance-a", "astrbot-push", false);
    await settle();
    assert.equal(received.length, 1);
    assert.equal(reads.length, 1);
    hub.dispose();
});
