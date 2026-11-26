import assert from "node:assert/strict";
import test from "node:test";
import type { IslandStateRequest } from "../../contracts/island.ts";
import { createIslandCommitGate, ISLAND_MASK_FAIL_FALLBACK_MS, type IslandCommitGate, type IslandMaskHost, type IslandMaskScheduler } from "./islandCommitGate.ts";
import { ISLAND_NO_FRAME_TIMEOUT_MS } from "./islandMotion.ts";

/** 一份可用作重同步基准的几何：窗口盒与首卡尺寸都可区分。 */
const geometry = (left: number, width: number): IslandStateRequest => ({
    state: "expanded",
    windowBox: { left, top: 0, width, height: 900 },
    viewport: { left: 0, top: 0, width: 1920, height: 1080 },
    cards: [{ rect: { left: 0, top: 0, width, height: 200 }, radius: 12 }],
});

/**
 * 可控调度：帧与计时器都由测试推进，解除时序不依赖真实浏览器。
 * 每次收口后把微任务队列排空，等待 settle 内部的 promise 链走完。
 */
const createClock = () => {
    const frames: Array<() => void> = [];
    const timers: Array<{ at: number; callback: () => void; id: number }> = [];
    const cancelled = new Set<number>();
    let now = 0;
    let seq = 0;
    const scheduler: IslandMaskScheduler = {
        requestFrame: (callback) => {
            frames.push(callback);
            return ++seq;
        },
        cancelFrame: (handle) => cancelled.add(Number(handle)),
        setTimer: (callback, milliseconds) => {
            const id = ++seq;
            timers.push({ at: now + milliseconds, callback, id });
            return id;
        },
        clearTimer: (handle) => {
            const index = timers.findIndex((timer) => timer.id === handle);
            if (index >= 0) timers.splice(index, 1);
        },
    };
    return {
        scheduler,
        /** 推进一帧；帧回调同步执行，随后排空微任务。 */
        async frame(): Promise<void> {
            const next = frames.shift();
            next?.();
            await Promise.resolve();
            await Promise.resolve();
        },
        /** 推进时钟并触发到期计时器。 */
        async advance(milliseconds: number): Promise<void> {
            now += milliseconds;
            const due = timers.filter((timer) => timer.at <= now);
            for (const timer of due) {
                const index = timers.indexOf(timer);
                if (index >= 0) timers.splice(index, 1);
                timer.callback();
            }
            await Promise.resolve();
            await Promise.resolve();
        },
        frameCount: () => frames.length,
        timerCount: () => timers.length,
    };
};

/** 宿主桩：只记录遮蔽门问了什么、写了什么，不碰真实 DOM。 */
const createHost = () => {
    const log: string[] = [];
    let hidden = false;
    let painted: { width: number; height: number } | undefined = { width: 320, height: 200 };
    let target: IslandStateRequest | undefined;
    let current: IslandStateRequest | undefined;
    let resyncable = true;
    let alive = true;
    let resyncs = 0;
    const host: IslandMaskHost = {
        onHidden(next) {
            const wasHidden = hidden;
            hidden = next;
            log.push(next ? `hide:${wasHidden ? "already" : "visible"}` : "show");
            return wasHidden;
        },
        onRestore() {
            if (!alive) return false; // 已卸载实例：只结算租约。
            log.push("restore");
            return true;
        },
        onResync() {
            if (!resyncable) return false;
            resyncs += 1;
            log.push("resync");
            return true;
        },
        paintedCard: () => painted,
        current: () => current,
        target: () => target,
        onRevealed: (token) => log.push(`reveal:${token}`),
    };
    return {
        host,
        log,
        resyncs: () => resyncs,
        setPainted: (value: { width: number; height: number } | undefined) => {
            painted = value;
        },
        setTarget: (value: IslandStateRequest | undefined) => {
            target = value;
        },
        setCurrent: (value: IslandStateRequest | undefined) => {
            current = value;
        },
        disableResync: () => {
            resyncable = false;
        },
        setAlive: (value: boolean) => {
            alive = value;
        },
        hidden: () => hidden,
    };
};

/** 建门并挂上宿主。 */
const setup = () => {
    const clock = createClock();
    const stub = createHost();
    const gate: IslandCommitGate = createIslandCommitGate(clock.scheduler);
    gate.attach(stub.host);
    return { clock, stub, gate };
};

test("租约身份：新任务接管后旧 token 的领取、解除与提交一律无效", () => {
    const { stub, gate } = setup();
    const first = gate.begin();
    assert.equal(gate.owns(first), true);
    assert.equal(gate.state().masked, true);
    assert.deepEqual(stub.log, ["hide:visible"]);

    const stale = first;
    const current = gate.begin();
    assert.equal(gate.owns(stale), false); // 旧任务被接管。
    assert.equal(gate.state().owner, current);
    gate.settle(stale); // 旧回包不得解除新遮蔽。
    assert.equal(gate.state().masked, true);
    assert.equal(stub.log.filter((entry) => entry.startsWith("reveal")).length, 0);
});

