import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

/** 用桥接替身验证释放握手，不依赖真实 Shell 文件或窗口。 */
test("cross monitor drop waits for matching target and propagates bridge failure", async () => {
    const events = new EventTarget(); // 当前源页事件通道。
    globalThis.window = Object.assign(events, { setTimeout, clearTimeout }) as unknown as Window & typeof globalThis;
    let send: (args: any) => Promise<void> = async (args) => {
        const payload = args.payload; // 实際发送的释放消息。
        assert.equal(payload.phase, "drop");
        events.dispatchEvent(new CustomEvent("desktop-tools-drag", { detail: { phase: "result", session: "wrong", target: "B", drop: { fenceId: "wrong" } } }));
        events.dispatchEvent(new CustomEvent("desktop-tools-drag", { detail: { phase: "result", session: payload.session, target: "B", drop: { fenceId: "f", beforeItemId: "next" } } }));
    };
    (globalThis as any).__dragTestInvoke = (_command: string, args: any) => send(args);
    const hooks = registerHooks({ resolve(specifier, context, next) {
        if (specifier.endsWith("bridge/runtime")) return { url: "data:text/javascript,export const invokeRuntime=(...args)=>globalThis.__dragTestInvoke(...args)", shortCircuit: true };
        return next(specifier, context);
    } }); // 仅替换真实 transport 入口。
    try {
        const { resolveCrossMonitorDrop } = await import("./crossMonitorDrag.ts"); // 被测会话握手。
        const drag = { session: "s", source: "A", target: "B", pointer: { left: 20, top: 20 }, origin: { left: 0, top: 0 }, delta: { left: 20, top: 20 }, kind: "items" as const, visuals: [], itemIds: ["item"] }; // 跨屏释放状态。
        assert.deepEqual(await resolveCrossMonitorDrop(drag), { fenceId: "f", beforeItemId: "next" });
        send = async () => { throw new Error("bridge failed"); };
        await assert.rejects(resolveCrossMonitorDrop(drag), /bridge failed/);
    } finally { hooks.deregister(); delete (globalThis as any).__dragTestInvoke; delete (globalThis as any).window; }
});
