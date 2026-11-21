import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";
import { islandAttachedCorner } from "./islandGeometry.ts";

/** 锚点接力复用视觉规范的跨内容位移 4 DIP；只移动详情视觉层，不改变几何。 */
const ISLAND_HOVER_TRAVEL_DIP = 4;
/** hover 切锚点时旧详情退出时长，不改变胶囊接力。 */
export const ISLAND_HOVER_DETAIL_EXIT_MS = 140;
/** 旧详情退出后，新详情入场时长，不改变顶部图标过渡。 */
export const ISLAND_HOVER_DETAIL_ENTER_MS = 180;

/** 按锚点行顺序决定离场终点与入场起点；缺少任一端时直接切换。 */
export const islandHoverTravel = (ids: readonly string[], fromId: string | undefined, toId: string | undefined) => {
    const from = fromId === undefined ? -1 : ids.indexOf(fromId);
    const to = toId === undefined ? -1 : ids.indexOf(toId);
    if (from < 0 || to < 0 || from === to) return undefined;
    const enterX = Math.sign(to - from) * ISLAND_HOVER_TRAVEL_DIP;
    return { exitX: -enterX, enterX };
};

/** 两态窗口盒的并集，夹在工作区内；并集是动画期间固定的原生窗口。 */
export const islandUnionBox = (from: IslandStateRequest, to: IslandStateRequest, work: IslandRect): IslandRect => {
    const left = Math.max(work.left, Math.min(from.windowBox.left, to.windowBox.left));
    const top = Math.max(work.top, Math.min(from.windowBox.top, to.windowBox.top));
    const right = Math.min(work.left + work.width, Math.max(from.windowBox.left + from.windowBox.width, to.windowBox.left + to.windowBox.width));
    const bottom = Math.min(work.top + work.height, Math.max(from.windowBox.top + from.windowBox.height, to.windowBox.top + to.windowBox.height));
    return { left, top, width: Math.max(1, right - left), height: Math.max(1, bottom - top) };
};

