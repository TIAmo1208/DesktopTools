import type { LyricLine } from "../../../contracts/music.ts";

/** 歌词区位置：下方通栏、右侧固定列或收起。 */
export type MusicLyricsPlacement = "bottom" | "right" | "none";

export const MUSIC_LYRICS_MIN_HEIGHT = 168; // 高度达标优先使用下方通栏。
export const MUSIC_LYRICS_MIN_WIDTH = 420; // 高度不足时宽度达标使用右侧固定列。
export const MUSIC_LYRICS_HYSTERESIS = 16; // 与音乐布局模式共用同一滞回宽度。
export const MUSIC_LYRICS_LINE_HEIGHT = 16; // 其余句 13px 的行盒高度。
export const MUSIC_LYRICS_CURRENT_HEIGHT = 17; // 当前句 14px 的行盒高度。
export const MUSIC_LYRICS_GAP = 2; // 行间距。

/** 先看高度再看宽度；当前布局在阈值内侧保留 16px 滞回，避免拖动时反复切换。 */
export const musicLyricsPlacement = (width: number, height: number, hasLyrics: boolean, current: MusicLyricsPlacement = "none"): MusicLyricsPlacement => {
    if (!hasLyrics) return "none"; // 取词中、无匹配、失败与无会话都收起。
    const minHeight = current === "bottom" ? MUSIC_LYRICS_MIN_HEIGHT - MUSIC_LYRICS_HYSTERESIS : MUSIC_LYRICS_MIN_HEIGHT;
    if (height >= minHeight) return "bottom";
    const minWidth = current === "right" ? MUSIC_LYRICS_MIN_WIDTH - MUSIC_LYRICS_HYSTERESIS : MUSIC_LYRICS_MIN_WIDTH;
    if (width >= minWidth) return "right";
    return "none";
};

/** 按可用高度累加能放几行：n 行总高 = 17 + (n − 1) × (16 + 2)。 */
export const musicLyricsRowCount = (height: number): number => {
    if (!Number.isFinite(height) || height < MUSIC_LYRICS_CURRENT_HEIGHT) return 0;
    return Math.floor((height - MUSIC_LYRICS_CURRENT_HEIGHT) / (MUSIC_LYRICS_LINE_HEIGHT + MUSIC_LYRICS_GAP)) + 1;
};

/** 当前句有翻译时预留一行；只有一行高度时不挤掉当前句。 */
export const musicLyricsLineRows = (rows: number, hasTranslation: boolean): number => (hasTranslation && rows > 1 ? rows - 1 : Math.max(0, rows));

/** 以当前句为中心截取可见窗口；首句前从第一行开始且不高亮，窗口不越界。 */
export const musicLyricsWindow = (lines: readonly LyricLine[], index: number, rows: number) => {
    const visible = Math.min(rows, lines.length); // 实际可显示的行数。
    if (visible <= 0) return { start: 0, end: 0, current: -1 };
    const anchor = index < 0 ? 0 : Math.min(index, lines.length - 1); // 当前句在数组内的位置。
    const start = Math.min(Math.max(0, anchor - Math.floor(visible / 2)), lines.length - visible); // 居中后贴边收缩，翻译行让当前句仍在上半部。
    return { start, end: start + visible, current: index < 0 ? -1 : anchor };
};
