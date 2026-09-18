import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { stripTypeScriptTypes } from "node:module";

/** 在受控传输与计时器下执行实际门户桥接，不复制生命周期实现。 */
const portalHarness = () => {
    const source = readFileSync(new URL("./folderPortal.ts", import.meta.url), "utf8")
        .replace(/^import .*;\r?$/gm, "")
        .replace(/^export /gm, ""); // 实际产品模块，仅替换外部边界。
    const timers = new Map<number, { callback: () => void; delay: number }>(); // 可手动推进的重连计时器。
    const handlers = new Map<string, () => void>(); // 暂停与恢复通知。
    const reads: Array<{ path: string; finish: (value: unknown) => void }> = []; // 可乱序完成的目录读取。
    const stops: number[] = []; // 后端释放记录。
    const events: unknown[] = []; // 已交付快照。
    const errors: unknown[] = []; // 可观察错误。
    let paused = false; // 当前宿主状态。
    let fail = false; // 模拟目录暂时不可用。
    let next = 0; // 后端监听身份。
    let timerId = 0; // 调度身份。
    let listener: ((event: unknown) => void) | undefined; // 当前事件接收入口。
    const context: any = {
        Promise,
        Error,
        window: {
            __DESKTOP_TOOLS_NATIVE__: { isRefreshPaused: () => paused },
            addEventListener: (name: string, handler: () => void) => handlers.set(name, handler),
            removeEventListener: (name: string) => handlers.delete(name),
        },
        setTimeout: (callback: () => void, delay: number) => {
            timers.set(++timerId, { callback, delay });
            return timerId;
        },
        clearTimeout: (id: number) => timers.delete(id),
        parseFolderPortalSnapshot: (value: unknown) => value,
        parseApplicationCommandError: (value: unknown) => value,
        listenRuntime: async (_: string, callback: (event: unknown) => void) => {
            listener = callback;
            return () => {
                if (listener === callback) listener = undefined;
            };
        },
        invokeRuntime: async (command: string, args: any) => {
            if (command === "watch_folder_portal") {
                assert.ok(listener, "后端监听建立前必须已有事件接收入口");
                if (fail) throw new Error("目录不可用");
                return ++next;
            }
            if (command === "unwatch_folder_portal") {
                stops.push(args.subscription);
                return;
            }
            return new Promise((finish) => reads.push({ path: args.currentPath, finish }));
        },
    }; // 控制外部边界，生命周期代码保持原样。
    runInNewContext(stripTypeScriptTypes(source) + "\nglobalThis.watch = watchFolderPortal;", context);
    return {
        reads,
        events,
        errors,
        timers,
        stops,
        start: (path = "root", previous?: Promise<void>) =>
            context.watch(
                "root",
                path,
                "DISPLAY1",
                (value: unknown) => events.push(value),
                (error: unknown) => errors.push(error),
                previous,
            ),
        emit: (error?: string) => listener?.({ subscription: next, error }),
        pause: (value: boolean) => {
            paused = value;
            handlers.get("desktop-refresh-changed")?.();
        },
        fail: (value: boolean) => {
            fail = value;
        },
        settle: () => new Promise<void>((resolve) => setImmediate(resolve)),
    };
};

test("门户合并突发变化，暂停丢弃旧读取，恢复补读且退订拒绝迟到结果", async () => {
    const h = portalHarness(); // 可控真实桥接。
    const watch = h.start(); // 当前目录生命周期。
    await h.settle();
    assert.equal(h.reads.length, 1);
    h.emit();
    h.emit();
    h.emit();
    h.reads[0].finish({ value: "旧值" });
    await h.settle();
    assert.equal(h.events.length, 0);
    assert.equal(h.reads.length, 2);
    h.pause(true);
    h.reads[1].finish({ value: "暂停前" });
    await h.settle();
    h.pause(false);
    await h.settle();
    assert.equal(h.reads.length, 3);
    h.reads[2].finish({ value: "恢复值" });
    await h.settle();
    assert.deepEqual(h.events, [{ value: "恢复值" }]);
    h.emit();
    await h.settle();
    watch.stop();
    h.reads[3].finish({ value: "卸载后" });
    await h.settle();
    assert.equal(h.events.length, 1);
    assert.deepEqual(h.stops, [1]);
});

