import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { MusicMediaSnapshot, MusicTimeline } from "../../contracts/music.ts";
import { parseMusicDetails } from "../../contracts/music.ts";
import { positionAt } from "../music/musicState.ts";
import {
    ISLAND_CAPSULE_CHROME,
    ISLAND_CAPSULE_LIMITS,
    ISLAND_EXPANDED_TICK_MS,
    ISLAND_LOOP_MIN_MS,
    ISLAND_LYRIC_EMPTY,
    ISLAND_LYRIC_OFFSETS,
    islandCapsuleLimit,
    islandCapsuleLineLimit,
    islandLyricView,
    islandMusicFallback,
    islandMusicTickDelay,
    islandPlaybackStatus,
    islandScrollAnimationName,
    islandScrollDistance,
    islandScrollKey,
    islandScrollPlan,
    islandScrollPlayState,
} from "./islandLyrics.ts";

/** 构造与后端快照同形的媒体事实，只覆盖断言需要的字段。 */
const media = (patch: Partial<MusicMediaSnapshot> = {}): MusicMediaSnapshot => ({
    sessionId: "1",
    title: "起风了",
    artist: "买辣椒也用券",
    source: "网易云音乐",
    playback: "playing",
    canPrevious: true,
    canPlayPause: true,
    canNext: true,
    ...patch,
});

/** 构造与后端投影同形的时间轴，默认从 1s 起播、总长 10s。 */
const timeline = (patch: Partial<MusicTimeline> = {}): MusicTimeline => ({
    sessionId: "1",
    title: "起风了",
    artist: "买辣椒也用券",
    positionTicks: "10000000",
    endTimeTicks: "100000000",
    lastUpdatedTimeTicks: "134000000000000000",
    playbackStatus: 4,
    positionMs: 1000,
    durationMs: 10000,
    advancing: true,
    ...patch,
});

/** 三句同步歌词：1s、3s、5s 各一句；末句之后没有下一句。 */
const lines = parseMusicDetails({
    timeline: timeline(),
    lyrics: [
        { startMs: 1000, text: "这一路上走走停停" },
        { startMs: 3000, text: "顺着少年漂流的痕迹" },
        { startMs: 5000, text: "迈出车站的前一刻" },
    ],
}).lyrics;

/** 常态一行与展开三行都来自同一个投影。 */
const view = (positionMs: number, snapshot: MusicMediaSnapshot | undefined = media()) => islandLyricView(lines, positionMs, snapshot);

test("当前句按播放位置更新，首句前回落、末句后停在最后一句", () => {
    assert.equal(view(0).index, -1);
    assert.equal(view(999).index, -1);
    assert.deepEqual(view(0), {
        index: -1,
        hasLine: false,
        line: "起风了 · 买辣椒也用券",
        slots: [ISLAND_LYRIC_EMPTY, "起风了 · 买辣椒也用券", "这一路上走走停停"],
    }); // 首句前当前行回落，下一句已在后一个槽位。
    assert.deepEqual(view(1000), { index: 0, hasLine: true, line: "这一路上走走停停", slots: [ISLAND_LYRIC_EMPTY, "这一路上走走停停", "顺着少年漂流的痕迹"] });
    assert.deepEqual(view(2999), { index: 0, hasLine: true, line: "这一路上走走停停", slots: [ISLAND_LYRIC_EMPTY, "这一路上走走停停", "顺着少年漂流的痕迹"] });
    assert.deepEqual(view(3000), {
        index: 1,
        hasLine: true,
        line: "顺着少年漂流的痕迹",
        slots: ["这一路上走走停停", "顺着少年漂流的痕迹", "迈出车站的前一刻"],
    });
    // 末句之后停在最后一句，后一个槽位只留占位符。
    assert.deepEqual(view(9000), { index: 2, hasLine: true, line: "迈出车站的前一刻", slots: ["顺着少年漂流的痕迹", "迈出车站的前一刻", ISLAND_LYRIC_EMPTY] });
    assert.deepEqual(view(10000), view(9000));
});

