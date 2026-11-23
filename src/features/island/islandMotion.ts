import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";
import { islandAttachedCorner, islandWindowKey } from "./islandGeometry.ts";

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

/** 两次提交是否换了固定视口带：页面坐标原点随视口变化，换原点后必须先等新帧提交再解除遮蔽。 */
export const islandViewportChanged = (from: IslandStateRequest | undefined, next: IslandStateRequest): boolean => !from || !sameIslandRect(from.viewport, next.viewport);

/** 一次几何提交的贴边身份：视口相同但贴边方向或形态变了，内容锚点仍然换了位置。 */
export interface IslandGeometrySeat {
    edge: string; // 空串表示默认顶部居中。
    form: string; // capsule 或 notch。
}

/** 几何比较容差：原生与页面各自量化，半个 DIP 内视为同一位置或尺寸。 */
const ISLAND_GEOMETRY_TOLERANCE = 0.5;

/** 两个矩形是否在容差内相等；视口与窗口盒共用同一判据。 */
const sameIslandRect = (a: IslandRect, b: IslandRect): boolean =>
    Math.abs(a.left - b.left) <= ISLAND_GEOMETRY_TOLERANCE &&
    Math.abs(a.top - b.top) <= ISLAND_GEOMETRY_TOLERANCE &&
    Math.abs(a.width - b.width) <= ISLAND_GEOMETRY_TOLERANCE &&
    Math.abs(a.height - b.height) <= ISLAND_GEOMETRY_TOLERANCE;

/**
 * 几何 key 变化时是否需要整层遮蔽。
 * 同一视口、同一状态、同一贴边形态下，窗口盒改尺寸或只改卡片/锚点都不遮蔽：窗口没有换锚点，
 * 页面可以直接按新几何重绘，胶囊加宽时的新消息交接动画也就不必被整层藏掉。
 * 回锚提交（拖动落点后的首次提交）、换视口（显示器、DPI、工作区或沿边偏移）、换状态、
 * 换贴边或形态，以及不换尺寸的纯移动（父窗口已移位、子窗口还要回锚）仍沿用遮蔽。
 */
export const shouldMaskGeometryChange = (
    from: IslandStateRequest | undefined,
    next: IslandStateRequest,
    fromSeat: IslandGeometrySeat | undefined,
    nextSeat: IslandGeometrySeat,
    reseat = false,
): boolean => {
    if (reseat) return true; // 拖动落点后的首次提交：父窗口已移位，子窗口回锚前不能露出旧坐标。
    if (!from || !fromSeat) return true; // 首次提交或换边/换形态后已清空基准，没有可比坐标。
    if (from.state !== next.state) return true; // 三态各自一套内容坐标系。
    if (fromSeat.edge !== nextSeat.edge || fromSeat.form !== nextSeat.form) return true; // 贴边或形态换了锚点。
    if (!sameIslandRect(from.viewport, next.viewport)) return true; // 固定视口换了，旧坐标不能直接复用。
    const moved = Math.abs(from.windowBox.left - next.windowBox.left) > ISLAND_GEOMETRY_TOLERANCE || Math.abs(from.windowBox.top - next.windowBox.top) > ISLAND_GEOMETRY_TOLERANCE;
    const resized = Math.abs(from.windowBox.width - next.windowBox.width) > ISLAND_GEOMETRY_TOLERANCE || Math.abs(from.windowBox.height - next.windowBox.height) > ISLAND_GEOMETRY_TOLERANCE;
    return moved && !resized; // 纯移动要等窗口回锚后的重绘，改尺寸则可以原地重绘。
};

/** 一次几何提交的遮蔽决策：mask 决定是否整层遮蔽，seat 是下一次比较用的贴边身份基准。 */
export interface IslandGeometryCommitDecision {
    mask: boolean;
    seat: IslandGeometrySeat;
}

/**
 * 连续提交的纯决策：基准取自上一次提交，并随本次提交前移，不随测量 effect 固化。
 * 换边/换形态后的第一次提交遮蔽一次，同一 effect 内的下一次普通窗口盒变化就不再遮蔽。
 */
export const islandGeometryCommitDecision = (
    from: IslandStateRequest | undefined,
    next: IslandStateRequest,
    baseline: IslandGeometrySeat | undefined,
    nextSeat: IslandGeometrySeat,
    reseat = false,
): IslandGeometryCommitDecision => ({
    mask: shouldMaskGeometryChange(from, next, baseline, nextSeat, reseat),
    seat: nextSeat,
});

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

/** 一次挂起的解除：保留 keepOwner 与 alreadySynced 语义，回包结算时不再重画。 */
export interface IslandDeferredRelease {
    token: number;
    keepOwner: boolean;
    alreadySynced: boolean;
}

/** 登记挂起解除；同一个 token 只保留最后一次语义，避免旧集合丢失 alreadySynced。 */
export const deferIslandMaskRelease = (queue: Map<number, IslandDeferredRelease>, token: number, options: { keepOwner?: boolean; alreadySynced?: boolean } = {}): void => {
    queue.set(token, { token, keepOwner: options.keepOwner === true, alreadySynced: options.alreadySynced === true });
};