test("begin(false) 只占租约不写可见性；结算按未遮蔽直接释放", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin(false);
    assert.equal(gate.state().masked, false);
    assert.deepEqual(stub.log, []); // 没有真正遮蔽，不碰 DOM。
    stub.setCurrent(geometry(0, 320));
    gate.settle(token, { resynced: true });
    await clock.frame();
    assert.equal(gate.state().owner, undefined); // 租约释放。
    assert.equal(stub.log.includes("resync"), false); // 未遮蔽不重画。
});

test("pending 未清空时挂起解除，回包 drain 后才揭开", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.settle(token, { resynced: true }); // 在途：只登记挂起。
    assert.equal(gate.state().pending, 1);
    await clock.frame();
    assert.equal(gate.state().masked, true);
    assert.equal(stub.log.includes("show"), false);

    gate.nativeFinished(token, true, geometry(0, 320));
    assert.equal(gate.state().pending, 0);
    gate.drain(true);
    await clock.frame();
    assert.equal(gate.state().masked, false);
    assert.equal(stub.log.filter((entry) => entry === "show").length, 1);
});

test("失败兜底：未落地时先等 800ms，到点只解除 CSS 并补收一次", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.settle(token, { failed: true });
    assert.equal(gate.state().masked, true); // 立刻解除会露出旧坐标。
    await clock.advance(ISLAND_MASK_FAIL_FALLBACK_MS - 1);
    assert.equal(gate.state().masked, true);
    await clock.advance(1);
    await clock.frame();
    assert.equal(gate.state().masked, false); // 兜底到点。
    assert.equal(stub.resyncs(), 0); // 失败路径不重画。
});

test("无帧页面：解除由超时收口，不把提交挂死", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    gate.settle(token, { resynced: true });
    assert.equal(clock.frameCount(), 1); // 请求了帧。
    await clock.advance(ISLAND_NO_FRAME_TIMEOUT_MS); // 但页面不派发帧。
    assert.equal(gate.state().masked, false);
    assert.equal(stub.hidden(), false);
});

test("解除前一致性：已绘制宽度不等于目标时只补一帧再结算", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    stub.setPainted({ width: 160, height: 200 }); // 首帧还是旧宽。
    stub.setTarget(geometry(0, 320));
    gate.settle(token, { resynced: true });
    await clock.frame();
    // 一次按实际几何、一次按目标几何：解除前必须把可见首帧拉到目标尺寸。
    assert.equal(stub.resyncs(), 2);
    assert.equal(gate.state().masked, true); // 补完这一帧才解除。
    for (let step = 0; step < 6 && gate.state().masked; step += 1) await clock.frame();
    assert.equal(gate.state().masked, false);
    assert.equal(stub.log.filter((entry) => entry.startsWith("reveal")).length, 1);
});

test("宿主没有绘制闭包时不重画，直接按现状态解除", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    stub.setPainted({ width: 160, height: 200 });
    stub.setTarget(geometry(0, 320));
    stub.disableResync();
    gate.settle(token, { resynced: true });
    await clock.frame();
    assert.equal(stub.resyncs(), 0);
    assert.equal(gate.state().masked, false); // 没有可用的重绘就不再等待。
});

test("keepOwner：揭开后保留租约给同一任务的动画继续绘制", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    gate.settle(token, { keepOwner: true, alreadySynced: true });
    await clock.frame();
    assert.equal(gate.state().masked, false);
    assert.equal(gate.owns(token), true); // 所有权还在，动画可继续提交。
    assert.equal(stub.hidden(), false);
});

test("已同步的结算不再重绘：调用方已按终态画过一帧", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    gate.settle(token, { alreadySynced: true });
    await clock.frame();
    assert.equal(stub.log.includes("resync"), false);
    assert.equal(stub.log.includes("restore"), false);
    assert.equal(gate.state().masked, false);
});

test("重新 attach 由继任宿主接管写入：在途提交与租约跨 effect 重跑保留", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.settle(token, { resynced: true }); // 在途时只能挂起。

    const next = createHost();
    gate.attach(next.host); // 继任 effect 接手观测，不动租约与在途计数。
    assert.equal(gate.state().pending, 1); // 在途提交没有因为换实例被清零。
    assert.equal(gate.owns(token), true);

    gate.nativeFinished(token, true, geometry(0, 320));
    gate.drain(true);
    for (let step = 0; step < 6 && gate.state().masked; step += 1) await clock.frame();
    // 揭开由继任实例完成：旧实例换了宿主后不再收到任何写入。
    assert.equal(gate.state().masked, false);
    assert.deepEqual(stub.log.filter((entry) => entry === "show"), []);
    assert.equal(next.log.includes("show"), true);
});

