import type { IslandStateRequest } from "../../contracts/island.ts";
import {
    beginIslandMask,
    beginIslandNativeSwap,
    canReleaseIslandMask,
    canResyncIslandGeometry,
    deferIslandMaskRelease,
    finishIslandNativeSwap,
    holdsIslandMask,
    islandFrameOrTimeout,
    releaseIslandMask,
    takeIslandMaskReleases,
    unmaskIslandTransition,
    ISLAND_NO_FRAME_TIMEOUT_MS,
    type IslandMaskLease,
    type IslandNativeSwap,
} from "./islandMotion.ts";

/** native 失败兜底解除时长：到点只解除 CSS，避免窗口永久隐藏。 */
export const ISLAND_MASK_FAIL_FALLBACK_MS = 800;

/** 解除前一致性校验容差：半个 DIP 量级内视为同一尺寸，不重画。 */
const ISLAND_REVEAL_TOLERANCE = 1.5;

/**
 * 帧与计时器调度：默认取浏览器实现，单测注入假时钟即可推进整套解除时序。
 * 它是 module 的内部 seam，不是宿主需要知道的事实。
 */
export interface IslandMaskScheduler {
    requestFrame(callback: () => void): unknown; // 请求下一帧。
    cancelFrame(handle: unknown): void; // 取消已请求的帧。
    setTimer(callback: () => void, milliseconds: number): unknown; // 起一个计时器。
    clearTimer(handle: unknown): void; // 取消计时器。
}

/**
 * 宿主交给遮蔽门的能力：写入可见性、还原内联几何、按给定几何重绘、读取实测。
 * 遮蔽门不知道 DOM 结构，宿主不知道「什么时候可以揭开」——分界就在这里。
 */
export interface IslandMaskHost {
    /** 写入遮蔽可见性，返回写入前的状态；遮蔽门据此决定是否需要重等一帧。 */
    onHidden(hidden: boolean): boolean;
    /** 还原本实例写入的内联几何，返回本实例是否仍然存活；卸载后的实例不再重画。 */
    onRestore(): boolean;
    /** 按给定几何重绘一帧，返回是否真的画了（无绘制闭包时返回 false）。 */
    onResync(value: IslandStateRequest): boolean;
    /** 已绘制首张卡片的尺寸，用于解除前的一致性校验。 */
    paintedCard(): { width: number; height: number } | undefined;
    /** 当前实际显示几何。 */
    current(): IslandStateRequest | undefined;
    /** 本轮目标几何，优先于 current 作为重同步基准。 */
    target(): IslandStateRequest | undefined;
    /** 解除遮蔽之后：恢复输入、按实际几何重测，并补收遮蔽期间按下的 Esc。 */
    onRevealed(token: number): void;
}

export interface IslandMaskSettleOptions {
    keepOwner?: boolean; // 保留所有权供本任务后续动画绘制。
    resynced?: boolean; // 本次回包已同步目标布局。
    alreadySynced?: boolean; // 调用方已自行同步，不再重绘。
    failed?: boolean; // 本次提交失败，走 800ms 兜底。
}

/** 门对外的一次性状态快照：调用方只读这一份事实，不再各自持有一半。 */
export interface IslandCommitState {
    owner?: number; // 当前租约所有者 token。
    masked: boolean; // 是否真正遮蔽。
    pending: number; // 尚未回包的 set_bounds 数量。
    needsPaint: boolean; // 本次「可见变隐藏」是否还需要等一次绘制。
    applied: boolean; // 是否有过一次成功落地。
    settled: boolean; // 几何是否已落地、交互是否已恢复。
    transitioning: boolean; // 切态形变是否在途（期间窗口是两态并集）。
}

export interface IslandCommitGate {
    /** 新任务领取租约并使旧 token 失效；hide 为假时只占租约，不改可见性。 */
    begin(hide?: boolean): number;
    /** 当前任务是否仍持有租约。 */
    owns(token: number): boolean;
    /** 读取当前状态快照。 */
    state(): IslandCommitState;
    /** 隐藏帧已画上屏幕。 */
    markPainted(): void;
    /** 清就绪：几何或交互尚不可用。 */
    markUnsettled(): void;
    /** 置就绪：几何已落地、交互已恢复。 */
    markSettled(): void;
    /** 切态形变开始：期间窗口是两态并集，不能据此判 hover。 */
    beginTransition(): void;
    /** 形变终帧已结算：窗口已落到目标盒。 */
    endTransition(): void;
    /** set_bounds 发出前登记。 */
    nativeStarted(): void;
    /** set_bounds 回包：成功时记录物理落地几何。 */
    nativeFinished(token: number, applied: boolean, geometry?: IslandStateRequest): void;
    /** 唯一结算入口：native 已清空、token 匹配/重同步后，先重绘生效一帧再解除。 */
    settle(token: number, options?: IslandMaskSettleOptions): void;
    /** 回包之后把挂起的解除全部结算。 */
    drain(freshApplied: boolean, failed?: boolean): void;
    /** 一次 effect 实例交出观测能力；在途提交与挂起解除保持跨实例有效。 */
    attach(host: IslandMaskHost): void;
}

