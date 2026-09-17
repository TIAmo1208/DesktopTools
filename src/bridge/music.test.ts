import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

/** 执行正式宿主注入脚本，覆盖音乐刷新、故障恢复和退订中的迟到结果。 */
test("正式音乐订阅更新、故障后相同值恢复、退订不再交付", async () => {
    const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8"); // 真实注入脚本来源。
    const script = source.match(/const NATIVE_BRIDGE_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1]; // Rust 原始字符串内容。
    assert.ok(script);
    const timers: Array<() => Promise<void>> = []; // 手动推进的轮询任务。
    const events: unknown[] = []; // 实际事件回调载荷。
    const requests: string[] = []; // 实际读取命令。
    let reply: unknown = { ok: true, result: { media: null } }; // 本轮模拟的 native transport 返回。
    let finish: (() => void) | undefined; // 控制退订期间的在途请求。
    const window = { setTimeout: (callback: () => Promise<void>) => timers.push(callback), clearTimeout: () => {}, addEventListener: () => {} } as Record<string, any>; // 最小宿主环境。
    const nativeOrigin = "http://desktop-tools-host-0.localhost"; // 与 Rust 注入的逐屏协议地址一致。
    runInNewContext(script.replace("__DESKTOP_NATIVE_ORIGIN__", JSON.stringify(nativeOrigin)), {
        window,
        fetch: async (url: string) => {
            requests.push(url);
            if (finish)
                await new Promise<void>((resolve) => {
                    finish = resolve;
                });
            return { json: async () => reply };
        },
    });
    const unlisten = await window.__DESKTOP_TOOLS_NATIVE__.listen("music-session-changed", (payload: unknown) => events.push(payload)); // 真实订阅入口。
    assert.equal(requests.length, 1);
    assert.equal(events.length, 1);
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("music-session-changed", { media: null, error: "连接失败" });
    assert.equal((events.at(-1) as { error: string }).error, "连接失败");
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("music-session-changed", { media: null });
    assert.equal(events.length, 3);
    unlisten();
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("music-session-changed", { media: { title: "迟到值" } });
    assert.equal(events.length, 3);
    assert.equal(timers.length, 0);
});
