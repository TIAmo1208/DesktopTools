import assert from "node:assert/strict";
import test from "node:test";
import { effectiveBarCount, resampleSpectrum } from "./spectrumBars.ts";

test("频段数按条带宽度裁剪，每段至少保留 3px", () => {
    assert.equal(effectiveBarCount(64, 236), 64); // 宽布局右栏可容纳设置的全部段数。
    assert.equal(effectiveBarCount(64, 176), 58); // 紧凑条带按 floor(176 / 3) 裁剪。
    assert.equal(effectiveBarCount(32, 96), 32); // 恰好每段 3px 时保留设置值。
    assert.equal(effectiveBarCount(16, 10), 3); // 条带只够 3 段时按宽度裁剪。
    assert.equal(effectiveBarCount(16, 2), 1); // 极窄条带仍保留一段。
});

test("按峰值将设置段数重采样为实绘段数", () => {
    const round = (values: Float32Array) => [...values].map((value) => Number(value.toFixed(3))); // 抹平 float32 存储误差。
    assert.deepEqual(round(resampleSpectrum([0, 1, 0.5, 0, 0.9, 0.2], 3)), [1, 0.5, 0.9]); // 每段取源区间峰值。
    assert.deepEqual(round(resampleSpectrum([0, 1, 0.5, 0.25], 4)), [0, 1, 0.5, 0.25]); // 段数不变时逐段保留。
    assert.deepEqual(round(resampleSpectrum([0.1, 0.8, 0.3], 1)), [0.8]); // 只画一段时取全局峰值。
});

test("全 0 电平重采样后保持全 0，供静音基线分支判断", () => {
    const silence = resampleSpectrum(new Float32Array(32), 16);
    assert.equal(silence.length, 16);
    assert.ok(silence.every((value) => value === 0));
    assert.equal(Math.max(...silence), 0);
});
