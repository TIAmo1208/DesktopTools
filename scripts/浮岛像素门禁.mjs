// 浮岛像素验收的纯判定：不读取文件、不连接浏览器，供runner与负面测试共用。
export function expectedPixelFrames({ steady = false, filter = "" } = {}) {
    const frames = [];
    const pattern = filter ? new RegExp(filter) : null;
    for (const form of ["notch", "capsule"]) for (const edge of ["top", "bottom", "left", "right"])
        for (const dpr of [1, 1.25, 1.5, 1.75]) for (const direction of ["in", "out"])
            for (const requestedMs of steady ? [300] : [8, 16, 48, 96, 160, 232, 236]) for (const background of ["white", "dark"]) {
                const name = `${form}/${edge}/${dpr}/${direction}/${requestedMs}/${background}`;
                if (!pattern || pattern.test(name)) frames.push({ name, mode: steady ? "steady" : "animation", form, edge, dpr, direction, requestedMs, background });
            }
    return frames;
}

// 只检查可扫描前提，不把capsule合法无补面态误判为notch缺失；扫描结果由汇总函数另验。
export function validatePixelFrame(frame, expected) {
    const reasons = [];
    if (!frame || typeof frame !== "object") return { valid: false, reasons: ["帧数据缺失"] };
    for (const key of ["mode", "form", "edge", "direction"]) if (frame[key] !== expected[key]) reasons.push(`${key}与期望不符`);
    if (!Number.isFinite(frame.dpr) || Math.abs(frame.dpr - expected.dpr) > 1e-6) reasons.push("DPR缺失或与期望不符");
    if (!Number.isFinite(frame.elapsed) || frame.elapsed !== expected.requestedMs) reasons.push("requested elapsed缺失或与期望不符");
    // 几何去重可能保留较早的paint；只验证已绘制时间合法，不虚构固定的requested-actual帧距。
    const terminal = Number.isFinite(frame.actualElapsed) && Math.abs(frame.actualElapsed - 240) <= 0.001;
    if (!Number.isFinite(frame.actualElapsed) || frame.actualElapsed <= 0 || frame.actualElapsed > 240.001) reasons.push("actual elapsed缺失或非法");
    else if (expected.mode === "steady" ? !terminal : !terminal && frame.actualElapsed > expected.requestedMs + 0.001)
        reasons.push("actual elapsed不在期望采样阶段（240ms终态或请求时间以内的已绘制中帧）");
    const transparent = (color) => typeof color === "string" && (color === "transparent" || /^rgba\([^,]+,[^,]+,[^,]+,0(?:\.0*)?\)$/.test(color.replace(/\s+/g, "")));
    if (expected.form === "notch") {
        if (frame.filleted !== "true") reasons.push("notch期望filleted=true");
        if (frame.surfaceDisplay !== "block") reasons.push("notch期望surface=block");
        if (terminal && (typeof frame.maskSize !== "string" || frame.maskSize.split(",").length !== 4)) reasons.push("notch终态样本必须保持4层补面mask");
        if (!transparent(frame.cardBackground)) reasons.push("notch卡片背景必须透明");
        if (!transparent(frame.border)) reasons.push("notch补面帧描边必须透明");
        if (typeof frame.surfaceColor !== "string" || !frame.surfaceColor || transparent(frame.surfaceColor)) reasons.push("notch表面色缺失或透明");
    } else {
        if (frame.filleted !== "false" || frame.surfaceDisplay !== "none") reasons.push("capsule期望合法空补面态（filleted=false/surface=none）");
        if (typeof frame.cardBackground !== "string" || !frame.cardBackground || transparent(frame.cardBackground)) reasons.push("capsule卡片应独立承载表面");
    }
    const validRect = (rect) => rect && [rect.x, rect.y, rect.width, rect.height, rect.x + rect.width, rect.y + rect.height].every(Number.isFinite) && rect.width > 0 && rect.height > 0;
    if (!validRect(frame.card)) reasons.push("card几何缺失/非有限或宽高非法");
    if (!validRect(frame.root)) reasons.push("root几何缺失/非有限或宽高非法");
    if (validRect(frame.card) && validRect(frame.root) && (frame.card.x < frame.root.x - 0.1 || frame.card.y < frame.root.y - 0.1 || frame.card.x + frame.card.width > frame.root.x + frame.root.width + 0.1 || frame.card.y + frame.card.height > frame.root.y + frame.root.height + 0.1)) reasons.push("card超出root有效范围");
    const color = expected.background === "dark" ? 16 : 255;
    if (!Array.isArray(frame.background) || frame.background.length !== 3 || !frame.background.every((channel) => channel === color)) reasons.push("背景颜色缺失或与期望不符");
    return { valid: reasons.length === 0, reasons, phase: terminal ? "terminal" : "animation", reusedPaint: !terminal && frame.actualElapsed < expected.requestedMs - 0.001 };
}

