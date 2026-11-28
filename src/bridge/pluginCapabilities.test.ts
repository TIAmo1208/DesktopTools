import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";

import type { ComponentCapability } from "../contracts/component.ts";
import { createPluginCapabilityContext, type PluginCapabilityCommands } from "./pluginCapabilities.ts";
import { publishIslandMessagePayload } from "./pluginMessagePayload.ts";

// 记录调用的假命令；参数逐项断言，证明实例身份由宿主补齐。
const createCommands = () => {
    const calls: string[] = [];
    const commands: PluginCapabilityCommands = {
        setCredential: async (instanceId, slot, value) => void calls.push(`set:${instanceId}:${slot}:${value}`),
        clearCredential: async (instanceId, slot) => void calls.push(`clear:${instanceId}:${slot}`),
        exchangeCredential: async (instanceId, slot, request) => {
            calls.push(`exchange:${instanceId}:${slot}:${request.url}:${JSON.stringify(request.body)}:${request.extract}`);
            return { stored: true };
        },
        publishIslandMessage: async (instanceId, key, kind, text, uri) => void calls.push(`publish:${instanceId}:${key}:${kind}:${text}:${uri ?? ""}`),
        openWindow: async (instanceId, request) => void calls.push(`window:${instanceId}:${request.view}:${request.windowKey}`),
        openUri: async (instanceId, uri) => void calls.push(`open:${instanceId}:${uri}`),
        registerLoopback: async (registration) => {
            calls.push(`register:${registration.instanceId}:${registration.channelId}:${registration.mode}`);
            return { channelId: registration.channelId, mode: registration.mode, port: null, baseUrl: registration.baseUrl ?? null, registeredAt: 1 };
        },
        readLoopback: async (instanceId, channelId, afterSequence) => {
            calls.push(`read:${instanceId}:${channelId}:${afterSequence ?? ""}`);
            return [];
        },
        requestLoopback: async (instanceId, channelId, method, path, body) => {
            calls.push(`request:${instanceId}:${channelId}:${method}:${path}:${body ?? ""}`);
            return { status: 200, headers: [], body: "{}" };
        },
    };
    return { calls, commands };
};

test("上下文只暴露实例已声明的能力，其余保持 undefined", () => {
    const { commands } = createCommands();
    const none = createPluginCapabilityContext("island.plugin.demo", [], commands);
    assert.deepEqual(none, {}); // 未声明任何能力时不提供通用入口。

    const credential = createPluginCapabilityContext("island.plugin.demo", ["plugin-credential"], commands);
    assert.equal(typeof credential.setCredential, "function");
    assert.equal(typeof credential.clearCredential, "function");
    assert.equal(credential.exchangeCredential, undefined); // 兑换是独立能力。
    assert.equal(credential.publishIslandMessage, undefined);
    assert.equal(credential.openUri, undefined);
    assert.equal(credential.registerLoopbackChannel, undefined);

    const exchange = createPluginCapabilityContext("island.plugin.demo", ["plugin-credential-exchange"], commands);
    assert.equal(typeof exchange.exchangeCredential, "function");
    assert.equal(exchange.setCredential, undefined); // 只有兑换，不附带明文写入。

    const loopback = createPluginCapabilityContext("island.plugin.demo", ["loopback-channel"], commands);
    assert.equal(typeof loopback.registerLoopbackChannel, "function");
    assert.equal(typeof loopback.readLoopbackChannel, "function");
    assert.equal(typeof loopback.requestLoopback, "function");
    assert.equal(loopback.openUri, undefined);

    const all: ComponentCapability[] = ["plugin-credential", "plugin-credential-exchange", "publish-island-message", "open-uri", "loopback-channel"];
    const granted = createPluginCapabilityContext("island.plugin.demo", all, commands);
    assert.deepEqual(Object.keys(granted).sort(), [
        "clearCredential",
        "exchangeCredential",
        "openUri",
        "publishIslandMessage",
        "readLoopbackChannel",
        "registerLoopbackChannel",
        "requestLoopback",
        "setCredential",
    ]);

    assert.deepEqual(createPluginCapabilityContext("", all, commands), {}); // 没有实例身份（设置页未绑定）时不暴露写入口。
});

