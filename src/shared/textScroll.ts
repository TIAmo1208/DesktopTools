/**
 * 单行文本溢出滚动的公共节奏：浮岛卡片与桌面音乐格子共用同一套距离与时长换算。
 *
 * 偏移只用 transform 推进，不写布局；窗口尺寸与浮岛几何都不参与，避免滚动引起重新测量。
 */

/** 滚动速度：20 DIP/s，慢速可读。 */
export const OVERFLOW_SPEED_PX_PER_SECOND = 20;

/** 往返周期下限：短文本也保持慢速，不出现来回急停。 */
export const OVERFLOW_LOOP_MIN_MS = 4000;

/** 每端最短停顿，避免刚到达就折返。 */
export const OVERFLOW_HOLD_MS = 600;

/** 一次溢出滚动的完整计划。 */
export interface OverflowScrollPlan
{
    distance: number; // 需要位移的像素。
    travelMs: number; // 单程滑动时长。
    holdMs: number; // 每端停顿。
    cycleMs: number; // 一个往返周期。
}

/**
 * 需要位移的像素：内容宽度超过可视宽度才滚动，按整数取整避免亚像素反复触发。
 * 任一数值无效或内容不超出时返回 0，调用方据此保持单行截断。
 */
export const overflowDistance = (contentSize: number, boxSize: number): number => {
    if (!Number.isFinite(contentSize) || !Number.isFinite(boxSize) || contentSize <= 0 || boxSize <= 0 || contentSize <= boxSize) return 0;
    return Math.round(contentSize - boxSize);
};

/**
 * 按实测距离生成往返计划：单程时长 = 距离 ÷ 20 DIP/s，往返位移 = 2 × 距离，
 * 两端停顿由关键帧区间承担，周期不短于 4 秒。
 */
export const overflowScrollPlan = (distance: number): OverflowScrollPlan | undefined => {
    if (!Number.isFinite(distance) || distance <= 0) return undefined;
    const pixels = Math.round(distance);
    if (pixels <= 0) return undefined;
    const travelMs = Math.round((pixels / OVERFLOW_SPEED_PX_PER_SECOND) * 1000);
    const cycleMs = Math.max(OVERFLOW_LOOP_MIN_MS, travelMs * 2 + OVERFLOW_HOLD_MS * 2);
    const holdMs = Math.round((cycleMs - travelMs * 2) / 2);
    return { distance: pixels, travelMs, holdMs, cycleMs };
};

/** 往返周期时长：保留给只需要时长的调用方；无溢出或非法距离返回 0。 */
export const overflowLoopMs = (distance: number): number => overflowScrollPlan(distance)?.cycleMs ?? 0;

/** 关键帧位移点：offset 为周期占比，x 为向左位移像素；两端停顿在 offset 区间内体现。 */
export interface OverflowScrollKeyframe
{
    offset: number;
    x: number;
}

/** 生成与计划一致的往返关键帧：总位移 2 × 距离，滑动段严格按 20 DIP/s。 */
export const overflowScrollKeyframes = (plan: OverflowScrollPlan): OverflowScrollKeyframe[] => {
    const hold = plan.holdMs / plan.cycleMs;
    const travel = plan.travelMs / plan.cycleMs;
    return [
        { offset: 0, x: 0 },
        { offset: hold, x: 0 },
        { offset: hold + travel, x: -plan.distance },
        { offset: hold + travel + hold, x: -plan.distance },
        { offset: 1, x: 0 },
    ];
};
