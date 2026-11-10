import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";
import { islandAttachedCorner } from "./islandGeometry.ts";

/** 两态窗口盒的并集，夹在工作区内；并集是动画期间固定的原生窗口。 */
export const islandUnionBox = (from: IslandStateRequest, to: IslandStateRequest, work: IslandRect): IslandRect => {
    const left = Math.max(work.left, Math.min(from.windowBox.left, to.windowBox.left));
    const top = Math.max(work.top, Math.min(from.windowBox.top, to.windowBox.top));
    const right = Math.min(work.left + work.width, Math.max(from.windowBox.left + from.windowBox.width, to.windowBox.left + to.windowBox.width));
    const bottom = Math.min(work.top + work.height, Math.max(from.windowBox.top + from.windowBox.height, to.windowBox.top + to.windowBox.height));
    return { left, top, width: Math.max(1, right - left), height: Math.max(1, bottom - top) };
};

/** 固定窗口盒的动画帧：窗口取两态并集，内容按原插值在并集内移动，切态不再同时移动原生窗口。 */
export const islandUnionFrame = (from: IslandStateRequest, to: IslandStateRequest, elapsed: number, edge: string, work: IslandRect): IslandStateRequest => {
    // 展开终态保留入态并集，只有其余态在结束时收缩；内容仍使用目标屏幕坐标。
    if (elapsed >= ISLAND_MOTION_MS && to.state !== "expanded") return to;
    const frame = elapsed <= 0 ? from : islandMotionFrame(from, to, elapsed, edge, work); // 起点帧必须按 from 自己的贴边锚点取窗口，否则内容会错位到 to 那一侧。
    const union = islandUnionBox(from, to, work);
    const dx = frame.windowBox.left - union.left;
    const dy = frame.windowBox.top - union.top;
    const cards = frame.cards.map((card) => {
        const width = Math.min(card.rect.width, union.width);
        const height = Math.min(card.rect.height, union.height);
        const left = Math.min(Math.max(0, card.rect.left + dx), Math.max(0, union.width - width));
        const top = Math.min(Math.max(0, card.rect.top + dy), Math.max(0, union.height - height));
        return { ...card, rect: { ...card.rect, left, top, width, height } };
    });
    return { state: frame.state, windowBox: union, viewport: to.viewport, cards };
};

export const ISLAND_MOTION_MS = 240; // 有界形变与内容显现共用时长。
export const ISLAND_FRAME_MS = 1000 / 30; // 先以 30fps 验证原生区域更新成本。

/** ease-out-back 的峰值约 5.3%，端点严格收敛。 */
export const islandMotionProgress = (elapsed: number): number => {
    const t = Math.min(1, Math.max(0, elapsed / ISLAND_MOTION_MS));
    if (t === 0 || t === 1) return t;
    const u = t - 1;
    return 1 + 2.2 * u ** 3 + 1.2 * u ** 2;
};
const mix = (from: number, to: number, progress: number) => from + (to - from) * progress;

/** 固定贴边锚点，非贴边轴沿既有偏好；默认位固定既有中心而非采样新中心。 */
export const islandMotionFrame = (from: IslandStateRequest, to: IslandStateRequest, elapsed: number, edge: string, work: IslandRect): IslandStateRequest => {
    const p = islandMotionProgress(elapsed);
    if (elapsed >= ISLAND_MOTION_MS) return to;
    const bounded = (a: number, b: number, limit: number) => Math.min(limit, Math.max(Math.min(a, b) * 0.94, Math.min(Math.max(a, b) * 1.06, mix(a, b, p))));
    const width = bounded(from.windowBox.width, to.windowBox.width, work.width);
    const height = bounded(from.windowBox.height, to.windowBox.height, work.height);
    // 沿边轴与真实落位一致：上下贴边沿水平居中、左右贴边沿竖直居中；非沿边轴贴边锚定。
    const horizontalEdge = edge === "left" || edge === "right";
    const left = horizontalEdge
        ? edge === "right"
            ? to.windowBox.left + to.windowBox.width - width
            : to.windowBox.left
        : to.windowBox.left + (to.windowBox.width - width) / 2;
    const top = horizontalEdge
        ? to.windowBox.top + (to.windowBox.height - height) / 2
        : edge === "bottom"
          ? to.windowBox.top + to.windowBox.height - height
          : to.windowBox.top;
    const windowBox = {
        left: Math.max(work.left, Math.min(left, work.left + work.width - width)),
        top: Math.max(work.top, Math.min(top, work.top + work.height - height)),
        width,
        height,
    };
    const shape = Math.min(1, Math.max(0, p)); // 回弹只作用外框，区域分配保持在盒内。
    const cards = to.cards.map((card, index) => {
        const previous =
            from.cards.length === to.cards.length
                ? from.cards[index]!
                : {
                      rect: {
                          left: 0,
                          top: (from.windowBox.height * index) / to.cards.length,
                          width: from.windowBox.width,
                          height: from.windowBox.height / to.cards.length,
                      },
                      radius: from.cards[0]?.radius ?? 0,
                  };
        const rect = {
            left: mix(previous.rect.left / from.windowBox.width, card.rect.left / to.windowBox.width, shape) * width,
            top: mix(previous.rect.top / from.windowBox.height, card.rect.top / to.windowBox.height, shape) * height,
            width: mix(previous.rect.width / from.windowBox.width, card.rect.width / to.windowBox.width, shape) * width,
            height: mix(previous.rect.height / from.windowBox.height, card.rect.height / to.windowBox.height, shape) * height,
        };
        // 浮点末位也不能让局部矩形超过原生窗口盒。
        rect.width = Math.min(rect.width, width - rect.left);
        rect.height = Math.min(rect.height, height - rect.top);
        return { rect, radius: Math.max(0, Math.min(mix(previous.radius, card.radius, shape), rect.width / 2, rect.height / 2)) };
    });
    return { state: to.state, windowBox, viewport: to.viewport, cards };
};

/** 单帧贴边侧的带符号圆角：从最近实际绘制的起点值按形变进度过渡到目标态的贴边角。 */
export const islandAttachedCornerAt = (start: number, to: IslandStateRequest, elapsed: number, form: "capsule" | "notch"): number => {
    if (form !== "notch") return 0;
    const end = islandAttachedCorner(to.state, form, to.cards[0]?.radius ?? 0);
    if (elapsed >= ISLAND_MOTION_MS) return end;
    const shape = Math.min(1, Math.max(0, islandMotionProgress(elapsed))); // 回弹只作用外框，贴边角按同一进度插值。
    return start + (end - start) * shape;
};