test("门户失败仅退避重连，暂停停止重试，恢复重新建立监听", async () => {
    const h = portalHarness(); // 模拟启动失败与运行中失效。
    h.fail(true);
    const watch = h.start(); // 首次订阅失败。
    await h.settle();
    assert.equal([...h.timers.values()][0].delay, 1000);
    h.reads[0].finish({ accessible: false });
    await h.settle();
    h.pause(true);
    assert.equal(h.timers.size, 0);
    h.fail(false);
    h.pause(false);
    await h.settle();
    h.reads[1].finish({ accessible: true });
    await h.settle();
    assert.equal(h.timers.size, 0);
    h.pause(true);
    h.emit("监听终止");
    assert.equal(h.timers.size, 0);
    h.pause(false);
    await h.settle();
    assert.deepEqual(h.stops, [1]);
    h.reads[2].finish({ accessible: true });
    await h.settle();
    watch.stop();
    assert.deepEqual(h.stops, [1, 2]);
});

test("门户导航等待旧枚举结束，旧结果不写入新目录", async () => {
    const h = portalHarness(); // 连续目录生命周期共用串行读取尾部。
    const old = h.start("old"); // 旧目录。
    await h.settle();
    old.stop();
    const next = h.start("new", old.drained()); // 新目录等待旧请求结束。
    await h.settle();
    assert.equal(h.reads.length, 1);
    h.reads[0].finish({ path: "old" });
    await h.settle();
    assert.equal(h.reads[1].path, "new");
    h.reads[1].finish({ path: "new" });
    await h.settle();
    assert.deepEqual(h.events, [{ path: "new" }]);
    next.stop();
});

test("原生宿主通过工作区事件交付门户，取消订阅停止交付", async () => {
    const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8"); // 实际注入宿主的脚本来源。
    const script = source.split('const NATIVE_BRIDGE_SCRIPT: &str = r#"')[1].split('"#;')[0].replace("__REFRESH_POLICY_FALLBACK__", "false"); // 运行产品代码并代入创建宿主时确定的故障兜底状态。
    const pending: Array<() => Promise<void>> = []; // 可控的下一次轮询。
    const calls: string[] = []; // 实际读取命令。
    const received: unknown[] = []; // 订阅者收到的目录。
    const window: any = { setTimeout: (callback: () => Promise<void>) => pending.push(callback), clearTimeout: () => {}, addEventListener: () => {} }; // 独立宿主全局对象。
    runInNewContext(script, {
        window,
        __DESKTOP_NATIVE_ORIGIN__: "http://host.localhost",
        fetch: async (url: string) => {
            calls.push(url);
            return { json: async () => ({ ok: true, result: ["C:\\资料", "C:\\资料"] }) };
        },
    });
    const unlisten = await window.__DESKTOP_TOOLS_NATIVE__.listen("workspace-snapshot-changed", (value: unknown) => received.push(value)); // 门户随工作区统一发布。
    assert.equal(pending.length, 0);
    assert.deepEqual(received, [["C:\\资料", "C:\\资料"]]);
    assert.equal(calls[0], "http://host.localhost/__desktop_tools_invoke/get_current_workspace_snapshot");
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("workspace-snapshot-changed", ["C:\\新目录"]);
    assert.equal(received.length, 2);
    assert.equal(calls.length, 1);
    unlisten();
    window.__DESKTOP_TOOLS_NATIVE__.dispatch("workspace-snapshot-changed", []);
    assert.equal(received.length, 2);
    assert.equal(pending.length, 0);
});
