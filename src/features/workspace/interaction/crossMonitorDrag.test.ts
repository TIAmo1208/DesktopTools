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
        events.dispatchEvent(
            new CustomEvent("desktop-tools-drag", {
                detail: { phase: "result", session: payload.session, target: "B", drop: { fenceId: "f", beforeItemId: "next" } },
            }),
        );
    };
    (globalThis as any).__dragTestInvoke = (_command: string, args: any) => send(args);
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            if (specifier.endsWith("bridge/runtime"))
                return { url: "data:text/javascript,export const invokeRuntime=(...args)=>globalThis.__dragTestInvoke(...args)", shortCircuit: true };
            return next(specifier, context);
        },
    }); // 仅替换真实 transport 入口。
    try {
        const { resolveCrossMonitorDrop } = await import("./crossMonitorDrag.ts"); // 被测会话握手。
        const drag = {
            session: "s",
            source: "A",
            target: "B",
            pointer: { left: 20, top: 20 },
            origin: { left: 0, top: 0 },
            delta: { left: 20, top: 20 },
            kind: "items" as const,
            visuals: [],
            itemIds: ["item"],
        }; // 跨屏释放状态。
        assert.deepEqual(await resolveCrossMonitorDrop(drag), { fenceId: "f", beforeItemId: "next" });
        send = async () => {
            throw new Error("bridge failed");
        };
        await assert.rejects(resolveCrossMonitorDrop(drag), /bridge failed/);
    } finally {
        hooks.deregister();
        delete (globalThis as any).__dragTestInvoke;
        delete (globalThis as any).window;
    }
});

/** 验证相同目标只发送一次视觉树，释放清理帧和握手订阅。 */
test("跨屏静态帧复用、切屏重发、失败重试及结束取消待发送帧", async () => {
    const events = new EventTarget(); // 当前源页面。
    const frames = new Map<number, FrameRequestCallback>(); // 可确定执行的单帧调度。
    let nextFrame = 0; // 唯一调度身份。
    const sent: any[] = []; // 已经过 transport 的消息。
    let fail: (() => void) | undefined; // 首帧传输失败的受控回调。
    let failNext = false; // 仅下一次首帧传输等待失败。
    (globalThis as any).__dragTestInvoke = (_command: string, args: any) => {
        sent.push(args.payload);
        if (!failNext) return Promise.resolve();
        failNext = false;
        return new Promise<void>((_resolve, reject) => {
            fail = () => reject(new Error("transient"));
        });
    };
    globalThis.window = Object.assign(events, {
        setTimeout,
        clearTimeout,
        requestAnimationFrame: (fn: FrameRequestCallback) => {
            frames.set(++nextFrame, fn);
            return nextFrame;
        },
        cancelAnimationFrame: (id: number) => frames.delete(id),
    }) as any;
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            if (specifier.endsWith("bridge/runtime"))
                return { url: "data:text/javascript,export const invokeRuntime=(...args)=>globalThis.__dragTestInvoke(...args)", shortCircuit: true };
            return next(specifier, context);
        },
    }); // 保留正式合帧和会话清理逻辑。
    const flush = async () => {
        const callbacks = [...frames.values()];
        frames.clear();
        callbacks.forEach((fn) => fn(0));
        await Promise.resolve();
    }; // 单次浏览器帧。
    const drag = {
        session: "visual-session",
        source: "A",
        target: "B",
        pointer: { left: 20, top: 20 },
        origin: { left: 0, top: 0 },
        delta: { left: 20, top: 20 },
        kind: "fence" as const,
        visuals: [
            { title: "收集箱", content: { tag: "section", children: [{ tag: "text", text: "固定帧" }] }, bounds: { left: 0, top: 0, width: 240, height: 180 } },
        ],
        itemIds: [],
    }; // 原始完整帧不得被原地改写。
    try {
        const { publishCrossMonitorDrag, resolveCrossMonitorDrop } = await import("./crossMonitorDrag.ts");
        await publishCrossMonitorDrag(drag, "move");
        await publishCrossMonitorDrag({ ...drag, pointer: { left: 25, top: 25 } }, "move");
        assert.equal(frames.size, 1);
        await flush();
        assert.equal(sent.length, 1);
        assert.ok(sent[0].visuals[0].content);
        await publishCrossMonitorDrag(drag, "move");
        await flush();
        assert.equal(sent[1].visuals[0].content, undefined);
        await publishCrossMonitorDrag({ ...drag, target: "C" }, "move");
        await flush();
        assert.ok(sent[2].visuals[0].content);
        await publishCrossMonitorDrag(drag, "move");
        await flush();
        assert.ok(sent[3].visuals[0].content);
        await publishCrossMonitorDrag(drag, "move");
        await publishCrossMonitorDrag(drag, "end");
        assert.equal(frames.size, 0);
        assert.equal(sent.at(-1).phase, "end");
        assert.equal(sent.at(-1).visuals[0].content, undefined);
        assert.ok(drag.visuals[0].content);

        failNext = true;
        await publishCrossMonitorDrag(drag, "move");
        await flush();
        await publishCrossMonitorDrag(drag, "move");
        await flush();
        assert.equal(sent.at(-1).visuals[0].content, undefined);
        fail?.();
        await Promise.resolve();
        await publishCrossMonitorDrag(drag, "move");
        await flush();
        assert.ok(sent.at(-1).visuals[0].content);
        await publishCrossMonitorDrag(drag, "end");

        const abort = new AbortController(); // 在目标应答前撤销释放。
        const pending = resolveCrossMonitorDrop(drag, abort.signal);
        abort.abort();
        await assert.rejects(pending, { name: "AbortError" });
        events.dispatchEvent(
            new CustomEvent("desktop-tools-drag", { detail: { phase: "result", session: drag.session, target: "B", drop: { fenceId: "late" } } }),
        );
        const count = sent.length;
        await assert.rejects(resolveCrossMonitorDrop(drag, abort.signal), { name: "AbortError" });
        assert.equal(sent.length, count);
    } finally {
        const { publishCrossMonitorDrag } = await import("./crossMonitorDrag.ts");
        await publishCrossMonitorDrag(drag, "end");
        hooks.deregister();
        delete (globalThis as any).__dragTestInvoke;
        delete (globalThis as any).window;
    }
});
