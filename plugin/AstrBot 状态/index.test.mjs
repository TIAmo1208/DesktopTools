import assert from "node:assert/strict";
import test from "node:test";

import { createCapabilityProbe, isStale, pollDelay, probeFromError, readyLabel, startProbeLoop, viewOf } from "./index.js";

// 模拟宿主页面桥：记录命令名与参数对象，按 fetch body 语义原样接收。
const fakeBridge = () => {
    const calls = []; // 每次桥调用的命令名与参数。
    return {
        calls,
        invoke(command, args) {
            calls.push({ command, args });
            return Promise.resolve({ instanceId: args.instanceId, capability: args.capability, payload: { level: "ok", startTime: 1_793_365_017, checkedAt: 1_793_372_400 } });
        },
    };
};

// 真实调用链：插件必须用命令名 + 对象参数调用能力通道，参数能被逐字段还原。
test("能力探测按命令名与对象参数调用页面桥", async () => {
    const bridge = fakeBridge();
    const invoke = createCapabilityProbe(bridge, "instance-7");
    const result = await invoke();
    assert.equal(bridge.calls.length, 1);
    const [call] = bridge.calls;
    assert.equal(call.command, "invoke_component_capability");
    assert.deepEqual(call.args, { instanceId: "instance-7", capability: "probe-service-status" });
    assert.equal(result.payload.level, "ok");
    // 参数必须是可 JSON 序列化的纯对象：桥以 body: JSON.stringify(args) 发送。
    assert.deepEqual(JSON.parse(JSON.stringify(call.args)), { instanceId: "instance-7", capability: "probe-service-status" });
});

// 插件不得调用任何平铺原生命令，只走能力通道这一个命令名。
test("能力探测只发出 invoke_component_capability", async () => {
    const bridge = fakeBridge();
    await createCapabilityProbe(bridge, "instance-7")();
    assert.deepEqual([...new Set(bridge.calls.map((call) => call.command))], ["invoke_component_capability"]);
});

const NOW = new Date("2026-10-31T12:00:00+08:00").getTime(); // 统一的时间基准，避免测试依赖当前时钟。
const nowSeconds = Math.floor(NOW / 1000);

/** 生成一档探活结论。 */
const probe = (level, startTime = null, checkedAt = nowSeconds) => ({ level, startTime, checkedAt });

// 四档映射：在线绿带就绪时长，离线灰、探测被拒黄、未知灰且都不显示时长。
test("四档结论映射到文案与状态点", () => {
    const ok = viewOf(probe("ok", nowSeconds - 125), NOW);
    assert.equal(ok.text, "AstrBot 在线 · 已就绪 2 分");
    assert.equal(ok.levelClass, "astrbot-status__level--ok");

    const off = viewOf(probe("off"), NOW);
    assert.equal(off.text, "离线");
    assert.equal(off.levelClass, "astrbot-status__level--off");

    const warn = viewOf(probe("warn"), NOW);
    assert.equal(warn.text, "探测被拒");
    assert.equal(warn.levelClass, "astrbot-status__level--warn");

    const unknown = viewOf(probe("unknown"), NOW);
    assert.equal(unknown.text, "未知");
    assert.equal(unknown.levelClass, "astrbot-status__level--unknown");
});

// 超时回来的 unknown 不能被渲染成在线或离线。
test("未知档不上绿色也不写成离线", () => {
    const unknown = viewOf(probe("unknown"), NOW);
    assert.notEqual(unknown.levelClass, "astrbot-status__level--ok");
    assert.doesNotMatch(unknown.text, /在线|离线|已就绪/);
});

// 过期判定以 checkedAt 为准，恰好 60s 仍新鲜，超过 1ms 即过期并变灰。
test("checkedAt 距今超过 60s 判定为数据过期", () => {
    assert.equal(isStale(nowSeconds, NOW), false);
    assert.equal(isStale(nowSeconds - 60, NOW), false);
    assert.equal(isStale(nowSeconds - 61, NOW), true);

    const stale = viewOf(probe("ok", nowSeconds - 300, nowSeconds - 61), NOW);
    assert.equal(stale.text, "AstrBot 在线 · 已就绪 5 分");
    assert.equal(stale.levelClass, "astrbot-status__level--unknown");
});

