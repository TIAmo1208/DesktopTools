import assert from "node:assert/strict";
import test from "node:test";
import { isCompactMusicLayout } from "./musicLayout.ts";

test("音乐布局仅按内容宽度切换，在滞回带内保留原状态", () => {
    assert.equal(isCompactMusicLayout(235, 13), true);
    assert.equal(isCompactMusicLayout(236, 13), false);
    assert.equal(isCompactMusicLayout(243, 13, true), true);
    assert.equal(isCompactMusicLayout(244, 13, true), false);
    assert.equal(isCompactMusicLayout(229, 13, false), false);
    assert.equal(isCompactMusicLayout(228, 13, false), true);
    assert.equal(isCompactMusicLayout(260, 18), true);
});
