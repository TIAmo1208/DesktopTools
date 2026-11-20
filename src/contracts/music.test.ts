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
        { media: { ...media, seekMinMs: "0" } },
    ])
        assert.throws(() => parseMusicSnapshot(value));
});

test("跳转区间缺省或 null 都表示播放器没给区间，不能因此拒绝整份快照", () => {
    const media = { sessionId: "1", title: "歌曲", artist: "艺人", source: "播放器", playback: "playing", canPrevious: false, canPlayPause: true, canNext: false, canSeek: false };
    assert.equal(parseMusicSnapshot({ media })?.seekMinMs, undefined);
    const withNull = parseMusicSnapshot({ media: { ...media, seekMinMs: null, seekMaxMs: null } });
    assert.equal(withNull?.seekMinMs, undefined);
    assert.equal(withNull?.seekMaxMs, undefined);
    const ranged = parseMusicSnapshot({ media: { ...media, canSeek: true, seekMinMs: 0, seekMaxMs: 120_000 } });
    assert.equal(ranged?.seekMaxMs, 120_000);
});