test("已卸载的实例只结算租约：不还原样式、不重画", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    stub.setAlive(false); // cleanup 之后、继任尚未 begin 时的旧回包。
    gate.settle(token, { resynced: true });
    await clock.frame();
    assert.deepEqual(stub.log.filter((entry) => entry === "restore" || entry === "resync"), []);
    assert.equal(gate.state().masked, false); // 租约照常结算，界面不会永久隐藏。
});

test("pending 期间再次 begin：旧 token 的挂起不会揭开新任务", async () => {
    const { clock, stub, gate } = setup();
    const first = gate.begin();
    gate.nativeStarted();
    gate.settle(first, { resynced: true }); // first 挂起。
    const second = gate.begin();
    gate.nativeFinished(first, true, geometry(0, 320));
    gate.drain(true); // first 的挂起到点，但它已不是所有者。
    await clock.frame();
    assert.equal(gate.state().masked, true); // 新任务仍然遮蔽。
    assert.equal(gate.owns(second), true);
    assert.deepEqual(stub.log.filter((entry) => entry === "show"), []);
});

test("成功结算清掉失败兜底计时器：同一 token 不会二次揭开", async () => {
    const { clock, stub, gate } = setup();
    const token = gate.begin();
    gate.settle(token, { failed: true }); // 先排一次兜底。
    assert.equal(clock.timerCount(), 1); // 只有 800ms 兜底。
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    gate.settle(token, { resynced: true }); // 成功路径必须清掉兜底。
    // 只剩解除用的帧超时；兜底若还在这里会是 2。
    assert.equal(clock.timerCount(), 1);
    for (let step = 0; step < 6 && gate.state().masked; step += 1) await clock.frame();
    assert.equal(gate.state().masked, false);
    await clock.advance(ISLAND_MASK_FAIL_FALLBACK_MS);
    assert.equal(stub.log.filter((entry) => entry === "show").length, 1); // 只揭开一次。
});

test("反复遮蔽：揭开后再次遮蔽要重新等一次绘制", async () => {
    const { clock, stub, gate } = setup();
    const first = gate.begin();
    gate.nativeStarted();
    gate.nativeFinished(first, true, geometry(0, 320));
    gate.settle(first, { resynced: true });
    for (let step = 0; step < 6 && gate.state().masked; step += 1) await clock.frame();
    assert.equal(stub.hidden(), false); // 已经回到可见。

    gate.begin(); // 从可见重新遮蔽。
    assert.equal(gate.state().needsPaint, true); // 必须重新等一帧，不能沿用上一轮的已绘制标记。
    gate.markPainted();
    assert.equal(gate.state().needsPaint, false);
    gate.begin(); // 仍在隐藏态：不重复等。
    assert.equal(gate.state().needsPaint, false);
});

test("attach 之前结算不抛错，也不触碰任何宿主", async () => {
    const clock = createClock();
    const gate: IslandCommitGate = createIslandCommitGate(clock.scheduler);
    const token = gate.begin();
    gate.settle(token); // 没有宿主：只走内部状态。
    assert.equal(gate.owns(token), true);
    assert.equal(gate.state().pending, 0);
});

test("markPainted：只有真正从可见变隐藏才需要重等一次绘制", () => {
    const { gate, stub } = setup();
    gate.begin();
    assert.equal(gate.state().needsPaint, true); // 新遮蔽要等一帧。
    gate.markPainted();
    assert.equal(gate.state().needsPaint, false);
    gate.begin(); // 已处于隐藏态：不重复等。
    assert.equal(gate.state().needsPaint, false);
    assert.equal(stub.log.filter((entry) => entry === "hide:already").length, 1);
});

test("就绪与形变两个事实由门统一持有：领租约清就绪、揭开置就绪，形变自成一对", async () => {
    const { clock, gate } = setup();
    assert.equal(gate.state().settled, false, "冷启动尚未落地");
    const token = gate.begin();
    assert.equal(gate.state().settled, false, "领租约即未就绪");
    gate.nativeStarted();
    gate.nativeFinished(token, true, geometry(0, 320));
    gate.settle(token, { resynced: true });
    for (let step = 0; step < 6 && gate.state().masked; step += 1) await clock.frame();
    assert.equal(gate.state().settled, true, "揭开即恢复交互");

    gate.beginTransition();
    assert.equal(gate.state().transitioning, true, "形变在途");
    gate.endTransition();
    assert.equal(gate.state().transitioning, false, "终帧结算后放开");

    gate.markUnsettled();
    assert.equal(gate.state().settled, false, "换边清几何时显式置未就绪");
    gate.markSettled();
    assert.equal(gate.state().settled, true);
});