test("展开歌词区恒定三行，空槽用占位符，当前槽始终等于常态一行", () => {
    assert.deepEqual([...ISLAND_LYRIC_OFFSETS], [-1, 0, 1]);
    for (const position of [0, 1000, 2999, 3000, 5000, 9999, 10000]) {
        const current = view(position);
        assert.equal(current.slots.length, 3, `${position}ms 的歌词槽位必须恒定为三行`);
        assert.equal(current.slots[1], current.line, `${position}ms 的当前槽必须与常态一行一致`);
        for (const [slot, text] of current.slots.entries()) assert.ok(text.length > 0, `${position}ms 第 ${slot} 行不得为空`);
        // 空槽内容只能是占位符，不出现空字符串导致的槽位塌陷。
        for (const [slot, text] of current.slots.entries()) if (slot !== 1) assert.ok(text === ISLAND_LYRIC_EMPTY || lines.some((line) => line.text === text));
    }
});

test("按播放位置逐句推进：锚点推进只在跨句时改变当前行", () => {
    const state = { timeline: timeline(), anchorNow: 0 };
    const at = (nowMs: number) => islandLyricView(lines, positionAt(state, nowMs), media());
    assert.equal(at(0).line, "这一路上走走停停"); // 起点 1000ms。
    assert.equal(at(1999).line, "这一路上走走停停");
    assert.equal(at(2000).line, "顺着少年漂流的痕迹"); // 3000ms。
    assert.equal(at(4000).line, "迈出车站的前一刻"); // 5000ms。
    assert.equal(at(60000).line, "迈出车站的前一刻"); // 超过时长夹紧在轨道末端。
    // 暂停冻结在时间轴位置，不随本地时钟继续推进。
    const paused = { timeline: timeline({ advancing: false, positionMs: 1500 }), anchorNow: 0 };
    assert.equal(positionAt(paused, 0), 1500);
    assert.equal(positionAt(paused, 90000), 1500);
    assert.equal(islandLyricView(lines, positionAt(paused, 90000), media()).line, "这一路上走走停停");
    // 跳转后立即以新时间轴对齐，不保留上一位置的行。
    assert.equal(islandLyricView(lines, positionAt({ timeline: timeline({ positionMs: 5200 }), anchorNow: 0 }, 0), media()).index, 2);
});

test("无词、纯音乐、取词中与失败都回落到固定文本，不出现空白", () => {
    const cases: [string, MusicMediaSnapshot | undefined, string][] = [
        ["无会话", undefined, "未在播放"],
        ["无词可回落歌名·艺人", media({ playback: "playing" }), "起风了 · 买辣椒也用券"],
        ["纯音乐等空歌词", media({ playback: "playing" }), "起风了 · 买辣椒也用券"],
        ["取词失败只剩艺人", media({ title: "", artist: "买辣椒也用券", playback: "paused" }), "买辣椒也用券"],
        ["歌名与艺人都缺失", media({ title: "", artist: "", playback: "paused" }), "已暂停"],
        ["停止状态", media({ title: "", artist: "", playback: "stopped" }), "已停止"],
        ["播放状态未知", media({ title: "", artist: "", playback: "unknown" }), "播放状态未知"],
    ];
    for (const [label, snapshot, expected] of cases) {
        assert.equal(islandMusicFallback(snapshot), expected, label);
        const empty = islandLyricView([], 3000, snapshot); // 取词中、无匹配、纯音乐与失败都表现为空歌词。
        assert.equal(empty.index, -1, label);
        assert.equal(empty.hasLine, false, label);
        assert.equal(empty.line, expected, label);
        assert.deepEqual(empty.slots, [ISLAND_LYRIC_EMPTY, expected, ISLAND_LYRIC_EMPTY], label);
    }
    // 播放状态文案与媒体状态一一对应，缺会话才显示「未在播放」。
    assert.equal(islandPlaybackStatus(undefined), "未在播放");
    assert.equal(islandPlaybackStatus(media({ playback: "playing" })), "正在播放");
});