// 就绪时长每次用当前时间现算，不来自任何 uptimeSec 字段。
test("就绪时长按 startTime 现算", () => {
    assert.equal(readyLabel(nowSeconds, NOW), "刚刚就绪");
    assert.equal(readyLabel(nowSeconds - 59, NOW), "刚刚就绪");
    assert.equal(readyLabel(nowSeconds - 60, NOW), "已就绪 1 分");
    assert.equal(readyLabel(nowSeconds - 3_600, NOW), "已就绪 1 小时 0 分");
    assert.equal(readyLabel(nowSeconds - 90_000, NOW), "已就绪 1 天 1 小时");
    assert.equal(readyLabel(nowSeconds + 5, NOW), "刚刚就绪");
});

// 稳定 15s，连续失败 3 次退避到 60s。
test("失败计数驱动轮询退避", () => {
    assert.equal(pollDelay(0), 15_000);
    assert.equal(pollDelay(1), 15_000);
    assert.equal(pollDelay(2), 15_000);
    assert.equal(pollDelay(3), 60_000);
    assert.equal(pollDelay(9), 60_000);
});

// 命令被拒或实例未就绪一律记成 unknown，不等于离线。
test("探活失败记为未知档", () => {
    const failed = probeFromError(new Error("capability_denied"), NOW);
    assert.equal(failed.level, "unknown");
    assert.equal(failed.startTime, null);
    assert.equal(failed.checkedAt, nowSeconds);
});

/**
 * 可控的假定时器：按全局 setTimeout/clearTimeout 换实现，记录排期间隔与清理。
 * 只有间隔 0 且被测代码直接创建的任务参与驱动，其它定时器一律忽略。
 */
const useFakeTimers = () => {
    const real = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout };
    const delays = []; // 每次排期的间隔。
    const cleared = []; // 被清掉的定时器。
    const pending = new Map(); // 尚未触发的任务。
    let nextId = 1; // 定时器身份。
    globalThis.setTimeout = (callback, delay) => {
        delays.push(delay);
        pending.set(nextId, callback);
        return nextId++;
    };
    globalThis.clearTimeout = (id) => {
        cleared.push(id);
        pending.delete(id);
    };
    return {
        delays,
        cleared,
        /** 触发最早一个待处理任务，再等微任务跑完，返回是否真的触发了。 */
        async settle() {
            const [id, callback] = pending.entries().next().value ?? [];
            if (!callback) return false;
            pending.delete(id);
            callback();
            await new Promise((resolve) => real.setTimeout(resolve, 10));
            return true;
        },
        restore() {
            globalThis.setTimeout = real.setTimeout;
            globalThis.clearTimeout = real.clearTimeout;
        },
    };
};

// 连续失败 3 次退避到 60s，拿到结论后立即复位到 15s。
test("探测循环按失败次数退避并在成功后复位", async () => {
    const outcomes = [new Error("超时"), new Error("超时"), new Error("超时"), probe("off"), new Error("超时"), probe("ok", nowSeconds)];
    const queue = outcomes.slice(); // 依次给出成功或失败。
    const probes = []; // 交给展示层的结论。
    const failures = []; // 交给展示层的失败。
    const timers = useFakeTimers();
    const invoke = () => {
        const next = queue.shift();
        return next instanceof Error ? Promise.reject(next) : Promise.resolve({ payload: next });
    };
    const stop = startProbeLoop(
        invoke,
        (payload) => probes.push(payload),
        (error) => failures.push(error),
    );
    try {
        for (let index = 0; index < outcomes.length; index += 1) assert.equal(await timers.settle(), true);
        assert.equal(failures.length, 4);
        assert.deepEqual(
            probes.map((payload) => payload.level),
            ["off", "ok"],
        );
        // 前两次失败仍排 15s，第 3 次起排 60s，拿到 off 与 ok 各复位一次。
        assert.deepEqual(timers.delays.slice(0, 6), [0, 15_000, 15_000, 60_000, 15_000, 15_000]);
    } finally {
        stop();
        timers.restore();
    }
});

// 卸载后清掉未触发的定时器，迟到的回调不再探测。
test("停止探测循环后不残留定时器", async () => {
    const probes = []; // 卸载后不应再有结论。
    const timers = useFakeTimers();
    const stop = startProbeLoop(
        () => Promise.resolve({ payload: probe("ok", nowSeconds) }),
        (payload) => probes.push(payload),
        () => undefined,
    );
    try {
        stop();
        assert.deepEqual(timers.cleared, [1]);
        assert.equal(probes.length, 0);
        assert.equal(await timers.settle(), false);
    } finally {
        timers.restore();
    }
});
