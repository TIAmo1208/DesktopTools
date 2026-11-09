import assert from "node:assert/strict";
import test from "node:test";
import { isCompactMusicLayout, musicMediaMinimum } from "./musicLayout.ts";

test("音乐布局仅按内容宽度切换，在滞回带内保留原状态", () => {
    assert.equal(isCompactMusicLayout(235, 13), true);
    assert.equal(isCompactMusicLayout(236, 13), false);
    assert.equal(isCompactMusicLayout(243, 13, true), true);
    assert.equal(isCompactMusicLayout(244, 13, true), false);
    assert.equal(isCompactMusicLayout(229, 13, false), false);
    assert.equal(isCompactMusicLayout(228, 13, false), true);
    assert.equal(isCompactMusicLayout(260, 18), true);
});

test("正文最小高度计入频谱上下外部间距与紧凑绝对定位差异", () => {
    // 频谱关闭：默认字号取 94 保底，紧凑内边距也回到同一高度。
    assert.equal(musicMediaMinimum(52, 30, 0, 0, false, false), 94);
    assert.equal(musicMediaMinimum(52, 30, 6, 6, false, true), 94);
    // 频谱打开：宽布局条带上下各 6px，紧凑绝对定位只留 6px 下间距。
    assert.equal(musicMediaMinimum(52, 30, 0, 0, true, false), 112);
    assert.equal(musicMediaMinimum(52, 30, 6, 0, true, true), 112);
    // 大字号抬高信息区，宽与紧凑都按完整外部占用计算。
    assert.equal(musicMediaMinimum(70, 30, 0, 0, true, false), 130);
    assert.equal(musicMediaMinimum(70, 30, 6, 0, true, true), 130);
    assert.equal(musicMediaMinimum(76, 30, 6, 0, true, true), 136);
});