test("切歌后没有新歌词时回落歌名，不残留上一首的句子", () => {
    const next = media({ sessionId: "2", title: "新歌", artist: "新人" });
    const empty = islandLyricView([], 3000, next);
    assert.equal(empty.hasLine, false);
    assert.equal(empty.line, "新歌 · 新人");
    assert.deepEqual(empty.slots, [ISLAND_LYRIC_EMPTY, "新歌 · 新人", ISLAND_LYRIC_EMPTY]);
});

test("刷新只在常态逐句与展开进度调度，暂停、hover 与轨道末端不空转", () => {
    const state = { timeline: timeline(), anchorNow: 0 };
    // 常态对齐下一句起始时刻：1000ms 起播时下一句在 3000ms。
    assert.equal(islandMusicTickDelay("collapsed", state, lines, 0), 2000);
    assert.equal(islandMusicTickDelay("collapsed", { ...state, anchorNow: 2000 }, lines, 2000), 2000);
    // 展开按固定间隔推进进度与当前句。
    assert.equal(islandMusicTickDelay("expanded", state, lines, 0), ISLAND_EXPANDED_TICK_MS);
    // hover 不显示歌词与进度，不订阅定时器。
    assert.equal(islandMusicTickDelay("hover", state, lines, 0), undefined);
    // 暂停、无时间轴、轨道末端与没有下一句时都不安排下一次刷新。
    assert.equal(islandMusicTickDelay("collapsed", { timeline: timeline({ advancing: false }), anchorNow: 0 }, lines, 10000), undefined);
    assert.equal(islandMusicTickDelay("expanded", { timeline: timeline({ advancing: false }), anchorNow: 0 }, lines, 10000), undefined);
    assert.equal(islandMusicTickDelay("expanded", { timeline: undefined, anchorNow: 0 }, lines, 0), undefined);
    assert.equal(islandMusicTickDelay("collapsed", state, lines, 9000), undefined);
    assert.equal(islandMusicTickDelay("expanded", state, lines, 9000), undefined);
    // 无词时常态不调度，展开仍推进进度（歌词区用回落文本占位）。
    assert.equal(islandMusicTickDelay("collapsed", state, [], 0), undefined);
    assert.equal(islandMusicTickDelay("expanded", state, [], 0), ISLAND_EXPANDED_TICK_MS);
    // 零时长（播放器不报时长）按未知处理：不按轨道末端停止，展开仍按 250ms、常态按下一句调度。
    const zero = { timeline: timeline({ durationMs: 0, positionMs: 0 }), anchorNow: 0 };
    assert.equal(islandMusicTickDelay("expanded", zero, lines, 0), ISLAND_EXPANDED_TICK_MS);
    assert.equal(islandMusicTickDelay("collapsed", zero, lines, 0), 1000);
    // 没有下一句时才停止。
    assert.equal(islandMusicTickDelay("collapsed", zero, lines, 6000), undefined);
});

