import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

/** 运行真实注入脚本，验证暂停不再拉取快照及恢复只发布最新值。 */
test("桌面暂停保留命令心跳，恢复最新快照且不产生重复轮询", async () => {
    const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8"); // 产品注入脚本来源。
    const script = source.split('const NATIVE_BRIDGE_SCRIPT: &str = r#"')[1].split('"#;')[0].replace("__DESKTOP_NATIVE_ORIGIN__", '"http://desktop-tools-host-0.localhost"').replace("__REFRESH_POLICY_FALLBACK__", "false"); // 与产品相同的本地协议，全部 Hook 成功。
    let now = 1; // 虚拟单调时钟。
    let nextId = 0; // 唯一计时器身份。
    let pause = false; // 后端策略结果。
    let value = 1; // 权威组件快照。
    const timers = new Map<number, { at: number; callback: () => unknown }>(); // 可控调度队列。
    const calls: string[] = []; // 实际请求顺序。
    const handlers = new Map<string, Function>(); // DOM 事件监听器。
    const window: any = {
        setTimeout(callback: () => unknown, delay: number) { const id = ++nextId; timers.set(id, { at: now + delay, callback }); return id; },
        clearTimeout(id: number) { timers.delete(id); },
        addEventListener(name: string, handler: Function) { handlers.set(name, handler); },
        dispatchEvent() {},
    }; // 注入脚本使用的最小页面能力。
    vm.runInNewContext(script, {
        window, location: { search: "?monitor=0" }, URLSearchParams, performance: { now: () => now },
        document: { hidden: false, getAnimations: () => [], querySelector: () => null }, CustomEvent: class {},
        fetch: async (url: string) => { const command = url.split("/").at(-1)!; calls.push(command); return { json: async () => ({ ok: true, result: command === "get_desktop_refresh_policy" ? { paused: pause, committed: true, pauseOnFocus: pause, pauseOnFullscreen: false, snapshot: { revision: pause ? 2 : 1, locked: false, externalFocus: pause, coverage: [] } } : command === "get_desktop_host_snapshot" ? { hosts: [{ monitor: { id: "DISPLAY1" } }] } : value }) }; },
    });
    const values: number[] = []; // 实际交付给 React 的值。
    const musicValues: number[] = []; // 音乐恢复应读取缓存并唤醒后端收敛。
    await window.__DESKTOP_TOOLS_NATIVE__.listen("desktop-host-snapshot-changed", () => {});
    const unlisten = await window.__DESKTOP_TOOLS_NATIVE__.listen("component-host-event", (next: number) => values.push(next)); // 实际产品订阅。
    const unlistenMusic = await window.__DESKTOP_TOOLS_NATIVE__.listen("music-session-changed", (next: number) => musicValues.push(next));
    const advance = async (milliseconds: number) => {
        const end = now + milliseconds; // 本轮时间上限。
        for (;;) { const entry = [...timers].sort((a, b) => a[1].at - b[1].at)[0]; if (!entry || entry[1].at > end) break; timers.delete(entry[0]); now = entry[1].at; await entry[1].callback(); }
        now = end;
    }; // 串行执行已到期任务。
    await advance(500);
    assert.deepEqual(values, [1]);
    assert.deepEqual(musicValues, [1]);
    assert.equal(calls.filter(command => command === "get_desktop_host_snapshot").length, 1, "宿主只在订阅时读取，不进行周期轮询");
    assert.equal(timers.size, 0, "全部 WinEvent 可用时空闲期不保留 5 秒检查");
    pause = true;
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("desktop-refresh-policy-changed", null);
    await advance(1000);
    assert.equal(window.__DESKTOP_TOOLS_NATIVE__.isRefreshPaused(), true);
    const before = calls.filter(command => command === "get_current_component_host_event").length; // 暂停边界请求计数。
    value = 2;
    await advance(6000);
    assert.equal(calls.filter(command => command === "get_desktop_host_snapshot").length, 1, "暂停期间不重新读取宿主");
    assert.equal(calls.filter(command => command === "get_current_component_host_event").length, before);
    await window.__DESKTOP_TOOLS_NATIVE__.invoke("record_desktop_host_renderer_heartbeat");
    assert.ok(calls.includes("record_desktop_host_renderer_heartbeat"));
    value = 3;
    pause = false;
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("desktop-refresh-policy-changed", null);
    await advance(500);
    assert.equal(window.__DESKTOP_TOOLS_NATIVE__.isRefreshPaused(), false);
    await new Promise(resolve => setImmediate(resolve)); // 恢复读取在异步事件回调中完成。
    assert.deepEqual(values, [1, 3]);
    assert.deepEqual(musicValues, [1, 3]);
    assert.equal(calls.filter(command => command === "get_desktop_host_snapshot").length, 2, "恢复时读取一次最新宿主");
    assert.equal(calls.filter(command => command === "request_music_convergence").length, 1, "音乐恢复时只启动一次有限收敛");
    unlisten();
    unlistenMusic();
    const after = calls.filter(command => command === "get_current_component_host_event").length; // 取消后的边界计数。
    await advance(1000);
    assert.equal(calls.filter(command => command === "get_current_component_host_event").length, after);
});
