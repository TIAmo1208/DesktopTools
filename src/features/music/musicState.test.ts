import assert from "node:assert/strict";
import test from "node:test";
import { createMusicSource, lyricTickDelay, musicProgressRange, musicRangeMs, positionAt, lyricIndex } from "./musicState.ts";
import { parseMusicDetails, type MusicMediaSnapshot } from "../../contracts/music.ts";

const raw = {
    timeline: {
        sessionId: "1",
        title: "歌",
        artist: "人",
        positionTicks: "10000000",
        endTimeTicks: "100000000",
        lastUpdatedTimeTicks: "134000000000000000",
        playbackStatus: 4,
        positionMs: 1000,
        durationMs: 10000,
        advancing: true,
    },
    lyrics: [
        { startMs: 1000, text: "甲" },
        { startMs: 3000, text: "乙" },
    ],
};

test("锚点推进、暂停冻结、时长夹紧与句子边界", () => {
    const details = parseMusicDetails(raw);
    const state = { ...details, anchorNow: 100 };
    assert.equal(positionAt(state, 600), 1500);
    assert.equal(positionAt(state, 99999), 10000);
    assert.equal(positionAt({ ...state, timeline: { ...raw.timeline, advancing: false } }, 600), 1000);
    assert.equal(lyricIndex(details.lyrics, 999), -1);
    assert.equal(lyricIndex(details.lyrics, 1000), 0);
    assert.equal(lyricIndex(details.lyrics, 3000), 1);
    assert.equal(lyricIndex([], 3000), -1);
});

test("零时长（播放器不报时长）不夹位置，量程按歌词末行兜底", () => {
    const zeroRaw = { ...raw, timeline: { ...raw.timeline, positionMs: 3000, durationMs: 0, endTimeTicks: "0", lastUpdatedTimeTicks: "0" } };
    const zero = { ...parseMusicDetails(zeroRaw), anchorNow: 0 };
    assert.equal(positionAt(zero, 0), 3000);
    assert.equal(positionAt(zero, 2000), 5000); // 不因 duration=0 被夹到 0。
    assert.equal(positionAt({ ...zero, timeline: { ...zero.timeline!, advancing: false } }, 9000), 3000); // 暂停冻结。
    // 已知时长仍夹紧。
    assert.equal(positionAt({ timeline: { ...raw.timeline, positionMs: 9000, durationMs: 10000, advancing: true }, anchorNow: 0 }, 5000), 10000);
    // 量程：真实时长优先，否则歌词末行 startMs，再否则 0（显示占位）。
    assert.equal(musicRangeMs({ durationMs: 0 }, zero.lyrics), 3000);
    assert.equal(musicRangeMs({ durationMs: 10000 }, zero.lyrics), 10000);
    assert.equal(musicRangeMs({ durationMs: 0 }, []), 0);
    assert.equal(musicRangeMs(undefined, zero.lyrics), 3000);
    assert.equal(musicRangeMs({ durationMs: 0 }, [{ startMs: 0, text: "甲" }]), 0); // 末行在 0 时无法兜底。
    // 进度条投影：未知时长但有歌词 → 量程取末行 startMs 并夹紧；未知且无歌词 → 空条 1/0 与占位。
    const lyricLines = [
        { startMs: 1000, text: "甲" },
        { startMs: 3000, text: "乙" },
    ];
    assert.deepEqual(musicProgressRange({ durationMs: 0 }, lyricLines, 2000), { max: 3000, value: 2000, totalMs: 3000 });
    assert.deepEqual(musicProgressRange({ durationMs: 0 }, lyricLines, 9000), { max: 3000, value: 3000, totalMs: 3000 });
    assert.deepEqual(musicProgressRange({ durationMs: 0 }, lyricLines, -5), { max: 3000, value: 0, totalMs: 3000 });
    assert.deepEqual(musicProgressRange({ durationMs: 0 }, [], 9000), { max: 1, value: 0, totalMs: 0 });
    assert.deepEqual(musicProgressRange({ durationMs: 10000 }, lyricLines, 12000), { max: 10000, value: 10000, totalMs: 10000 });
});

test("第二事实严格拒绝类型、无穷与乱序歌词", () => {
    for (const patch of [{ positionMs: "1000" }, { durationMs: NaN }, { advancing: 1 }, { lastUpdatedTimeTicks: 123 }, { playbackStatus: "4" }])
        assert.throws(() => parseMusicDetails({ ...raw, timeline: { ...raw.timeline, ...patch } }));
    assert.throws(() =>
        parseMusicDetails({
            ...raw,
            lyrics: [
                { startMs: 1, text: "甲" },
                { startMs: 1, text: "乙" },
            ],
        }),
    );
    assert.deepEqual(parseMusicDetails({ timeline: null, lyrics: [] }), { timeline: undefined, lyrics: [] });
    // 零时长表示未知：位置可以大于 0；已知时长小于位置仍拒绝。
    assert.equal(parseMusicDetails({ ...raw, timeline: { ...raw.timeline, durationMs: 0, positionMs: 5000 } }).timeline?.durationMs, 0);
    // 负数时长无效（0 才表示未知），不能因放宽零时长而接受负值。
    assert.throws(() => parseMusicDetails({ ...raw, timeline: { ...raw.timeline, durationMs: -1, positionMs: 0 } }));
    assert.throws(() => parseMusicDetails({ ...raw, timeline: { ...raw.timeline, durationMs: -0.001, positionMs: 0 } }));
    assert.throws(() => parseMusicDetails({ ...raw, timeline: { ...raw.timeline, durationMs: 1000, positionMs: 2000 } }));
    const translated = parseMusicDetails({ ...raw, lyrics: [{ startMs: 1000, text: "甲", translation: "A" }] });
    assert.equal(translated.lyrics[0]?.translation, "A");
    assert.throws(() => parseMusicDetails({ ...raw, lyrics: [{ startMs: 1000, text: "甲", translation: 1 }] }));
});

