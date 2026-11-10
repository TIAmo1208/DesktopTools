import assert from "node:assert/strict";
import test from "node:test";
import { expectedPixelFrames, validatePixelFrame, summarizePixelFrames } from "./浮岛像素门禁.mjs";

// 合法记录只构造数据；这些测试不会导入runner、启动浏览器或读取截图。
const expected = expectedPixelFrames()[0];
const sample = (spec = expected) => ({
    name: spec.name, mode: spec.mode, form: spec.form, edge: spec.edge, dpr: spec.dpr, direction: spec.direction,
    elapsed: spec.requestedMs, actualElapsed: spec.mode === "steady" ? 240 : 4,
    filleted: spec.form === "notch" ? "true" : "false", surfaceDisplay: spec.form === "notch" ? "block" : "none",
    cardBackground: spec.form === "notch" ? "rgba(0, 0, 0, 0)" : "rgba(32, 38, 40, 0.58)",
    border: "rgba(0, 0, 0, 0)", surfaceColor: "rgba(32, 38, 40, 0.58)",
    maskSize: spec.form === "notch" ? "124px 44px, 12px 12px, 12px 12px, 148px 44px" : "auto",
    card: { x: 24, y: 24, width: 124, height: 44 }, root: { x: 12, y: 24, width: 148, height: 44 },
    background: spec.background === "dark" ? [16, 16, 16] : [255, 255, 255], scanned: true,
    maxLightWidth: 0, maxDarkAlpha: 0, maxSingleSurfaceDelta: 0, outsideBandChanged: 0, outsideNewCoverage: 0,
    minAlpha: 0.58, expectedAlpha: 0.58, steadyChangedBytes: 0, dragSteadyChangedBytes: 0, dragSteadyOutsideChanged: 0,
    rows: [{ values: [{ alpha: 0.58, rgb: [126, 129, 130] }] }],
});
const assess = (frame) => summarizePixelFrames([frame], [expected]);

test("期望集合严格枚举896动画与128稳态，筛选也产生准确期望数", () => {
    assert.equal(expectedPixelFrames().length, 896);
    assert.equal(new Set(expectedPixelFrames().map((frame) => frame.name)).size, 896);
    assert.equal(expectedPixelFrames({ steady: true }).length, 128);
    assert.equal(expectedPixelFrames({ filter: "^notch/top/1/in/8/white$" }).length, 1);
    assert.equal(summarizePixelFrames([], []).passed, false);
});
test("notch合法完整扫描记录通过", () => {
    assert.equal(validatePixelFrame(sample(), expected).valid, true);
    assert.equal(assess(sample()).passed, true);
});
test("capsule合法空态filleted=false/surface=none仍通过", () => {
    const spec = expectedPixelFrames().find((frame) => frame.form === "capsule");
    const frame = sample(spec);
    assert.equal(frame.filleted, "false");
    assert.equal(frame.surfaceDisplay, "none");
    assert.equal(summarizePixelFrames([frame], [spec]).passed, true);
});
test("notch期望帧未扫描必须失败并显示完整定位与原因", () => {
    const result = assess({ ...sample(), scanned: false, reason: "故意未扫描反例" });
    assert.equal(result.passed, false);
    assert.equal(result.unscannedCount, 1);
    assert.equal(result.scannedCount, 0);
    assert.equal(result.expectedCount, 1);
    assert.match(result.firstFailure, /mode=animation edge=top form=notch DPR=1 requestedMs=8 actualMs=4 reason=未扫描：故意未扫描反例/);
    assert.equal(result.reasonCounts["未扫描：故意未扫描反例"], 1);
});
test("notch filleted=false或surface隐藏不能伪造scanned=true过门禁", () => {
    for (const patch of [{ filleted: "false" }, { surfaceDisplay: "none" }, { cardBackground: "rgb(32,38,40)" }])
        assert.equal(assess({ ...sample(), ...patch }).passed, false);
});
test("缺失、非有限、非正及越界卡片几何不能PASS", () => {
    for (const card of [undefined, { x: 24, y: 24, width: NaN, height: 44 }, { x: Infinity, y: 24, width: 124, height: 44 }, { x: 24, y: 24, width: 0, height: 44 }, { x: 24, y: 24, width: 124, height: -1 }, { x: 24, y: 24, width: 200, height: 44 }])
        assert.equal(assess({ ...sample(), card }).passed, false);
    assert.equal(assess({ ...sample(), root: undefined }).passed, false);
});
test("缺失或不在所请求阶段的elapsed不能PASS", () => {
    for (const actualElapsed of [undefined, null, NaN, Infinity, -1, 0, 20, 999])
        assert.equal(assess({ ...sample(), actualElapsed }).passed, false);
    assert.equal(assess({ ...sample(), elapsed: undefined }).passed, false);
    const spec = expectedPixelFrames({ steady: true })[0];
    assert.equal(summarizePixelFrames([sample(spec)], [spec]).passed, true);
    assert.equal(summarizePixelFrames([{ ...sample(spec), actualElapsed: 220 }], [spec]).passed, false);
});
test("capsule requested160/actual112是合法沿用已绘制样本", () => {
    const spec = expectedPixelFrames().find((frame) => frame.name === "capsule/top/1/in/160/white");
    const result = summarizePixelFrames([{ ...sample(spec), actualElapsed: 112 }], [spec]);
    assert.equal(result.passed, true);
    assert.deepEqual(result.sampleCounts, { animation: 1, terminal: 0, reusedPaint: 1 });
});
test("notch requested236/actual240是合法终态样本而不是未扫描", () => {
    const spec = expectedPixelFrames().find((frame) => frame.name === "notch/top/1/in/236/white");
    const result = summarizePixelFrames([{ ...sample(spec), actualElapsed: 240 }], [spec]);
    assert.equal(result.passed, true);
    assert.deepEqual(result.sampleCounts, { animation: 0, terminal: 1, reusedPaint: 0 });
    assert.equal(summarizePixelFrames([{ ...sample(spec), actualElapsed: 240, maskSize: "auto" }], [spec]).passed, false);
});
test("notch requested160/actual999或非终态的未来时间必须失败", () => {
    const spec = expectedPixelFrames().find((frame) => frame.name === "notch/top/1/in/160/white");
    for (const actualElapsed of [200, 999]) assert.equal(summarizePixelFrames([{ ...sample(spec), actualElapsed }], [spec]).passed, false);
});
test("缺帧、重复帧和实际扫描数不足均不能用记录数量顶替", () => {
    const result = summarizePixelFrames([], [expected]);
    assert.equal(result.passed, false);
    assert.match(result.countReasons.join("；"), /实际扫描数0不等于期望1/);
    const specs = expectedPixelFrames().slice(0, 2);
    assert.equal(summarizePixelFrames([sample(specs[0]), sample(specs[0])], specs).passed, false);
});
test("scanned缺省、空像素集合、缺失或NaN指标都不能PASS", () => {
    for (const patch of [{ scanned: undefined }, { scanned: "true" }, { rows: [] }, { maxLightWidth: NaN }, { maxDarkAlpha: undefined }, { outsideBandChanged: Infinity }])
        assert.equal(assess({ ...sample(), ...patch }).passed, false);
});
