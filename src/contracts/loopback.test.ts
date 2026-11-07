import assert from "node:assert/strict";
import test from "node:test";

import { parseLoopbackChannelHandle, parseLoopbackRequestEntries, parseLoopbackRequestEntry, parseLoopbackResponse } from "./loopback.ts";

// listen 句柄必须有端口，request 句柄必须有本机地址；字段原样保留。
test("parses loopback channel handles", () => {
    assert.deepEqual(parseLoopbackChannelHandle({ channelId: "push", mode: "listen", port: 8765, baseUrl: null, registeredAt: 7 }), {
        channelId: "push",
        mode: "listen",
        port: 8765,
        baseUrl: null,
        registeredAt: 7,
    });
    assert.deepEqual(parseLoopbackChannelHandle({ channelId: "pull", mode: "request", port: null, baseUrl: "http://127.0.0.1:8766", registeredAt: 7 }), {
        channelId: "pull",
        mode: "request",
        port: null,
        baseUrl: "http://127.0.0.1:8766",
        registeredAt: 7,
    });
    assert.throws(() => parseLoopbackChannelHandle({ channelId: "push", mode: "listen", port: null, baseUrl: null, registeredAt: 7 }));
});

// 原始请求保留请求头顺序与重复，正文是未解析的字符串。
test("parses raw request entries with ordered headers", () => {
    const entries = parseLoopbackRequestEntries([
        {
            sequence: 3,
            receivedAt: 100,
            method: "POST",
            path: "/v1/snapshot",
            headers: [
                { name: "X-Status-Token", value: "secret" },
                { name: "X-Duplicate", value: "a" },
                { name: "X-Duplicate", value: "b" },
            ],
            body: "{\"schemaVersion\":2}",
        },
    ]);
    assert.equal(entries.length, 1);
    assert.deepEqual(
        entries[0].headers.map((header) => header.value),
        ["secret", "a", "b"],
    );
    assert.equal(entries[0].body, "{\"schemaVersion\":2}");
    assert.throws(() => parseLoopbackRequestEntries([{ sequence: 0 }]));
});

// 单条原始请求与 read 的数组项共用同一套字段校验。
test("parses a single request entry", () => {
    assert.deepEqual(
        parseLoopbackRequestEntry({
            sequence: 4,
            receivedAt: 100,
            method: "POST",
            path: "/v1/snapshot",
            headers: [{ name: "X-Status-Token", value: "secret" }],
            body: "{\"schemaVersion\":2}",
        }),
        { sequence: 4, receivedAt: 100, method: "POST", path: "/v1/snapshot", headers: [{ name: "X-Status-Token", value: "secret" }], body: "{\"schemaVersion\":2}" },
    );
    assert.throws(() => parseLoopbackRequestEntry({ sequence: 0, receivedAt: 100, method: "POST", path: "/", headers: [], body: "" }));
});

// 非 2xx 仍是完整响应而不是契约错误，由插件判断。
test("parses non-2xx responses", () => {
    assert.deepEqual(parseLoopbackResponse({ status: 401, headers: [], body: "" }), { status: 401, headers: [], body: "" });
    assert.throws(() => parseLoopbackResponse({ status: 99, headers: [], body: "" }));
});
