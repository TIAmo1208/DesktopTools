import assert from "node:assert/strict";
import { test } from "node:test";

import { OVERFLOW_LOOP_MIN_MS, overflowDistance, overflowLoopMs } from "./textScroll.ts";

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
    assert.equal(overflowLoopMs(100), 10000);
    assert.equal(overflowLoopMs(0), 0);
    assert.equal(overflowLoopMs(Number.NaN), 0);
});
