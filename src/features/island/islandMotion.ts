import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";

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
    const left =
        edge === "right"
            ? to.windowBox.left + to.windowBox.width - width
            : edge === ""
              ? to.windowBox.left + (to.windowBox.width - width) / 2
              : to.windowBox.left;
    const top = edge === "bottom" ? to.windowBox.top + to.windowBox.height - height : to.windowBox.top;
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
    return { state: to.state, windowBox, cards };
};