test("逐句刷新在轨道末端或下一句超出时长时停止", () => {
    const lines = [
        { startMs: 0, text: "甲" },
        { startMs: 1000, text: "乙" },
    ];
    assert.equal(lyricTickDelay(lines, 0, 5000), 1000);
    assert.equal(lyricTickDelay(lines, 1000, 5000), undefined);
    assert.equal(lyricTickDelay(lines, 5000, 5000), undefined);
    assert.equal(lyricTickDelay([{ startMs: 5001, text: "尾" }], 0, 5000), undefined);
    assert.equal(lyricTickDelay([], 0, 5000), undefined);
    // 零时长表示播放器不报时长：只以「没有下一句」停止，不能把歌词跟随冻结在 0。
    assert.equal(lyricTickDelay(lines, 0, 0), 1000);
    assert.equal(lyricTickDelay(lines, 0, Number.NaN), 1000);
    // 下一句恰在轨末允许最后一次刷新，剩余不足 16ms 时按最小间隔。
    assert.equal(
        lyricTickDelay(
            [
                { startMs: 0, text: "甲" },
                { startMs: 5000, text: "乙" },
            ],
            0,
            5000,
        ),
        5000,
    );
    assert.equal(
        lyricTickDelay(
            [
                { startMs: 0, text: "甲" },
                { startMs: 1008, text: "乙" },
            ],
            1000,
            5000,
        ),
        16,
    );
    // 非有限或负位置不调度；零/负/无限时长按未知处理，仍按下一句调度。
    for (const position of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, -1]) assert.equal(lyricTickDelay(lines, position, 5000), undefined);
    for (const duration of [0, -1, Number.POSITIVE_INFINITY, Number.NaN]) assert.equal(lyricTickDelay(lines, 0, duration), 1000);
});

test("格子与岛消费者共用一条订阅，最后退订销毁，迟到事实被拒绝", async () => {
    let connections = 0,
        disposals = 0;
    let publish: ((value: ReturnType<typeof parseMusicDetails>) => void) | undefined;
    const source = createMusicSource(
        async (_snapshot, details) => {
            connections++;
            publish = details;
            return () => {
                disposals++;
            };
        },
        () => 500,
    );
    const tile = source.subscribe(() => {});
    const island = source.subscribe(() => {});
    await Promise.resolve();
    assert.equal(connections, 1);
    publish!(parseMusicDetails(raw));
    assert.equal(source.getSnapshot().anchorNow, 500);
    tile();
    assert.equal(disposals, 0);
    island();
    assert.equal(disposals, 1);
    publish!(parseMusicDetails(raw));
    assert.equal(source.getSnapshot().timeline, undefined);
    const remount = source.subscribe(() => {});
    await Promise.resolve();
    assert.equal(connections, 2);
    remount();
});

test("订阅握手完成前卸载仍释放迟到句柄", async () => {
    let resolve: ((stop: () => void) => void) | undefined;
    let disposed = false;
    const source = createMusicSource(
        () =>
            new Promise((done) => {
                resolve = done;
            }),
        () => 0,
    );
    source.subscribe(() => {})();
    resolve!(() => {
        disposed = true;
    });
    await Promise.resolve();
    assert.equal(disposed, true);
});

test("切歌清空上一首时间轴与歌词，同一会话的快照更新保留", async () => {
    let publishSnapshot: ((value: MusicMediaSnapshot | undefined) => void) | undefined;
    let publishDetails: ((value: ReturnType<typeof parseMusicDetails>) => void) | undefined;
    const source = createMusicSource(
        async (snapshot, details) => {
            publishSnapshot = snapshot;
            publishDetails = details;
            return () => {};
        },
        () => 500,
    );
    const stop = source.subscribe(() => {});
    await Promise.resolve();
    publishSnapshot!({
        sessionId: "1",
        title: "歌",
        artist: "人",
        source: "播放器",
        playback: "playing",
        canPrevious: true,
        canPlayPause: true,
        canNext: true,
    });
    publishDetails!(parseMusicDetails(raw));
    assert.equal(source.getSnapshot().lyrics.length, 2);
    // 同一会话的播放状态变化只是快照更新，不能丢时间轴或歌词。
    publishSnapshot!({
        sessionId: "1",
        title: "歌",
        artist: "人",
        source: "播放器",
        playback: "paused",
        canPrevious: true,
        canPlayPause: true,
        canNext: true,
    });
    assert.notEqual(source.getSnapshot().timeline, undefined);
    assert.equal(source.getSnapshot().lyrics.length, 2);
    // 切歌立即清空上一首的时间轴与歌词，等待新歌事实。
    publishSnapshot!({
        sessionId: "2",
        title: "新歌",
        artist: "新人",
        source: "播放器",
        playback: "playing",
        canPrevious: true,
        canPlayPause: true,
        canNext: true,
    });
    assert.equal(source.getSnapshot().timeline, undefined);
    assert.deepEqual(source.getSnapshot().lyrics, []);
    // 新歌时间轴到达前，迟到的旧歌时间轴被身份校验拒绝，不写回。
    publishDetails!(parseMusicDetails(raw));
    assert.equal(source.getSnapshot().timeline, undefined);
    assert.deepEqual(source.getSnapshot().lyrics, []);
    stop();
});
