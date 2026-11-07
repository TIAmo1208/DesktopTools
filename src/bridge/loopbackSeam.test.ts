import assert from "node:assert/strict";
import test from "node:test";

import { createLoopbackHub, readLoopbackChannel, registerLoopbackChannel, requestLoopback } from "./loopback.ts";
import { LOOPBACK_EVENT_PREFIX, parseComponentHostEvent, type ComponentHostEvent } from "../contracts/component.ts";
import type { LoopbackRequestEntry } from "../contracts/loopback.ts";

/** 后端 event_kind 拼出的通知载荷，字段与 Rust 序列化逐字段一致。 */
const event = (kind: string, sequence = 1): ComponentHostEvent => ({ sequence, kind: kind as ComponentHostEvent["kind"], unixMillis: 1_700_000_000_000 });

/** 造一条宿主缓冲条目。 */
const entry = (sequence: number, body: string): LoopbackRequestEntry => ({
    sequence,
    receivedAt: 1_700_000_000,
    method: "POST",
    path: "/v1/snapshot",
    headers: [{ name: "Content-Type", value: "application/json" }],
    body,
});

const settle = (milliseconds = 10) => new Promise((resolve) => setTimeout(resolve, milliseconds));

// 通知只认 loopback-data:<channelId>，与后端 event_kind、TS 契约前缀一致。
test("loopback 通知载荷与后端 kind 一致", () => {
    assert.equal(LOOPBACK_EVENT_PREFIX, "loopback-data:");
    assert.deepEqual(parseComponentHostEvent(event("loopback-data:astrbot-push")), {
        sequence: 1,
        kind: "loopback-data:astrbot-push",
        unixMillis: 1_700_000_000_000,
    });
    assert.throws(() => parseComponentHostEvent({ sequence: 1, kind: "loopback-data:", unixMillis: 1 }));
    assert.throws(() => parseComponentHostEvent({ sequence: 0, kind: "loopback-data:astrbot-push", unixMillis: 1 }));
});

// 同一通道在两个实例上各读自己的缓冲，退订一个不影响另一个。
test("多实例按各自实例读取，退订互不影响", async () => {
    const listeners: Array<(received: ComponentHostEvent) => void> = [];
    const reads: Array<[string, string]> = [];
    const hub = createLoopbackHub({
        listen: (_name, handler) => {
            listeners.push(handler);
            return Promise.resolve(() => undefined);
        },
        read: (instanceId, channelId) => {
            reads.push([instanceId, channelId]);
            return Promise.resolve([entry(1, JSON.stringify({ instanceId }))]);
        },
    });
    const receivedA: LoopbackRequestEntry[][] = [];
    const receivedB: LoopbackRequestEntry[][] = [];
    const stopA = hub.subscribe("instance-a", "astrbot-push", { onEntries: (entries) => receivedA.push(entries), onError: () => undefined });
    const stopB = hub.subscribe("instance-b", "astrbot-push", { onEntries: (entries) => receivedB.push(entries), onError: () => undefined });
    await settle();

    listeners[0](event("loopback-data:astrbot-push"));
    await settle();
    assert.deepEqual(reads, [
        ["instance-a", "astrbot-push"],
        ["instance-b", "astrbot-push"],
    ]);
    assert.equal(receivedA[0][0].body, JSON.stringify({ instanceId: "instance-a" }));
    assert.equal(receivedB[0][0].body, JSON.stringify({ instanceId: "instance-b" }));

    stopA();
    listeners[0](event("loopback-data:astrbot-push", 2));
    await settle();
    assert.equal(reads.filter(([instanceId]) => instanceId === "instance-a").length, 1, "退订后实例 A 不再读取");
    assert.equal(reads.filter(([instanceId]) => instanceId === "instance-b").length, 2, "实例 B 不受影响");

    stopB();
    hub.dispose();
});

// 第二个假数据源换一条通道与一套 JSON，前端契约层零改动即可分别解析。
test("两路数据源各自解析互不干扰", async () => {
    const listeners: Array<(received: ComponentHostEvent) => void> = [];
    const bodies: Record<string, string> = {
        "astrbot-push": JSON.stringify({ schemaVersion: 2, service: { probe: { level: "ok" } } }),
        "demo-push": JSON.stringify({ source: "demo", items: [1, 2, 3] }),
    };
    const hub = createLoopbackHub({
        listen: (_name, handler) => {
            listeners.push(handler);
            return Promise.resolve(() => undefined);
        },
        read: (_instanceId, channelId) => Promise.resolve([entry(1, bodies[channelId])]),
    });
    const seen: Array<[string, unknown]> = [];
    const stopA = hub.subscribe("instance-a", "astrbot-push", {
        onEntries: (entries) => seen.push(["astrbot", JSON.parse(entries[0].body).schemaVersion]),
        onError: () => undefined,
    });
    const stopB = hub.subscribe("instance-b", "demo-push", {
        onEntries: (entries) => seen.push(["demo", JSON.parse(entries[0].body).items.length]),
        onError: () => undefined,
    });
    await settle();

    listeners[0](event("loopback-data:astrbot-push"));
    listeners[0](event("loopback-data:demo-push", 2));
    await settle();

    assert.deepEqual(seen, [
        ["astrbot", 2],
        ["demo", 3],
    ]);
    stopA();
    stopB();
    hub.dispose();
});

// 三条命令的线上参数名必须与 Rust 命令分派读取的键一致（两种 transport 共用）。
test("三条命令的参数名与后端分派一致", async () => {
    const calls: Array<[string, Record<string, unknown>]> = [];
    const target = globalThis as Record<string, unknown>;
    const previous = target.window;
    target.window = {
        __DESKTOP_TOOLS_NATIVE__: {
            invoke: (command: string, payload: Record<string, unknown>) => {
                calls.push([command, payload]);
                if (command === "register_loopback_channel") {
                    return Promise.resolve({
                        channelId: payload.channelId,
                        mode: payload.mode,
                        port: payload.mode === "listen" ? payload.port : null,
                        baseUrl: payload.mode === "request" ? payload.baseUrl : null,
                        registeredAt: 1_700_000_000,
                    });
                }
                if (command === "read_loopback_channel") return Promise.resolve([]);
                return Promise.resolve({ status: 200, headers: [], body: "ok" });
            },
            listen: () => Promise.resolve(() => undefined),
        },
    };
    try {
        await registerLoopbackChannel({
            instanceId: "instance-a",
            channelId: "astrbot-push",
            mode: "listen",
            port: 8765,
            credentialRef: "DesktopTools/astrbot-status",
            headerName: "X-Status-Token",
        });
        await readLoopbackChannel("instance-a", "astrbot-push", 4);
        await requestLoopback("instance-a", "astrbot-request", "GET", "/v1/snapshot");
        await requestLoopback("instance-a", "astrbot-request", "POST", "/v1/actions", "{}");
    } finally {
        target.window = previous;
    }
    assert.deepEqual(calls, [
        [
            "register_loopback_channel",
            {
                instanceId: "instance-a",
                channelId: "astrbot-push",
                mode: "listen",
                port: 8765,
                credentialRef: "DesktopTools/astrbot-status",
                headerName: "X-Status-Token",
            },
        ],
        ["read_loopback_channel", { instanceId: "instance-a", channelId: "astrbot-push", afterSequence: 4 }],
        ["request_loopback", { instanceId: "instance-a", channelId: "astrbot-request", method: "GET", path: "/v1/snapshot" }],
        ["request_loopback", { instanceId: "instance-a", channelId: "astrbot-request", method: "POST", path: "/v1/actions", body: "{}" }],
    ]);
});
