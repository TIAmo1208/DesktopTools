import assert from "node:assert/strict";
import test from "node:test";
import { parseMusicSnapshot } from "./music.ts";

test("媒体 IPC 区分无会话、合法媒体和无效数据", () => {
    assert.equal(parseMusicSnapshot({ media: null }), undefined);
    const media = {
        sessionId: "1",
        title: "歌曲",
        artist: "艺人",
        source: "播放器",
        playback: "paused",
        canPrevious: false,
        canPlayPause: true,
        canNext: false,
        artworkUrl: null,
    }; // 合法后端载荷。
    assert.equal(parseMusicSnapshot({ media })?.playback, "paused");
    assert.equal(parseMusicSnapshot({ media })?.sessionId, "1");
    assert.throws(() => parseMusicSnapshot({ media: null, error: "连接失败" }), /连接失败/);
    assert.throws(() => parseMusicSnapshot({ media: { ...media, sessionId: "" } }));
    for (const value of [
        null,
        {},
        { media: {} },
        { media: { ...media, canNext: "false" } },
        { media: { ...media, playback: "invalid" } },
        { media: { ...media, artworkUrl: "https://example.com/image" } },
    ])
        assert.throws(() => parseMusicSnapshot(value));
});
