import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";

import { OVERFLOW_LOOP_MIN_MS, overflowDistance, overflowLoopMs, overflowScrollKeyframes, overflowScrollPlan } from "./textScroll.ts";

// 只有内容真的超出可视宽度才需要滚动。
test("overflow_distance_requires_real_overflow", () => {
    assert.equal(overflowDistance(120, 100), 20);
    assert.equal(overflowDistance(100, 100), 0);
    assert.equal(overflowDistance(80, 100), 0);
});

// 亚像素差异按整数收边，避免反复在 0 与 1 之间抖动。
test("overflow_distance_rounds_subpixels", () => {
    assert.equal(overflowDistance(100.4, 100), 0);
    assert.equal(overflowDistance(100.6, 100), 1);
});

// 无效尺寸视为不溢出，不产生动画。
test("overflow_distance_rejects_invalid_sizes", () => {
    assert.equal(overflowDistance(Number.NaN, 100), 0);
    assert.equal(overflowDistance(120, Number.POSITIVE_INFINITY), 0);
    assert.equal(overflowDistance(120, 0), 0);
    assert.equal(overflowDistance(-120, 100), 0);
});

// 周期时长按距离增长，最短保持下限。
test("overflow_loop_duration_follows_distance", () => {
    assert.equal(overflowLoopMs(10), OVERFLOW_LOOP_MIN_MS);
    assert.equal(overflowLoopMs(100), 11200);
    assert.equal(overflowLoopMs(0), 0);
    assert.equal(overflowLoopMs(Number.NaN), 0);
});

// 单程速度固定 20 DIP/s，往返位移为 2×距离，停顿落在关键帧区间。
test("overflow_scroll_plan_matches_speed_and_distance", () => {
    const plan = overflowScrollPlan(120);
    assert.ok(plan);
    assert.equal(plan.distance, 120);
    assert.equal(plan.travelMs, 6000); // 120 DIP ÷ 20 DIP/s。
    assert.equal(plan.holdMs, 600);
    assert.equal(plan.cycleMs, 13200); // 2×6000 + 2×600。
    for (const distance of [10, 60, 100, 333]) {
        const item = overflowScrollPlan(distance);
        assert.ok(item);
        assert.equal(Math.round((item.distance / item.travelMs) * 1000), 20); // 滑动段严格 20 DIP/s。
        assert.ok(item.cycleMs >= OVERFLOW_LOOP_MIN_MS);
    }
    assert.equal(overflowScrollPlan(0), undefined);
    assert.equal(overflowScrollPlan(Number.NaN), undefined);
    const frames = overflowScrollKeyframes(plan);
    assert.equal(Math.max(...frames.map((frame) => Math.abs(frame.x))), 120); // 最大位移就是实测距离。
    assert.deepEqual(frames.map((frame) => frame.offset), [0, 600 / 13200, 6600 / 13200, 7200 / 13200, 1]); // 停顿后滑动，再停顿后回程。
    for (let index = 1; index < frames.length; index += 1) assert.ok(frames[index].offset >= frames[index - 1].offset);
});

// 关闭动效或文本变短时组件必须清空计划、取消动画并复位，而不是沿用旧计划。
test("scrolling_text_clears_plan_and_resets_animation", () => {
    const source = readFileSync(new URL("./ScrollingText.tsx", import.meta.url), "utf8");
    assert.match(source, /setPlan\(\(current\) => \{/); // 用函数式状态而不是 last 缓存早退。
    assert.doesNotMatch(source, /last = 0/);
    assert.match(source, /animation\.current\?\.cancel\(\);/);
    assert.match(source, /style\.transform = ""/); // 停止时 translateX 归零。
    assert.match(source, /typeof matchMedia === "function" && matchMedia\("\(prefers-reduced-motion: reduce\)"\)/);
    assert.match(source, /typeof trackElement\.animate !== "function"/); // 无动画能力时保持省略号。
});