test("岛音乐卡调用点消费投影与调度，回退到内联定时或旧投影会红", () => {
    const component = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    // 静态接线守卫：调用点仍必须引用被测的调度与投影函数；运行时是否真的不刷新仍需断言页或实机核对。
    assert.match(component, /import \{[^}]*islandLyricView[^}]*islandMusicTickDelay[^}]*\} from "\.\/islandLyrics";/);
    assert.match(component, /islandMusicTickDelay\(mode, music, music\.lyrics, current\)/);
    assert.match(component, /timer = setTimeout\(refresh, delay\)/);
    assert.doesNotMatch(component, /lyricTickDelay\(/); // 不再在组件内直接内联行定位定时。
    assert.doesNotMatch(component, /mode === "expanded" \? 250/); // 展开间隔只在 islandMusicTickDelay 内。
    // 常态一行与展开三行必须来自同一投影，不能各自推导。
    assert.match(component, /ISLAND_LYRIC_OFFSETS\.map\(\(offset, slot\)/);
    assert.match(component, /\{view\.slots\[slot\]\}/);
    assert.match(component, /\{view\.line\}/);
    // 尺寸输入只有「是否有同步词 + 贴边方向」，换句不重设窗口。
    assert.match(component, /islandCapsuleLineLimit\(vertical, view\.hasLine\)/);
    assert.match(component, /islandScrollPlan\(\{/);
    assert.match(component, /islandScrollKey\(\{/);
    assert.match(component, /islandScrollAnimationName\(scroll\.kind, vertical\)/);
    assert.match(component, /islandScrollPlayState\(scroll\.paused\)/);
    assert.match(component, /--island-capsule-line-max/);
    assert.match(component, /data-motion=\{motion \? "on" : "off"\}/);
    assert.doesNotMatch(component, /island-music__reserve/);
    assert.doesNotMatch(component, /island-lyric-scroll-[xy]"/); // 轴向只由 islandScrollAnimationName 决定。
    assert.doesNotMatch(component, /animationPlayState: scroll\.paused/); // 暂停映射只由 islandScrollPlayState 决定。
    // 展开进度条的量程、夹紧与占位必须走 musicProgressRange；回退到 durationMs||1 会红。
    assert.match(component, /musicProgressRange\(music\.timeline, music\.lyrics, position\)/);
    assert.match(component, /max=\{progress\.max\}/);
    assert.match(component, /value=\{progress\.value\}/);
    assert.match(component, /\{progress\.totalMs > 0 \? formatTime\(progress\.totalMs\) : "--:--"\}/);
    assert.doesNotMatch(component, /durationMs \|\| 1/);
    assert.doesNotMatch(component, /formatTime\(music\.timeline\?\.durationMs \?\? 0\)/);
});

test("常态胶囊按贴边方向与是否有同步词分档，内容盒扣除固定件", () => {
    assert.deepEqual(ISLAND_CAPSULE_LIMITS, { horizontal: { lyric: 240, fallback: 168 }, vertical: { lyric: 176, fallback: 96 } });
    assert.equal(ISLAND_CAPSULE_CHROME, 52);
    assert.equal(islandCapsuleLimit(false, true), 240); // 贴上下、有同步词。
    assert.equal(islandCapsuleLimit(false, false), 168); // 贴上下、无同步词或首句前。
    assert.equal(islandCapsuleLimit(true, true), 176); // 贴左右、有同步词。
    assert.equal(islandCapsuleLimit(true, false), 96); // 贴左右、无同步词。
    // 内容盒 = 档位 − 固定件（内边距 8×2＋媒体图 28＋间距 8）；固定档位，换句不变。
    assert.equal(islandCapsuleLineLimit(false, true), 188);
    assert.equal(islandCapsuleLineLimit(false, false), 116);
    assert.equal(islandCapsuleLineLimit(true, true), 124);
    assert.equal(islandCapsuleLineLimit(true, false), 44);
    // 竖向最小胶囊 52 由卡片 min-block-size 兜底，扣完固定件的无词内容盒仍为正。
    assert.ok(islandCapsuleLineLimit(true, false) > 0);
});

test("超长判定取整且非有限尺寸不滚动", () => {
    assert.equal(islandScrollDistance(200, 188), 12);
    assert.equal(islandScrollDistance(200.6, 188), 13);
    assert.equal(islandScrollDistance(188, 188), 0);
    assert.equal(islandScrollDistance(120, 188), 0);
    for (const [content, box] of [
        [Number.NaN, 188],
        [200, Number.NaN],
        [Number.POSITIVE_INFINITY, 188],
        [200, Number.NEGATIVE_INFINITY],
    ])
        assert.equal(islandScrollDistance(content, box), 0);
});

test("进度联动滚动：句内映射、暂停冻结、跳转重置、首句前与末句后", () => {
    const base = { distance: 120, hasLine: true, lineStartMs: 3000, lineEndMs: 8000, positionMs: 3000, advancing: true, animationsEnabled: true };
    assert.deepEqual(islandScrollPlan(base), { kind: "progress", distance: 120, durationMs: 5000, delayMs: 0, paused: false });
    assert.deepEqual(islandScrollPlan({ ...base, positionMs: 5500 }), { kind: "progress", distance: 120, durationMs: 5000, delayMs: 2500, paused: false });
    // 暂停冻结在当前偏移：位置不变、动画暂停，恢复时从同一偏移继续。
    const paused = islandScrollPlan({ ...base, positionMs: 5500, advancing: false });
    assert.deepEqual(paused, { kind: "progress", distance: 120, durationMs: 5000, delayMs: 2500, paused: true });
    assert.equal(islandScrollPlan({ ...base, positionMs: 5500 }).delayMs, paused.delayMs);
    // 跳转按新位置重新映射，不沿用旧句累计。
    assert.equal(islandScrollPlan({ ...base, positionMs: 7000 }).delayMs, 4000);
    assert.equal(islandScrollPlan({ ...base, positionMs: 3500 }).delayMs, 500);
    // 末句后夹紧到句尾并停在末位（时间轴停止推进时冻结）。
    assert.deepEqual(islandScrollPlan({ ...base, positionMs: 90000, advancing: false }), {
        kind: "progress",
        distance: 120,
        durationMs: 5000,
        delayMs: 5000,
        paused: true,
    });
    // 首句前没有当前句：回落文本慢速循环；循环周期随距离增长并有下限。
    assert.deepEqual(islandScrollPlan({ ...base, hasLine: false, positionMs: 0 }), {
        kind: "loop",
        distance: 120,
        durationMs: 12000,
        delayMs: 0,
        paused: false,
    });
    assert.equal(islandScrollPlan({ ...base, hasLine: false, positionMs: 0, advancing: false }).paused, true);
    assert.equal(islandScrollPlan({ ...base, hasLine: false, distance: 10 }).durationMs, ISLAND_LOOP_MIN_MS);
    // 减少动效、无溢出、无当前位置：不滚动，交给 CSS 单行截断。
    assert.equal(islandScrollPlan({ ...base, animationsEnabled: false, positionMs: 4000 }).kind, "none");
    assert.equal(islandScrollPlan({ ...base, distance: 0, positionMs: 4000 }).kind, "none");
    assert.equal(islandScrollPlan({ ...base, positionMs: Number.NaN }).kind, "none");
});

test("零时间轴末句没有结束时间时回退慢速循环，不静默裁切", () => {
    const base = { distance: 120, hasLine: true, lineStartMs: 5000, lineEndMs: 0, positionMs: 6000, advancing: true, animationsEnabled: true };
    // 零时间轴末句：end=0（或等于起句时间）不可审计 → loop，动效开启时往返显示全文。
    assert.deepEqual(islandScrollPlan(base), { kind: "loop", distance: 120, durationMs: 12000, delayMs: 0, paused: false });
    assert.deepEqual(islandScrollPlan({ ...base, lineEndMs: 5000 }), { kind: "loop", distance: 120, durationMs: 12000, delayMs: 0, paused: false });
    assert.deepEqual(islandScrollPlan({ ...base, lineEndMs: undefined }), { kind: "loop", distance: 120, durationMs: 12000, delayMs: 0, paused: false });
    // 暂停冻结循环相位；减少动效仍由 CSS 单行截断（none）。
    assert.equal(islandScrollPlan({ ...base, advancing: false }).paused, true);
    assert.equal(islandScrollPlan({ ...base, animationsEnabled: false }).kind, "none");
    // 有下一句或真实时长时仍按进度推进。
    assert.equal(islandScrollPlan({ ...base, lineEndMs: 8000 }).kind, "progress");
});

test("整首尺寸不随句子变化：同一贴边方向下只由有无同步词与换边决定", () => {
    // 逐点扫描整首歌：尺寸输入只有 hasLine 与贴边方向，换句（index 变化）不改变它，因此不会逐句重设窗口。
    for (let position = 0; position <= 10000; position += 250) {
        const current = view(position);
        assert.equal(islandCapsuleLimit(false, current.hasLine), current.hasLine ? 240 : 168);
        assert.equal(islandCapsuleLimit(true, current.hasLine), current.hasLine ? 176 : 96);
    }
    const first = view(1000);
    const second = view(3000);
    assert.notEqual(first.index, second.index); // 确实是不同句。
    assert.equal(islandCapsuleLineLimit(false, first.hasLine), islandCapsuleLineLimit(false, second.hasLine));
    assert.equal(islandCapsuleLineLimit(true, first.hasLine), islandCapsuleLineLimit(true, second.hasLine));
    // 唯一会改变尺寸的状态变化：换边（false↔true）与有词/无词（hasLine）。
    assert.notEqual(islandCapsuleLineLimit(false, true), islandCapsuleLineLimit(true, true));
    assert.notEqual(islandCapsuleLineLimit(false, false), islandCapsuleLineLimit(false, true));
});

test("滚动方向与暂停映射：竖向 *-y、横向 *-x，暂停冻结", () => {
    assert.equal(islandScrollAnimationName("progress", false), "island-lyric-scroll-x");
    assert.equal(islandScrollAnimationName("progress", true), "island-lyric-scroll-y");
    assert.equal(islandScrollAnimationName("loop", false), "island-lyric-loop-x");
    assert.equal(islandScrollAnimationName("loop", true), "island-lyric-loop-y");
    assert.equal(islandScrollAnimationName("none", true), undefined);
    assert.equal(islandScrollAnimationName("none", false), undefined);
    assert.equal(islandScrollPlayState(true), "paused");
    assert.equal(islandScrollPlayState(false), "running");
});

test("无词循环暂停/恢复不重建 span，进度滚动按冻结位置重建", () => {
    const loop = islandScrollPlan({ distance: 88, hasLine: false, positionMs: 0, advancing: true, animationsEnabled: true });
    assert.equal(loop.kind, "loop");
    assert.equal(loop.delayMs, 0); // 循环相位由 CSS 动画保持，不在 plan 里重复表达。
    const loopKey = (paused: boolean, patch: { sessionId?: string; vertical?: boolean; distance?: number } = {}) =>
        islandScrollKey({
            sessionId: patch.sessionId ?? "1",
            index: -1,
            vertical: patch.vertical ?? false,
            plan: { ...loop, paused, distance: patch.distance ?? loop.distance },
        });
    // 暂停与恢复用同一个键：span 不重建，animation-play-state 冻结/恢复现有循环相位。
    assert.equal(loopKey(true), loopKey(false));
    // 换边/换歌/回落文本长度变化仍重建，按新档位重新开始。
    assert.notEqual(loopKey(true, { vertical: true }), loopKey(true));
    assert.notEqual(loopKey(true, { sessionId: "2" }), loopKey(true));
    assert.notEqual(loopKey(true, { distance: 120 }), loopKey(true, { distance: 88 }));
    const progress = islandScrollPlan({
        distance: 120,
        hasLine: true,
        lineStartMs: 3000,
        lineEndMs: 8000,
        positionMs: 5500,
        advancing: true,
        animationsEnabled: true,
    });
    const progressKey = (plan: typeof progress) => islandScrollKey({ sessionId: "1", index: 1, vertical: false, plan });
    // 进度档暂停时按冻结位置重建，偏移精确停留在暂停处（不依赖 CSS 自身时间）。
    assert.notEqual(progressKey({ ...progress, paused: true }), progressKey(progress));
    // 跳转（延迟变化 ≥100ms）重建并按新位置对齐。
    const seek = islandScrollPlan({
        distance: 120,
        hasLine: true,
        lineStartMs: 3000,
        lineEndMs: 8000,
        positionMs: 7000,
        advancing: true,
        animationsEnabled: true,
    });
    assert.notEqual(progressKey(seek), progressKey(progress));
});