test("命令使用实例身份与契约参数", async () => {
    const { calls, commands } = createCommands();
    const context = createPluginCapabilityContext(
        "island.plugin.demo",
        ["plugin-credential", "plugin-credential-exchange", "publish-island-message", "open-uri", "loopback-channel"],
        commands,
    );
    await context.setCredential?.("token", "test-value");
    await context.clearCredential?.("token");
    await context.exchangeCredential?.("token", { url: "http://127.0.0.1:47832/pair", body: { code: "ABCD" }, extract: "token" });
    await context.publishIslandMessage?.({ key: "pending:p1", kind: "waiting", text: "待批准：重构登录" });
    await context.openUri?.("dsh://open");
    await context.registerLoopbackChannel?.({ instanceId: "island.plugin.demo", channelId: "bridge", mode: "request", baseUrl: "http://127.0.0.1:47832" });
    await context.readLoopbackChannel?.("bridge", 3);
    await context.requestLoopback?.("bridge", "GET", "/health", "{}");
    assert.deepEqual(calls, [
        "set:island.plugin.demo:token:test-value",
        "clear:island.plugin.demo:token",
        'exchange:island.plugin.demo:token:http://127.0.0.1:47832/pair:{"code":"ABCD"}:token',
        "publish:island.plugin.demo:pending:p1:waiting:待批准：重构登录:",
        "open:island.plugin.demo:dsh://open",
        "register:island.plugin.demo:bridge:request", // 实例身份由宿主补齐。
        "read:island.plugin.demo:bridge:3",
        "request:island.plugin.demo:bridge:GET:/health:{}",
    ]);
});

test("开窗只接受已授权实例和本地视图身份，不接受 URL 或附加脚本字段", async () => {
    const { calls, commands } = createCommands();
    const request = { view: "detail", windowKey: "record:1", title: "详情", size: { width: 640, height: 520 } };
    assert.equal(createPluginCapabilityContext("desk-1", [], commands).openWindow, undefined);
    const open = createPluginCapabilityContext("desk-1", ["plugin-window"], commands).openWindow!;
    for (const invalid of [{ ...request, view: "https://example.org" }, { ...request, url: "https://example.org" }, { ...request, minSize: { width: 700, height: 500 } }]) {
        await assert.rejects(() => open(invalid));
    }
    await open(request);
    assert.deepEqual(calls, ["window:desk-1:detail:record:1"]);
});

test("删除原生 DSH 后没有残留引用", () => {
    const removed = [
        "contracts/dsh",
        "bridge/dsh",
        "IslandDsh",
        "islandDshModel",
        "islandDsh.css",
        "settings/dsh",
        "settings-dsh.svg",
    ];
    const violations: string[] = [];
    const walk = (directory: string) => {
        for (const name of readdirSync(directory)) {
            const path = join(directory, name);
            if (statSync(path).isDirectory()) {
                walk(path);
                continue;
            }
            if (!/\.(ts|tsx|css)$/.test(path)) continue;
            if (path.endsWith("pluginCapabilities.test.ts")) continue; // 本文件自带被禁字符串清单。
            const source = readFileSync(path, "utf8");
            for (const needle of removed) if (source.includes(needle)) violations.push(`${path} → ${needle}`);
        }
    };
    walk("src");
    assert.deepEqual(violations, []);
});

test("插件消息 uri 透传到命令，缺省保持旧 5 字段载荷", async () => {
    const { calls, commands } = createCommands();
    const context = createPluginCapabilityContext("island.plugin.demo", ["publish-island-message"], commands);
    await context.publishIslandMessage?.({ key: "pending:p1", kind: "waiting", text: "待批准", uri: "dsh://open" });
    await context.publishIslandMessage?.({ key: "pending:p2", kind: "info", text: "普通提醒" });
    assert.deepEqual(calls, [
        "publish:island.plugin.demo:pending:p1:waiting:待批准:dsh://open", // 声明了去向就随命令下发。
        "publish:island.plugin.demo:pending:p2:info:普通提醒:", // 未声明不传 uri。
    ]);
    assert.deepEqual(publishIslandMessagePayload("island.plugin.demo", "p1", "waiting", "待批准", "dsh://open"), {
        instanceId: "island.plugin.demo",
        key: "p1",
        kind: "waiting",
        text: "待批准",
        uri: "dsh://open",
    });
    const legacy = publishIslandMessagePayload("island.plugin.demo", "p2", "info", "普通提醒");
    assert.deepEqual(legacy, { instanceId: "island.plugin.demo", key: "p2", kind: "info", text: "普通提醒" });
    assert.equal("uri" in legacy, false); // 旧载荷没有 uri 字段。
});
