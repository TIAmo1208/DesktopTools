import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { stripTypeScriptTypes } from "node:module";

test("收集箱主动删除事件同时交付所有格子，退订后停止", async () => {
    const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8");
    const script = source.split('const NATIVE_BRIDGE_SCRIPT: &str = r#"')[1].split('"#;')[0];
    const window: any = { addEventListener() {}, setTimeout() {}, clearTimeout() {} };
    runInNewContext(script, {
        window,
        __DESKTOP_NATIVE_ORIGIN__: "http://host.localhost",
        __REFRESH_POLICY_FALLBACK__: false,
        URLSearchParams,
        location: { search: "?monitor=0" },
    });
    const first: string[] = [];
    const second: string[] = [];
    const bridge = window.__DESKTOP_TOOLS_NATIVE__;
    const stop = await bridge.listen("collect-items-changed", (event: { deletedName: string }) => first.push(event.deletedName));
    await bridge.listen("collect-items-changed", (event: { deletedName: string }) => second.push(event.deletedName));
    bridge.dispatch("collect-items-changed", { deletedName: "测试.md" });
    assert.deepEqual(first, ["测试.md"]);
    assert.deepEqual(second, ["测试.md"]);
    stop();
    bridge.dispatch("collect-items-changed", { deletedName: "另一条.md" });
    assert.deepEqual(first, ["测试.md"]);
    assert.deepEqual(second, ["测试.md", "另一条.md"]);
});

test("时间消费者先订阅再登记，释放与登记失败均解除接收入口", async () => {
    const source = readFileSync(new URL("./components.ts", import.meta.url), "utf8")
        .replace(/^import [\s\S]*?;\r?$/gm, "")
        .replace(/^export /gm, ""); // 执行真实组件桥接。
    const calls: string[] = []; // 记录边界调用顺序。
    let failed = false; // 模拟后端拒绝登记。
    const context: any = {
        parseComponentHostEvent: (value: unknown) => value,
        listenRuntime: async () => {
            calls.push("listen");
            return () => calls.push("stop");
        },
        invokeRuntime: async (command: string) => {
            calls.push(command);
            if (failed) throw new Error("登记失败");
            return 1;
        },
    }; // 仅替换进程边界。
    runInNewContext(stripTypeScriptTypes(source) + "\nglobalThis.listen = listenComponentHostEvents;", context);
    const stop = await context.listen(
        "clock",
        "DISPLAY1",
        () => {},
        () => {},
    ); // 已授权实例订阅。
    assert.deepEqual(calls, ["listen", "subscribe_component_time"]);
    stop();
    assert.deepEqual(calls.slice(-2), ["stop", "unsubscribe_component_time"]);
    failed = true;
    await assert.rejects(
        context.listen(
            "clock",
            "DISPLAY1",
            () => {},
            () => {},
        ),
        /登记失败/,
    );
    assert.equal(calls.at(-1), "stop");
});

/** 真实桥接必须丢弃被事件超越的初始读取，且退订后不再交付。 */
for (const eventName of [
    "desktop-host-snapshot-changed",
    "workspace-snapshot-changed",
    "desktop-item-snapshot-changed",
    "component-runtime-snapshot-changed",
    "music-session-changed",
    "preferences-changed",
    "windows-accent-color-changed",
    "component-host-event",
]) {
    test(`${eventName} 保留先到事件且退订移除接收者`, async () => {
        const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8"); // 产品注入脚本。
        const script = source.split('const NATIVE_BRIDGE_SCRIPT: &str = r#"')[1].split('"#;')[0]; // 未复制业务实现。
        let finish!: (value: unknown) => void; // 控制旧快照的返回时机。
        const response = new Promise((resolve) => {
            finish = resolve;
        }); // 预先建立可控的返回通道。
        const events: number[] = []; // 实际交付顺序。
        const window: any = { addEventListener() {}, setTimeout() {}, clearTimeout() {} }; // 本测试无需推进健康计时器。
        runInNewContext(script, {
            window,
            __DESKTOP_NATIVE_ORIGIN__: "http://host.localhost",
            __REFRESH_POLICY_FALLBACK__: false,
            URLSearchParams,
            location: { search: "?monitor=0" },
            fetch: async () => ({ json: () => response }),
        });
        const subscribed = window.__DESKTOP_TOOLS_NATIVE__.listen(eventName, (value: { revision: number }) => events.push(value.revision)); // 先建立入口再读取。
        await Promise.resolve();
        window.__DESKTOP_TOOLS_NATIVE__.dispatch(eventName, { revision: 2, hosts: [] });
        finish({ ok: true, result: { revision: 1, hosts: [] } });
        const unlisten = await subscribed; // 完成初始读取后仍应保留事件新值。
        assert.deepEqual(events, [2]);
        window.__DESKTOP_TOOLS_NATIVE__.dispatch(eventName, { revision: 3, hosts: [] });
        assert.deepEqual(events, [2, 3]);
        unlisten();
        window.__DESKTOP_TOOLS_NATIVE__.dispatch(eventName, { revision: 4, hosts: [] });
        assert.deepEqual(events, [2, 3]);
    });
}