/** 固定窗口盒的动画帧：窗口取两态并集，内容按原插值在并集内移动，切态不再同时移动原生窗口。 */
export const islandUnionFrame = (from: IslandStateRequest, to: IslandStateRequest, elapsed: number, edge: string, work: IslandRect, staggerMs = 0): IslandStateRequest => {
    // 展开分两段：当前卡先走满 240ms，其他卡延迟 staggerMs 后从当前卡背景弹出；其他态保持单段。
    const stagger = to.state === "expanded" && staggerMs > 0 && to.cards.length > 1 ? staggerMs : 0;
    const total = ISLAND_MOTION_MS + stagger;
    // 只在形变期间保留并集，终态精确落到目标盒；展开目标已覆盖整条工作区高度。
    if (elapsed >= total) return to;
    const union = islandUnionBox(from, to, work);
    if (stagger > 0) return islandStaggeredFrame(from, to, elapsed, union, stagger); // 展开的两段编排。
    if (elapsed > 0 && (from.state === "expanded" || to.state === "expanded")) {
        // 全高盒只是宿主，不是内容尺寸；从真实卡片绝对位置插值，防止出展开时第一帧撑成整列。
        return islandStaggeredFrame(from, to, elapsed, union, 0);
    }
    const frame = elapsed <= 0 ? from : islandMotionFrame(from, to, elapsed, edge, work); // 起点帧必须按 from 自己的贴边锚点取窗口，否则内容会错位到 to 那一侧。
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

/** 展开两段帧：当前卡从入态矩形走向展开矩形，其他卡延迟后从当前卡展开矩形弹到各自位置。 */
const islandStaggeredFrame = (from: IslandStateRequest, to: IslandStateRequest, elapsed: number, union: IslandRect, stagger: number): IslandStateRequest => {
    // 目标矩形在 to 自己的窗口坐标里，先换算到并集坐标；并集可能比展开窗口更大。
    const dx = to.windowBox.left - union.left;
    const dy = to.windowBox.top - union.top;
    const target = to.cards.map((card) => ({ ...card, rect: { ...card.rect, left: card.rect.left + dx, top: card.rect.top + dy } }));
    const anchor = target[0]!.rect; // 其他卡的起点与当前卡展开矩形重合，视觉上从当前卡背后冒出。
    const anchorRadius = target[0]!.radius;
    const origin = from.cards[0];
    const progress = Math.min(1, Math.max(0, islandMotionProgress(elapsed))); // 卡片矩形不继承外框回弹。
    const clamp = (rect: { left: number; top: number; width: number; height: number }) => {
        const width = Math.min(rect.width, union.width - rect.left);
        const height = Math.min(rect.height, union.height - rect.top);
        return { ...rect, width: Math.max(1, width), height: Math.max(1, height) };
    };
    const cards = target.map((card, index) => {
        if (index === 0) {
            const start = origin
                ? { left: origin.rect.left + (from.windowBox.left - union.left), top: origin.rect.top + (from.windowBox.top - union.top), width: origin.rect.width, height: origin.rect.height }
                : anchor;
            // 宽度只用不回弹的进度：back 曲线越过 1 时会让卡宽先缩到目标以下再胀回，观感是展开前先向内收一下。
            const widthProgress = Math.min(1, progress);
            const rect = clamp({ left: mix(start.left, card.rect.left, progress), top: mix(start.top, card.rect.top, progress), width: mix(start.width, card.rect.width, widthProgress), height: mix(start.height, card.rect.height, progress) });
            return { rect, radius: Math.max(0, Math.min(mix(origin?.radius ?? card.radius, card.radius, progress), rect.width / 2, rect.height / 2)) };
        }
        const local = Math.min(1, Math.max(0, islandMotionProgress(elapsed - stagger)));
        const rect = clamp({ left: mix(anchor.left, card.rect.left, local), top: mix(anchor.top, card.rect.top, local), width: mix(anchor.width, card.rect.width, local), height: mix(anchor.height, card.rect.height, local) });
        return { rect, radius: Math.max(0, Math.min(mix(anchorRadius, card.radius, local), rect.width / 2, rect.height / 2)) };
    });
    return { state: to.state, windowBox: union, viewport: to.viewport, cards };
};

export const ISLAND_MOTION_MS = 240; // 有界形变与内容显现共用时长。
export const ISLAND_EXPAND_STAGGER_MS = 100; // 展开时其他卡相对当前卡延迟弹出，与相位 1 末尾重叠。
export const ISLAND_EXPAND_MOTION_MS = ISLAND_MOTION_MS + ISLAND_EXPAND_STAGGER_MS; // 展开动画总时长。
export const ISLAND_FRAME_MS = 1000 / 30; // 先以 30fps 验证原生区域更新成本。

/** ease-out-back 的峰值约 5.3%，端点严格收敛。 */
export const islandMotionProgress = (elapsed: number): number => {
    const t = Math.min(1, Math.max(0, elapsed / ISLAND_MOTION_MS));
    if (t === 0 || t === 1) return t;
    const u = t - 1;
    return 1 + 2.2 * u ** 3 + 1.2 * u ** 2;
};
const mix = (from: number, to: number, progress: number) => from + (to - from) * progress;

/** 视口切换的遮蔽租约：owner 是唯一可绘制/解除的任务 token，masked 是当前可见性状态。 */
export interface IslandMaskLease {
    seq: number;
    owner: number | undefined;
    masked: boolean;
}

/** 新任务领取租约并使旧 token 失效；旧任务后续的绘制/解除都会被拒绝。 */
export const beginIslandMask = (lease: IslandMaskLease): number => {
    const token = ++lease.seq;
    lease.owner = token;
    lease.masked = true;
    return token;
};

/** 仅租约所有者可解除视觉遮蔽，保留 owner 供本任务后续动画绘制。 */
export const unmaskIslandTransition = (lease: IslandMaskLease, token: number): boolean => {
    if (lease.owner !== token || !lease.masked) return false;
    lease.masked = false;
    return true;
};

/** 幂等释放：旧 token 不得解除新租约或清掉新 owner。 */
export const releaseIslandMask = (lease: IslandMaskLease, token: number): boolean => {
    if (lease.owner !== token) return false;
    lease.masked = false;
    lease.owner = undefined;
    return true;
};

/** 当前任务是否仍持有租约；旧回包据此拒绝绘制或解除新遮蔽。 */
export const holdsIslandMask = (lease: IslandMaskLease, token: number): boolean => lease.owner === token;

/** 共享原生物理状态：pending 是尚未回包的 set_bounds 数，appliedToken 是最后成功落地的任务 token。 */
export interface IslandNativeSwap {
    pending: number;
    appliedToken: string | null;
    appliedGeometry?: IslandStateRequest;
}

/** set_bounds 发出前登记；回包后必须成对 finish。 */
export const beginIslandNativeSwap = (swap: IslandNativeSwap): void => {
    swap.pending += 1;
};

/** set_bounds 回包：成功记录物理落地的 token，失败也减少 pending。 */
export const finishIslandNativeSwap = (swap: IslandNativeSwap, token: string, applied: boolean): void => {
    swap.pending = Math.max(0, swap.pending - 1);
    if (applied) swap.appliedToken = token;
};

/** 只有 native 已清空且本任务落地（或已按实际几何重同步）才允许解除遮蔽。 */
export const canReleaseIslandMask = (swap: IslandNativeSwap, token: string, resynced: boolean): boolean =>
    swap.pending === 0 && (resynced || swap.appliedToken === token);

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