/** 取出全部挂起解除并清空队列，顺序即登记顺序。 */
export const takeIslandMaskReleases = (queue: Map<number, IslandDeferredRelease>): IslandDeferredRelease[] => {
    const releases = [...queue.values()];
    queue.clear();
    return releases;
};

/**
 * 挂起的旧回包是否还允许重画：实例仍存活、原生已落地且没有「已同步」标记。
 * 被替换或已卸载的实例只允许结算租约，不得再用旧闭包写固定尺寸与透明度。
 */
export const canResyncIslandGeometry = (input: { active: boolean; applied: boolean; alreadySynced: boolean }): boolean =>
    input.active && input.applied && !input.alreadySynced;

/**
 * 有界补偿重提交：每次重试前先等待，再确认实例仍持有租约；全部失败返回 false。
 * @param options submit 提交、wait 等待、current 实例是否仍持有租约、attempts 重试次数。
 * @returns 是否有一次提交成功。
 */
export const islandRetrySubmit = async (options: {
    submit: () => Promise<unknown>;
    wait: (milliseconds: number) => Promise<unknown>;
    current: () => boolean;
    attempts?: number;
}): Promise<boolean> => {
    const attempts = options.attempts ?? 2;
    for (let attempt = 0; attempt < attempts; attempt += 1) {
        await options.wait(100);
        if (!options.current()) return false;
        try {
            await options.submit();
            return true;
        } catch {
            // 继续下一次有界重试。
        }
    }
    return false;
};

/**
 * 提交重试耗尽后的基准：几何、贴边身份与去重键全部失效，
 * 下一次提交按「没有可比坐标」重新遮蔽，不再沿用从未落地的几何。
 */
export const resetIslandGeometryBaseline = (): { geometry: undefined; seat: undefined; key: "" } => ({ geometry: undefined, seat: undefined, key: "" });

/** 几何去重键：展开只看窗口盒/视口/态，紧凑态看整份请求；提交与失败门共用同一算式。 */
export const islandGeometryKey = (request: IslandStateRequest): string => (request.state === "expanded" ? islandWindowKey(request) : JSON.stringify(request));

/**
 * 失败门：上一条从未落地的目标几何不再重发，避免「失败清基准 → 释放时强制重测 → 再提交」无限重启。
 * 只有外部状态变化后的 fail-closed 重提或几何本身改变才放行。
 * @param input failedKey 上次重试耗尽的目标键；key 本次目标键；resubmit 外部变化后的首个强制重提。
 * @returns 是否跳过本次提交。
 */
export const islandFailedGeometryGate = (input: { failedKey: string; key: string; resubmit: boolean }): boolean =>
    input.failedKey !== "" && input.failedKey === input.key && !input.resubmit;

/** 无帧收口上限：页面不派发帧（隐藏/被节流）时，等待与揭幕都由计时器推进，不允许卡住提交。 */
export const ISLAND_NO_FRAME_TIMEOUT_MS = 300;

/**
 * 可被外部提前唤醒、也可由计时器推进的帧等待：帧回调、wake 或超时任一先到都收口。
 * cleanup 或 finishMotion 取消帧时必须调用 wake，否则 submission 永不结束。
 * @param requestFrame 帧调度；options.timeoutMs/setTimer/clearTimer 提供无帧时的兜底。
 * @returns done 与 wake。
 */
export const islandWakingFrame = (
    requestFrame: (callback: () => void) => unknown,
    options: { timeoutMs?: number; setTimer?: (callback: () => void) => unknown; clearTimer?: (handle: unknown) => void } = {},
): { done: Promise<void>; wake: () => void } => {
    let 收口: () => void = () => {};
    let 定时: unknown;
    let 已收口 = false;
    const 完成 = () => {
        if (已收口) return;
        已收口 = true;
        if (定时 !== undefined) options.clearTimer?.(定时);
        收口();
    };
    const done = new Promise<void>((resolve) => {
        收口 = resolve;
    });
    requestFrame(完成);
    if (!已收口 && options.timeoutMs !== undefined && options.setTimer) 定时 = options.setTimer(完成);
    return { done, wake: 完成 };
};

/** 一次几何提交的预留：记录目标 key、提交权归属、是否已发过过渡帧与是否已发最终目标。 */
export interface IslandSubmitReservation {
    key: string;
    token: number;
    submitted: boolean; // 是否发过任意原生请求（含并集过渡帧）。
    submittedFinal: boolean; // 是否发过最终目标 request：只有它为真才算这次提交完成收口。
}

/**
 * cleanup 撤销「尚未发出最终目标」的提交预留：只有 token 匹配、最终目标未发且键仍是该 key 时才撤销。
 * 过渡帧（union）已发不算完成——胶囊的最终 windowBox 还没发，继任 effect 必须能补发；
 * 已发最终目标则保留 key，继任按同 key 去重，不会重复提交。
 * @returns 需要清掉的 key；不需要撤销时返回 undefined。
 */
export const revokeIslandReservation = (reservation: IslandSubmitReservation | undefined, token: number, currentKey: string): string | undefined =>
    reservation && reservation.token === token && !reservation.submittedFinal && reservation.key === currentKey ? reservation.key : undefined;

