import type { IslandStateRequest } from "../../contracts/island.ts";
import type { LyricLine, MusicMediaSnapshot } from "../../contracts/music.ts";
import { lyricIndex, lyricTickDelay, positionAt, type MusicState } from "../music/musicState.ts";

/** 展开歌词区固定三行槽位：前一句、当前句、后一句。 */
export const ISLAND_LYRIC_OFFSETS = [-1, 0, 1] as const;

/** 空槽占位：用不换行空格保住行盒，避免无词或首末句时歌词区塌陷。 */
export const ISLAND_LYRIC_EMPTY = "\u00a0";

/** 展开态进度与当前句的刷新间隔；常态直接对齐下一句起始时刻，由 lyricTickDelay 给出。 */
export const ISLAND_EXPANDED_TICK_MS = 250;

/** 播放状态文案：没有媒体会话时是「未在播放」，也是歌词与歌名都缺失时的最后回落。 */
export const islandPlaybackStatus = (media: MusicMediaSnapshot | undefined): string => {
    if (!media) return "未在播放";
    switch (media.playback) {
        case "playing":
            return "正在播放";
        case "paused":
            return "已暂停";
        case "stopped":
            return "已停止";
        default:
            return "播放状态未知";
    }
};

/** 歌词不可用时的回落顺序：歌名 · 艺人，缺歌名与艺人时用播放状态；取词中、无匹配、纯音乐与失败都走这一条。 */
export const islandMusicFallback = (media: MusicMediaSnapshot | undefined): string =>
    [media?.title, media?.artist].filter(Boolean).join(" · ") || islandPlaybackStatus(media);

/** 浮岛歌词投影：常态一行与展开三行共用同一当前句，不各自推导。 */
export interface IslandLyricView {
    index: number; // 当前句下标；首句前、无词或取词中为 -1。
    line: string; // 常态胶囊的一行歌词，歌词不可用时为回落文本。
    hasLine: boolean; // 当前是否已有开始的歌词句；决定胶囊档位与滚动语义（无词走回落循环）。
    slots: readonly [string, string, string]; // 展开固定三行：前一句、当前句、后一句，空槽为占位符。
}

/** 按播放位置定位当前句并投影两种形态的文本；首句前当前句保持回落，末句后停在最后一句。 */
export const islandLyricView = (lines: readonly LyricLine[], positionMs: number, media: MusicMediaSnapshot | undefined): IslandLyricView => {
    const index = lyricIndex(lines, positionMs);
    const current = lines[index];
    const hasLine = current !== undefined && current.text.length > 0;
    const line = hasLine ? current.text : islandMusicFallback(media);
    const slot = (offset: number) => (offset === 0 ? line : lines[index + offset]?.text || ISLAND_LYRIC_EMPTY);
    return { index, line, hasLine, slots: [slot(-1), slot(0), slot(1)] };
};

/**
 * 浮岛歌词刷新延迟：只在常态（逐句对齐）与展开（进度与当前句）调度。
 * hover 不显示歌词与进度，暂停、无时间轴与轨道末端都不安排下一次刷新，避免定时器空转。
 */
export const islandMusicTickDelay = (
    mode: IslandStateRequest["state"],
    state: Pick<MusicState, "timeline" | "anchorNow">,
    lines: readonly LyricLine[],
    nowMs: number,
): number | undefined => {
    const timeline = state.timeline;
    if (!timeline?.advancing || mode === "hover") return undefined;
    const position = positionAt(state, nowMs);
    if (timeline.durationMs > 0 && position >= timeline.durationMs) return undefined;
    return mode === "expanded" ? ISLAND_EXPANDED_TICK_MS : lyricTickDelay(lines, position, timeline.durationMs);
};

/** 常态胶囊尺寸档位（胶囊外框 DIP）：按贴边方向与当前是否有同步词分两档。 */
export const ISLAND_CAPSULE_LIMITS = { horizontal: { lyric: 240, fallback: 168 }, vertical: { lyric: 176, fallback: 96 } } as const;

/** 胶囊固定件：内边距 8×2 ＋ 媒体图 28 ＋ 间距 8；内容滚动盒上限 = 档位 − 固定件。 */
export const ISLAND_CAPSULE_CHROME = 52;

/** 无词回落文本的循环周期：每像素约 100ms（往返各 20px/s），并设最短周期保证可读。 */
export const ISLAND_LOOP_MIN_MS = 4000;

/** 档位选择：只看贴边方向与当前是否有歌词句，不看滚动偏移与句子长度。 */
export const islandCapsuleLimit = (vertical: boolean, hasLine: boolean): number =>
    vertical
        ? hasLine
            ? ISLAND_CAPSULE_LIMITS.vertical.lyric
            : ISLAND_CAPSULE_LIMITS.vertical.fallback
        : hasLine
          ? ISLAND_CAPSULE_LIMITS.horizontal.lyric
          : ISLAND_CAPSULE_LIMITS.horizontal.fallback;