// 不接受scanned缺省/false，不用记录数量冒充扫描数量，也不能靠重复记录填满期望集合。
export function summarizePixelFrames(records, expectedFrames) {
    const expectedMap = new Map(expectedFrames.map((frame) => [frame.name, frame]));
    const seen = new Set();
    const failures = [];
    const scannedNames = new Set();
    const sampleCounts = { animation: 0, terminal: 0, reusedPaint: 0 };
    let scannedRecordCount = 0;
    const add = (frame, expected, reasons) => { if (reasons.length) failures.push({ frame, expected, reasons }); };
    if (!expectedFrames.length) add(null, null, ["期望集合为空"]);
    for (const frame of records) {
        const expected = expectedMap.get(frame?.name);
        if (frame?.scanned === true) scannedRecordCount++;
        if (!expected) { add(frame, null, ["记录不属于期望集合"]); continue; }
        const validity = validatePixelFrame(frame, expected);
        const reasons = validity.reasons;
        if (seen.has(frame.name)) reasons.push("期望帧记录重复");
        seen.add(frame.name);
        if (frame.scanned !== true) reasons.push(`未扫描：${frame.reason || "未提供原因"}`);
        else {
            const metrics = ["maxLightWidth", "maxDarkAlpha", "maxSingleSurfaceDelta", "outsideBandChanged", "outsideNewCoverage", "minAlpha", "expectedAlpha"];
            for (const key of metrics) if (!Number.isFinite(frame[key])) reasons.push(`扫描指标${key}缺失或非法`);
            const validRows = Array.isArray(frame.rows) && frame.rows.length > 0 && frame.rows.every((row) => row && Array.isArray(row.values) && row.values.length > 0 && row.values.every((sample) => sample && Number.isFinite(sample.alpha) && Array.isArray(sample.rgb) && sample.rgb.length === 3 && sample.rgb.every((channel) => Number.isFinite(channel) && channel >= 0 && channel <= 255)));
            if (!validRows) reasons.push("像素采样集合为空或非法");
            const steadyMetrics = ["steadyChangedBytes", "dragSteadyChangedBytes", "dragSteadyOutsideChanged"];
            if (validity.valid && validRows && metrics.every((key) => Number.isFinite(frame[key])) && (expected.mode !== "steady" || steadyMetrics.every((key) => Number.isFinite(frame[key]))) && !scannedNames.has(frame.name)) {
                scannedNames.add(frame.name);
                sampleCounts[validity.phase]++;
                if (validity.reusedPaint) sampleCounts.reusedPaint++; // 动画样本的子集，不与动画数相加。
            }
            if (frame.maxLightWidth > 0 || frame.maxDarkAlpha > 0.06 || frame.maxSingleSurfaceDelta > 2) reasons.push("接缝亮缝/暗带或单层RGB差超限");
            if (frame.outsideBandChanged > 0 || frame.outsideNewCoverage > 0) reasons.push("接缝带外差异或新增覆盖");
            if (expected.mode === "steady") {
                for (const key of ["steadyChangedBytes", "dragSteadyChangedBytes", "dragSteadyOutsideChanged"]) if (!Number.isFinite(frame[key])) reasons.push(`稳态指标${key}缺失或非法`);
                if (frame.dragSteadyOutsideChanged > 0 || expected.form === "capsule" && (frame.steadyChangedBytes > 0 || frame.dragSteadyChangedBytes > 0)) reasons.push("稳态/拖动对照超限");
            }
        }
        add(frame, expected, reasons);
    }
    for (const expected of expectedFrames) if (!seen.has(expected.name)) add(null, expected, ["期望帧缺失"]);
    const scannedCount = scannedNames.size;
    const unscannedCount = expectedFrames.length - scannedCount;
    const countReasons = [];
    if (records.length !== expectedFrames.length) countReasons.push(`记录数${records.length}不等于期望${expectedFrames.length}`);
    if (scannedCount !== expectedFrames.length) countReasons.push(`实际扫描数${scannedCount}不等于期望${expectedFrames.length}`);
    const reasonCounts = {};
    for (const failure of failures) for (const reason of failure.reasons) reasonCounts[reason] = (reasonCounts[reason] || 0) + 1;
    const first = failures[0];
    const context = first?.expected || first?.frame || {};
    const firstFailure = first ? `FAIL mode=${context.mode ?? "?"} edge=${context.edge ?? "?"} form=${context.form ?? "?"} DPR=${context.dpr ?? "?"} requestedMs=${context.requestedMs ?? first.frame?.elapsed ?? "?"} actualMs=${first.frame?.actualElapsed ?? "未记录"} reason=${first.reasons.join("；")}` : countReasons.length ? `FAIL ${countReasons.join("；")}` : "";
    return { passed: !failures.length && !countReasons.length, expectedCount: expectedFrames.length, recordCount: records.length, scannedCount, scannedRecordCount, unscannedCount, sampleCounts, failures, reasonCounts, countReasons, firstFailure };
}
