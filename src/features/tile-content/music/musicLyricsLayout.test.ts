import assert from "node:assert/strict";
import test from "node:test";
import type { LyricLine } from "../../../contracts/music.ts";
import {
    MUSIC_LYRICS_GAP,
    MUSIC_LYRICS_LINE_HEIGHT,
    musicLyricsLineRows,
    musicLyricsPlacement,
    musicLyricsRowCount,
    musicLyricsWindow,
} from "./musicLyricsLayout.ts";

/** 生成指定行数的歌词，起始时间按顺序递增。 */
const lines = (count: number): LyricLine[] => Array.from({ length: count }, (_item, index) => ({ startMs: index * 1000, text: `第${index}句` }));

test("歌词区先看高度再看宽度，无同步词一律收起", () => {
    assert.equal(musicLyricsPlacement(500, 168, true), "bottom");
    assert.equal(musicLyricsPlacement(500, 167, true), "right");
    assert.equal(musicLyricsPlacement(419, 167, true), "none");
    assert.equal(musicLyricsPlacement(420, 167, true), "right");
    assert.equal(musicLyricsPlacement(900, 400, true), "bottom");
    assert.equal(musicLyricsPlacement(900, 400, false), "none");
    assert.equal(musicLyricsPlacement(0, 0, true), "none");
    // 精确边界：高度优先，宽度在 420 恰好进入右侧，152 与 404 是最内侧滞回边界。
    assert.equal(musicLyricsPlacement(420, 168, true), "bottom");
    assert.equal(musicLyricsPlacement(420, 167, true), "right");
    assert.equal(musicLyricsPlacement(500, 152, true, "bottom"), "bottom");
    assert.equal(musicLyricsPlacement(500, 151, true, "bottom"), "right");
    assert.equal(musicLyricsPlacement(404, 100, true, "right"), "right");
    assert.equal(musicLyricsPlacement(403, 100, true, "right"), "none");
});

test("当前布局在阈值内侧保留 16px 滞回，跨态时立即按新阈值判定", () => {
    // 已在下方：高度回落到 152 仍保留下方，低于 152 再考虑右侧。
    assert.equal(musicLyricsPlacement(500, 160, true, "bottom"), "bottom");
    assert.equal(musicLyricsPlacement(500, 151, true, "bottom"), "right");
    assert.equal(musicLyricsPlacement(300, 151, true, "bottom"), "none");
    // 已在右侧：宽度回落到 404 仍保留右侧，低于 404 收起。
    assert.equal(musicLyricsPlacement(410, 100, true, "right"), "right");
    assert.equal(musicLyricsPlacement(403, 100, true, "right"), "none");
    // 高度优先：右侧布局下高度达到 168 立即转下方，不再受宽度滞回影响。
    assert.equal(musicLyricsPlacement(410, 168, true, "right"), "bottom");
});

test("行数按实际高度累加，不设上下限", () => {
    assert.equal(musicLyricsRowCount(0), 0);
    assert.equal(musicLyricsRowCount(16), 0);
    assert.equal(musicLyricsRowCount(17), 1);
    assert.equal(musicLyricsRowCount(34), 1);
    assert.equal(musicLyricsRowCount(35), 2);
    assert.equal(musicLyricsRowCount(53), 3);
    assert.equal(musicLyricsRowCount(17 + 10 * (MUSIC_LYRICS_LINE_HEIGHT + MUSIC_LYRICS_GAP)), 11);
    assert.equal(musicLyricsRowCount(Number.NaN), 0);
    assert.equal(MUSIC_LYRICS_LINE_HEIGHT, 16);
});

test("翻译行只在能多容纳一行时预留", () => {
    assert.equal(musicLyricsLineRows(3, true), 2);
    assert.equal(musicLyricsLineRows(3, false), 3);
    assert.equal(musicLyricsLineRows(1, true), 1);
    assert.equal(musicLyricsLineRows(0, true), 0);
});

test("当前句居中、边界收缩、首句前不高亮", () => {
    const ten = lines(10); // 十句歌词。
    assert.deepEqual(musicLyricsWindow(ten, 5, 3), { start: 4, end: 7, current: 5 });
    assert.deepEqual(musicLyricsWindow(ten, 0, 3), { start: 0, end: 3, current: 0 });
    assert.deepEqual(musicLyricsWindow(ten, 9, 3), { start: 7, end: 10, current: 9 });
    assert.deepEqual(musicLyricsWindow(ten, -1, 3), { start: 0, end: 3, current: -1 });
    assert.deepEqual(musicLyricsWindow(lines(2), 1, 6), { start: 0, end: 2, current: 1 });
    assert.deepEqual(musicLyricsWindow(ten, 5, 2), { start: 4, end: 6, current: 5 });
    assert.deepEqual(musicLyricsWindow([], 0, 3), { start: 0, end: 0, current: -1 });
    assert.deepEqual(musicLyricsWindow(ten, 3, 0), { start: 0, end: 0, current: -1 });
});