/** 内容滚动盒上限：胶囊档位减固定件；短内容贴合实际内容，不因档位撑满。 */
export const islandCapsuleLineLimit = (vertical: boolean, hasLine: boolean): number => islandCapsuleLimit(vertical, hasLine) - ISLAND_CAPSULE_CHROME;

/** 超长判定：内容尺寸超过滚动盒才滚动，取整避免亚像素反复触发。 */
export const islandScrollDistance = (contentSize: number, boxSize: number): number => {
    if (!Number.isFinite(contentSize) || !Number.isFinite(boxSize) || contentSize <= boxSize) return 0;
    return Math.round(contentSize - boxSize);
};

/** 常态胶囊的滚动计划：CSS 只消费方向、时长与播放状态，偏移由合成层动画推进。 */
export interface IslandScrollPlan {
    kind: "none" | "progress" | "loop";
    distance: number; // 需要位移的像素。
    durationMs: number; // 进度档为整句时长；循环档为一个往返周期。
    delayMs: number; // 进度档为已播放的句子时长，CSS 以负 animation-delay 应用。
    paused: boolean; // 暂停/停止时冻结在当前偏移。
}

const ISLAND_SCROLL_NONE: IslandScrollPlan = { kind: "none", distance: 0, durationMs: 0, delayMs: 0, paused: true };

/**
 * 常态胶囊滚动计划：
 * 有当前句且结束时间可审计时按播放进度映射（首句前无当前句、末句后夹紧到句尾）；
 * 无同步词与「零时间轴末句没有结束时间」都回退慢速循环，避免固定内容盒裁切；
 * 减少动效、无需滚动时返回 none，由 CSS 改为单行截断。
 * 循环档的偏移相位由 CSS 动画自行保持：暂停只切换 animation-play-state，不重建动画元素。
 */
export const islandScrollPlan = (input: {
    distance: number;
    hasLine: boolean;
    lineStartMs?: number;
    lineEndMs?: number;
    positionMs: number;
    advancing: boolean;
    animationsEnabled: boolean;
}): IslandScrollPlan => {
    const distance = Math.max(0, Math.round(input.distance));
    if (!input.animationsEnabled || distance <= 0 || !Number.isFinite(input.positionMs)) return ISLAND_SCROLL_NONE;
    const loop = (): IslandScrollPlan => ({
        kind: "loop",
        distance,
        durationMs: Math.max(ISLAND_LOOP_MIN_MS, distance * 100),
        delayMs: 0,
        paused: !input.advancing,
    });
    // 无词/首句前：回落文本慢速往返；零时间轴末句：没有可审计的结束时间，同样回退慢速循环而不裁切。
    if (!input.hasLine) return loop();
    const start = input.lineStartMs;
    const end = input.lineEndMs;
    if (start === undefined || end === undefined || !Number.isFinite(start) || !Number.isFinite(end) || end <= start) return loop();
    const elapsed = Math.min(Math.max(0, input.positionMs - start), end - start);
    return { kind: "progress", distance, durationMs: end - start, delayMs: elapsed, paused: !input.advancing };
};

/** 滚动动画名：横向用 *-x、竖向用 *-y；不需要滚动时不产出动画。 */
export const islandScrollAnimationName = (kind: IslandScrollPlan["kind"], vertical: boolean): string | undefined => {
    if (kind === "none") return undefined;
    const axis = vertical ? "y" : "x";
    return kind === "progress" ? `island-lyric-scroll-${axis}` : `island-lyric-loop-${axis}`;
};

/** 播放/暂停映射：暂停只冻结现有动画，不重置偏移。 */
export const islandScrollPlayState = (paused: boolean): "paused" | "running" => (paused ? "paused" : "running");

/**
 * 动画元素重建键：换句、切歌、贴边方向、溢出距离、时长或进度偏移变化时重建，按新偏移重新对齐。
 * 进度档把暂停纳入键，重建后以冻结位置重新挂载；无词循环不纳入暂停，保持 CSS 动画相位不回到起点。
 */
export const islandScrollKey = (input: { sessionId: string; index: number; vertical: boolean; plan: IslandScrollPlan }): string => {
    const { plan } = input;
    const phase = plan.kind === "progress" ? (plan.paused ? "pause" : "play") : "loop";
    return [input.sessionId, input.index, input.vertical ? "v" : "h", plan.kind, plan.distance, plan.durationMs, Math.round(plan.delayMs / 100), phase].join(
        ":",
    );
};
