import assert from "node:assert/strict";
import test from "node:test";
import { createMusicSource, positionAt, lyricIndex } from "./musicState.ts";
import { parseMusicDetails } from "../../contracts/music.ts";

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
