import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

/** 读取浮岛传输相关的原生源码，断言命令与事件都已登记。 */
const source = (path: string) => readFileSync(new URL(`../../../${path}`, import.meta.url), "utf8");

test("浮岛消息命令已进入原生 transport 分发表", () => {
    const commands = source("src-tauri/src/commands/mod.rs");
    for (const name of ["get_island_messages", "dismiss_island_message"]) assert.ok(commands.includes(`"${name}" =>`), `原生 transport 缺少命令 ${name}`);
});

test("浮岛消息事件已进入转发白名单与注入桥映射", () => {
    const host = source("src-tauri/src/platform/windows/desktop_host.rs");
    assert.ok(host.includes(`"island-messages-changed"`), "宿主未转发浮岛消息事件");
    assert.ok(host.includes(`"island-messages-changed": "get_island_messages"`), "注入桥未映射浮岛消息事件");
});

test("注入桥按映射读取投影，并把事件载荷交给订阅者", async () => {
    const host = source("src-tauri/src/platform/windows/desktop_host.rs");
    const script = host.match(/const NATIVE_BRIDGE_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1]; // 生产注入脚本原文。
    assert.ok(script);
    const requests: string[] = []; // 实际读取的命令地址。
    const received: unknown[] = []; // 订阅者收到的载荷。
    const payload = [{ id: 1, type: "info", content: "正文", source: "来源", loading: false, receivedAtMs: 1 }]; // 与 Rust 投影同形。
    const window = { setTimeout: () => 0, clearTimeout: () => {}, addEventListener: () => {} } as Record<string, unknown>;
    runInNewContext(script.replace("__DESKTOP_NATIVE_ORIGIN__", JSON.stringify("http://desktop-tools-island.localhost")).replace("__REFRESH_POLICY_FALLBACK__", "false"), {
        window,
        location: { search: "" },
        fetch: async (url: string) => {
            requests.push(url);
            return { json: async () => ({ ok: true, result: payload }) };
        },
    });
    const bridge = window.__DESKTOP_TOOLS_NATIVE__ as { listen: (name: string, handler: (payload: unknown) => void) => Promise<() => void>; dispatch: (name: string, payload: unknown) => void };
    const stop = await bridge.listen("island-messages-changed", (value) => received.push(value));
    assert.match(requests[0]!, /\/__desktop_tools_invoke\/get_island_messages$/);
    assert.deepEqual(received, [payload]);
    bridge.dispatch("island-messages-changed", payload); // 事件直接投递同一份完整投影。
    assert.equal(received.length, 2);
    stop();
    bridge.dispatch("island-messages-changed", payload);
    assert.equal(received.length, 2); // 退订后不再交付。
});

test("三态表面不透明度与可插值属性已登记", () => {
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    assert.match(css, /@property --island-surface-alpha \{[^}]*syntax: "<percentage>"/); // 过渡依赖注册属性。
    for (const [state, alpha] of [["collapsed", "60%"], ["hover", "75%"], ["expanded", "90%"]] as const) {
        const selector = state === "collapsed" ? ".island" : `.island[data-state="${state}"]`;
        assert.match(css, new RegExp(`${selector.replace(/[.[\]]/g, "\\$&")} \\{\\s*--island-surface-alpha: ${alpha};`));
    }
});
