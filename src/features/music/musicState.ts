import type { LyricLine, MusicDetails, MusicMediaSnapshot, MusicTimeline } from "../../contracts/music.ts";

/** 页面级共享事实，锚点使用本地单调毫秒，不使用系统墙钟推进。 */
export interface MusicState {
    snapshot?: MusicMediaSnapshot;
    timeline?: MusicTimeline;
    lyrics: LyricLine[];
    error?: unknown;
    anchorNow: number;
}

/** 在渲染时投影，不创建计时器或媒体轮询；durationMs 为 0 表示播放器不报时长，不能据此把位置夹到 0。 */
export const positionAt = (state: Pick<MusicState, "timeline" | "anchorNow">, now: number): number => {
    const value = state.timeline;
    if (!value) return 0;
    const projected = Math.max(0, value.positionMs + (value.advancing ? Math.max(0, now - state.anchorNow) : 0));
    return value.durationMs > 0 ? Math.min(value.durationMs, projected) : projected;
};

/** 找到最后一个已开始的句子，首句前保持空态。 */
export const lyricIndex = (lines: readonly LyricLine[], positionMs: number): number => {
    let low = 0,
        high = lines.length;
    while (low < high) {
        const middle = (low + high) >>> 1;
        if (lines[middle]!.startMs <= positionMs) low = middle + 1;
        else high = middle;
    }
    return low - 1;
};

/**
 * 下一句刷新延迟；durationMs 为 0（或非有限）表示播放器不报时长，只以「没有下一句」为停止条件，
 * 避免零时长把歌词跟随整体冻结；已知时长仍按轨道末端与下一句是否越界停止。
 */
export const lyricTickDelay = (lines: readonly LyricLine[], positionMs: number, durationMs: number): number | undefined => {
    if (!Number.isFinite(positionMs) || positionMs < 0) return undefined;
    const knownDuration = Number.isFinite(durationMs) && durationMs > 0;
    if (knownDuration && positionMs >= durationMs) return undefined;
    const next = lines[lyricIndex(lines, positionMs) + 1]?.startMs;
    if (next === undefined) return undefined;
    if (knownDuration && next > durationMs) return undefined;
    return Math.max(16, next - positionMs);
};

/** 播放量程：真实时长优先；播放器不报时长时用歌词末行 startMs 兜底，仍未知返回 0（显示占位，不画假进度）。 */
export const musicRangeMs = (timeline: Pick<MusicTimeline, "durationMs"> | undefined, lyrics: readonly LyricLine[]): number => {
    const duration = timeline?.durationMs ?? 0;
    if (Number.isFinite(duration) && duration > 0) return duration;
    const lastStart = lyrics.length ? lyrics[lyrics.length - 1]!.startMs : 0;
    return Number.isFinite(lastStart) && lastStart > 0 ? lastStart : 0;
};

/** 进度条投影：量程未知时给空条（max 1、value 0）与 totalMs=0，供 UI 显示占位而不出现 0/0 满格。 */
export const musicProgressRange = (
    timeline: Pick<MusicTimeline, "durationMs"> | undefined,
    lyrics: readonly LyricLine[],
    positionMs: number,
): { max: number; value: number; totalMs: number } => {
    const totalMs = musicRangeMs(timeline, lyrics);
    return totalMs > 0 ? { max: totalMs, value: Math.min(Math.max(0, positionMs), totalMs), totalMs } : { max: 1, value: 0, totalMs: 0 };
};

type Connect = (
    snapshot: (value: MusicMediaSnapshot | undefined) => void,
    details: (value: MusicDetails) => void,
    error: (value: unknown) => void,
) => Promise<() => void>;

/** 引用计数隔离挂载代次；仅工厂用于无播放器测试，生产只创建一个实例。 */
export const createMusicSource = (connect: Connect, now: () => number) => {
    let state: MusicState = { lyrics: [], anchorNow: 0 };
    const listeners = new Set<() => void>();
    let generation = 0;
    let stop: (() => void) | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined; // 仅订阅握手失败时重连，正常运行不轮询。
    const emit = () => {
        for (const listener of listeners) listener();
    };
    const subscribe = (listener: () => void) => {
        listeners.add(listener);
        if (listeners.size === 1) {
            const current = ++generation;
            const valid = () => current === generation && listeners.size > 0;
            const start = () => {
                if (!valid()) return;
                void connect(
                    (snapshot) => {
                        if (!valid()) return;
                        const matches =
                            snapshot &&
                            state.timeline &&
                            snapshot.sessionId === state.timeline.sessionId &&
                            snapshot.title === state.timeline.title &&
                            snapshot.artist === state.timeline.artist;
                        state = { ...state, snapshot, error: undefined, ...(matches ? {} : { timeline: undefined, lyrics: [] }) };
                        emit();
                    },
                    (details) => {
                        if (!valid()) return;
                        const media = state.snapshot,
                            timeline = details.timeline;
                        if (media && timeline && (media.sessionId !== timeline.sessionId || media.title !== timeline.title || media.artist !== timeline.artist))
                            return;
                        state = { ...state, ...details, anchorNow: now() };
                        emit();
                    },
                    (error) => {
                        if (!valid()) return;
                        state = { ...state, error };
                        emit();
                    },
                ).then(
                    (dispose) => {
                        if (!valid()) dispose();
                        else stop = dispose;
                    },
                    (error) => {
                        if (valid()) {
                            state = { ...state, error };
                            emit();
                            retry = setTimeout(start, 1000);
                        }
                    },
                );
            };
            start();
        }
        let active = true;
        return () => {
            if (!active) return;
            active = false;
            listeners.delete(listener);
            if (!listeners.size) {
                generation++;
                clearTimeout(retry);
                stop?.();
                stop = undefined;
                state = { lyrics: [], anchorNow: 0 };
            }
        };
    };
    return { subscribe, getSnapshot: () => state };
};
