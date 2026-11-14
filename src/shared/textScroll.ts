/**
 * 单行文本溢出滚动的公共节奏：浮岛卡片与桌面音乐格子共用同一套距离与时长换算。
 *
 * 偏移只用 transform 推进，不写布局；窗口尺寸与浮岛几何都不参与，避免滚动引起重新测量。
 */

/** 一个往返每像素的时长（毫秒）：20px/s，慢速可读。 */
export const OVERFLOW_LOOP_MS_PER_PX = 100;

/** 往返周期下限：短文本也保持慢速，不出现来回急停。 */
export const OVERFLOW_LOOP_MIN_MS = 4000;

/**
 * 需要位移的像素：内容宽度超过可视宽度才滚动，按整数取整避免亚像素反复触发。
 * 任一数值无效或内容不超出时返回 0，调用方据此保持单行截断。
 */
export const overflowDistance = (contentSize: number, boxSize: number): number => {
    if (!Number.isFinite(contentSize) || !Number.isFinite(boxSize) || contentSize <= 0 || boxSize <= 0 || contentSize <= boxSize) return 0;
    return Math.round(contentSize - boxSize);
};

/** 往返周期时长：距离越长越慢，低于下限时取下限。 */
export const overflowLoopMs = (distance: number): number => {
    if (!Number.isFinite(distance) || distance <= 0) return 0;
    return Math.max(OVERFLOW_LOOP_MIN_MS, Math.round(distance * OVERFLOW_LOOP_MS_PER_PX));
};