/** 隐藏帧等待结果：frame = 真实渲染机会收口；timeout = 无帧超时收口；wake = 被取消唤醒。 */
export type IslandRenderGateResult = "frame" | "timeout" | "wake";

/**
 * 等 count 次渲染机会，共享一个总超时。
 * rAF 回调在绘制之前执行，单次 rAF 无法证明上一帧的样式已经上屏，所以默认等两次；
 * 取消（cleanup/接管）时必须调用 wake，避免等待方空等到超时。
 * @returns done（含收口原因）与 wake。
 */
export const islandRenderGate = (options: {
    count?: number;
    timeoutMs: number;
    requestFrame: (callback: () => void) => unknown;
    cancelFrame?: (handle: unknown) => void;
    setTimer: (callback: () => void) => unknown;
    clearTimer: (handle: unknown) => void;
}): { done: Promise<IslandRenderGateResult>; wake: () => void } => {
    const 次数 = Math.max(1, options.count ?? 2);
    let 收口: (结果: IslandRenderGateResult) => void = () => {};
    let 已完成 = false;
    let 已过帧 = 0;
    let 帧: unknown;
    let 定时: unknown;
    const 完成 = (结果: IslandRenderGateResult) => {
        if (已完成) return;
        已完成 = true;
        if (定时 !== undefined) options.clearTimer(定时);
        if (帧 !== undefined) options.cancelFrame?.(帧);
        收口(结果);
    };
    const 等帧 = () => {
        帧 = options.requestFrame(() => {
            已过帧 += 1;
            if (已过帧 >= 次数) 完成("frame");
            else 等帧();
        });
    };
    const done = new Promise<IslandRenderGateResult>((resolve) => {
        收口 = resolve;
    });
    等帧();
    if (!已完成) 定时 = options.setTimer(() => 完成("timeout"));
    return { done, wake: () => 完成("wake") };
};

/**
 * 帧或超时竞速：只执行一次。隐藏页不派发帧时由计时器推进，解除遮蔽等收口动作不被卡住。
 * @param options act 执行动作；requestFrame/cancelFrame/setTimer/clearTimer 注入（单测可只推进计时器）。
 * @returns 取消函数（已执行或取消后不再触发）。
 */
export const islandFrameOrTimeout = (options: {
    act: () => void;
    requestFrame: (callback: () => void) => unknown;
    cancelFrame: (handle: unknown) => void;
    setTimer: (callback: () => void) => unknown;
    clearTimer: (handle: unknown) => void;
    timeoutMs: number;
}): (() => void) => {
    let 完成 = false;
    let 帧: unknown;
    let 定时: unknown;
    const 执行 = () => {
        if (完成) return;
        完成 = true;
        options.clearTimer(定时);
        options.cancelFrame(帧);
        options.act();
    };
    帧 = options.requestFrame(执行);
    if (!完成) 定时 = options.setTimer(执行);
    return () => {
        if (完成) return;
        完成 = true;
        options.clearTimer(定时);
        options.cancelFrame(帧);
    };
};

/** 揭幕视口条件：页面 inner 尺寸与目标固定视口一致（容差 1 DIP）。 */
export const islandViewportMatched = (inner: { width: number; height: number }, viewport: Pick<IslandRect, "width" | "height">): boolean =>
    Math.abs(inner.width - viewport.width) <= 1 && Math.abs(inner.height - viewport.height) <= 1;

/** 视口条件等待上限：WebView 换视口偶发滞后，到点也必须收口，不能挂死。 */
export const ISLAND_VIEWPORT_WAIT_MS = 300;

/**
 * 等页面真正落到目标视口再解除遮蔽：轮询 inner 尺寸直到与目标一致（容差 1 DIP），
 * 匹配后再等一帧让新布局提交；有界超时（默认 ISLAND_VIEWPORT_WAIT_MS）到点也收口。
 * @param options viewport 目标视口；inner 尺寸读取；wait 单帧等待（调用方负责让等待也受剩余超时约束）；
 *   now 时钟；timeoutMs 超时。
 * @returns 是否在超时前匹配到目标视口（false = 超时收口）。
 */
export const awaitIslandViewport = async (options: {
    viewport: Pick<IslandRect, "width" | "height">;
    inner: () => { width: number; height: number };
    wait: (remainingMs: number) => Promise<void>;
    now?: () => number;
    timeoutMs?: number;
}): Promise<boolean> => {
    const now = options.now ?? Date.now;
    const deadline = now() + (options.timeoutMs ?? ISLAND_VIEWPORT_WAIT_MS);
    let attempts = 0;
    while (!islandViewportMatched(options.inner(), options.viewport) && now() < deadline && attempts < 600) {
        attempts += 1; // 保险丝：即使时钟不前进也不会无限等。
        await options.wait(Math.max(0, deadline - now()));
    }
    const matched = islandViewportMatched(options.inner(), options.viewport);
    await options.wait(Math.max(0, deadline - now())); // 匹配后也要等一帧提交；超时路径同样只再收口一次。
    return matched;
};