/** 浏览器默认调度：单测注入假实现时整条解除时序都可推进。 */
const browserScheduler: IslandMaskScheduler = {
    requestFrame: (callback) => requestAnimationFrame(callback),
    cancelFrame: (handle) => cancelAnimationFrame(Number(handle)),
    setTimer: (callback) => setTimeout(callback, ISLAND_NO_FRAME_TIMEOUT_MS),
    clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/**
 * 创建一次几何提交的遮蔽门。一个宿主节点只用一个实例：
 * 租约、在途提交与挂起解除必须跨 effect 重跑存活，否则换态途中的回包会被当成新任务的。
 */
export const createIslandCommitGate = (scheduler: IslandMaskScheduler = browserScheduler): IslandCommitGate => {
    const lease: IslandMaskLease = { seq: 0, owner: undefined, masked: false };
    const swap: IslandNativeSwap = { pending: 0, appliedToken: null, appliedGeometry: undefined };
    const deferred = new Map<number, { token: number; keepOwner: boolean; alreadySynced: boolean }>(); // pending 未清空时挂起的解除。
    const failTimers = new Map<number, unknown>(); // 失败兜底计时器，按 token 只登记一个。
    let painted = false;
    let settled = false; // 几何已落地、交互已恢复。
    let transitioning = false; // 切态形变在途。 // 本次「可见变隐藏」是否已画到屏幕上。
    let host: IslandMaskHost | undefined;

    const owns = (token: number): boolean => holdsIslandMask(lease, token);

    /** 只读快照：调用方拿它判遮蔽、在途、就绪与形变，不再各自持有半个事实。 */
    const state = (): IslandCommitState => ({
        owner: lease.owner,
        masked: lease.masked,
        pending: swap.pending,
        needsPaint: !painted,
        applied: swap.appliedToken !== null,
        settled,
        transitioning,
    });

    /** 仅当仍是当前 token 时解除视觉遮蔽，保留所有权供本任务后续动画绘制。 */
    const unmask = (token: number, resynced: boolean): boolean => {
        if (!canReleaseIslandMask(swap, String(token), resynced)) return false;
        if (!unmaskIslandTransition(lease, token)) return false;
        host?.onHidden(false);
        settled = true; // 揭开即恢复交互。
        host?.onRevealed(token);
        return true;
    };

    /** 幂等释放租约：旧 token 不得解除新任务的遮蔽或清掉新所有权。 */
    const release = (token: number, resynced: boolean): boolean => {
        if (!canReleaseIslandMask(swap, String(token), resynced)) return false;
        if (!releaseIslandMask(lease, token)) return false;
        host?.onHidden(false);
        settled = true; // 揭开即恢复交互。
        host?.onRevealed(token);
        return true;
    };

    /** 失败兜底：到点按「已同步」再结算一次，只解除 CSS，不重画。 */
    const scheduleFailFallback = (token: number, options: IslandMaskSettleOptions): void => {
        if (failTimers.has(token)) return;
        failTimers.set(
            token,
            scheduler.setTimer(() => {
                failTimers.delete(token);
                // 兜底这一跳不再带 failed：否则会重新排一次兜底，遮蔽永远解不开。
                settle(token, { keepOwner: options.keepOwner, alreadySynced: options.alreadySynced, resynced: true });
            }, ISLAND_MASK_FAIL_FALLBACK_MS),
        );
    };

    /** 解除动作本体：解除前核对已绘制卡片是否已是目标尺寸，不一致就只补一帧再结算。 */
    const reveal = (token: number, options: IslandMaskSettleOptions, actual: IslandStateRequest | undefined): void => {
        const { keepOwner = false, alreadySynced = false } = options;
        islandFrameOrTimeout({
            act: () => {
                if (!owns(token)) return;
                const 已绘制 = host?.paintedCard();
                const 目标几何 = host?.target() ?? actual;
                const 目标卡 = 目标几何?.cards[0]?.rect;
                const 一致 = !已绘制 || !目标卡 || (Math.abs(已绘制.width - 目标卡.width) < ISLAND_REVEAL_TOLERANCE && Math.abs(已绘制.height - 目标卡.height) < ISLAND_REVEAL_TOLERANCE);
                if (!一致 && 目标几何 && !alreadySynced && host?.onResync(目标几何)) {
                    // 只补一次，下一帧再结算，避免无限重画。
                    islandFrameOrTimeout({
                        act: () => settle(token, { ...options, alreadySynced: true }),
                        requestFrame: scheduler.requestFrame,
                        cancelFrame: scheduler.cancelFrame,
                        setTimer: (callback) => scheduler.setTimer(callback, ISLAND_NO_FRAME_TIMEOUT_MS),
                        clearTimer: scheduler.clearTimer,
                        timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
                    });
                    return;
                }
                if (keepOwner) unmask(token, true);
                else release(token, true);
            },
            requestFrame: scheduler.requestFrame,
            cancelFrame: scheduler.cancelFrame,
            setTimer: (callback) => scheduler.setTimer(callback, ISLAND_NO_FRAME_TIMEOUT_MS),
            clearTimer: scheduler.clearTimer,
            timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
        });
    };

    /** 唯一的解除入口：native 已清空、token 匹配/重同步后，先重绘生效一帧再解除。 */
    const settle = (token: number, options: IslandMaskSettleOptions = {}): void => {
        const { keepOwner = false, resynced = false, alreadySynced = false, failed = false } = options;
        if (!owns(token)) return;
        if (swap.pending > 0) {
            // 等回包后再结算，语义随 token 保留：被替换或已卸载的实例只结算租约，不再重画。
            deferIslandMaskRelease(deferred, token, { keepOwner, alreadySynced });
            return;
        }
        if (!lease.masked) {
            if (!keepOwner) release(token, true);
            return;
        }
        const actual = swap.appliedGeometry ?? host?.current();
        if (failed || (!resynced && swap.appliedToken !== String(token))) {
            scheduleFailFallback(token, { keepOwner, alreadySynced, resynced, failed });
            return;
        }
        const timer = failTimers.get(token);
        if (timer !== undefined) {
            scheduler.clearTimer(timer);
            failTimers.delete(token);
        }
        if (!alreadySynced) {
            // onRestore 返回 false 说明发起结算的实例已卸载：只结算租约，不得用旧闭包写回新实例的 DOM。
            const alive = host?.onRestore() === true; // 存活性由宿主当场给出，不在门里假定。
            // 先还样式，再由重绘写入实际几何，restore 不会覆盖新帧。
            if (canResyncIslandGeometry({ active: alive, applied: actual !== undefined, alreadySynced }) && actual) host?.onResync(actual);
        }
        reveal(token, options, actual);
    };

    return {
        begin(hide = true): number {
            const token = beginIslandMask(lease);
            settled = false; // 新任务的几何尚未落地。
            if (!hide) lease.masked = false; // 没有真正遮蔽，唯一结算入口按未遮蔽直接释放租约。
            else if (!host?.onHidden(true)) painted = false; // 只有真正从可见变隐藏才需要重新等一次绘制。
            return token;
        },
        owns,
        state,
        markPainted(): void {
            painted = true;
        },
        markUnsettled(): void {
            settled = false;
        },
        markSettled(): void {
            settled = true;
        },
        beginTransition(): void {
            transitioning = true;
        },
        endTransition(): void {
            transitioning = false;
        },
        nativeStarted(): void {
            beginIslandNativeSwap(swap);
        },
        nativeFinished(token: number, applied: boolean, geometry?: IslandStateRequest): void {
            finishIslandNativeSwap(swap, String(token), applied); // 任何返回路径都成对归零 pending。
            if (applied) swap.appliedGeometry = geometry; // 成功回包保留实际几何供重同步。
        },
        settle,
        drain(freshApplied: boolean, failed = false): void {
            if (swap.pending > 0) return;
            for (const entry of takeIslandMaskReleases(deferred)) settle(entry.token, { keepOwner: entry.keepOwner, alreadySynced: entry.alreadySynced, resynced: freshApplied, failed });
        },
        attach(next: IslandMaskHost): void {
            // 只换观测能力，不动租约与在途提交：旧实例的回包仍要能结算。
            // 次序约束：继任 effect 必须先 attach 再 begin，否则挂起解除会落到已卸载实例的宿主上。
            host = next;
        },
    };
};
