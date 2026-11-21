import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { animateIslandFold, beginIslandFold, beginIslandFoldSave, endIslandFoldSave, islandCardDropTarget, islandFoldTargetHeight, isIslandFoldSaveCurrent, settleIslandFold } from "./islandCardFold.ts";

test("连续点击只替换最新目标，旧动画与旧回执不能结算", () => {
    const idle = { phase: "idle" as const, target: false, seq: 0 };
    const first = beginIslandFold(idle, false);
    assert.deepEqual(first, { phase: "pending", target: true, seq: 1 });
    const second = beginIslandFold(first, true); // 动画途中再点回展开。
    assert.deepEqual(second, { phase: "pending", target: false, seq: 2 });
    assert.equal(animateIslandFold(second, 1).phase, "pending"); // 旧动画不能抢占新事务。
    assert.equal(animateIslandFold(second, 2).phase, "animating");
    assert.equal(settleIslandFold(second, 1).phase, "pending"); // 旧 finished 不清新 pending。
    assert.equal(settleIslandFold(second, 2).phase, "idle");
});

test("目标高度优先自然高，测量失败回落记忆高", () => {
    assert.equal(islandFoldTargetHeight(120), 120);
    assert.equal(islandFoldTargetHeight(Number.NaN, 88), 88);
    assert.equal(islandFoldTargetHeight(0, 88), 88);
    assert.equal(islandFoldTargetHeight(Number.POSITIVE_INFINITY, Number.NaN), undefined); // 都测不到交给自然流。
});

test("拖卡落点取指针覆盖卡，否则取纵向最近卡，排除自身", () => {
    const cards = [
        { id: "a", top: 0, height: 100 },
        { id: "b", top: 108, height: 100 },
        { id: "c", top: 216, height: 100 },
    ];
    assert.equal(islandCardDropTarget(cards, 50, "a"), "b"); // 自身被排除，落到下一张。
    assert.equal(islandCardDropTarget(cards, 150, "a"), "b");
    assert.equal(islandCardDropTarget(cards, 260, "a"), "c");
    assert.equal(islandCardDropTarget(cards, 260, "c"), "b");
    const bounds = { left: 0, top: 0, width: 200, height: 330 };
    assert.equal(islandCardDropTarget(cards, { x: 100, y: 150 }, "a", bounds), "b"); // 列表内正常落点。
    assert.equal(islandCardDropTarget(cards, { x: 100, y: -20 }, "a", bounds), undefined); // 列表上方越界。
    assert.equal(islandCardDropTarget(cards, { x: 260, y: 150 }, "a", bounds), undefined); // 窗口右侧越界。
    assert.equal(islandCardDropTarget(cards, { x: 100, y: 360 }, "a", bounds), undefined); // 列表下方越界。
});

test("拖动排序复用完整 islandCardOrder，松手只提交一次；拖动期间不回退", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const order = preferences\?\.islandCardOrder \?\? instances\.map/); // 同源完整顺序。
    assert.match(source, /const next = moveIslandVisibleCard\(order, from, to\);/);
    assert.match(source, /onPreferences\(await saveIslandCardOrder\(next\)\)/); // 保存一次完整顺序。
    assert.match(source, /const target = active \? cardDropAt\(id, event\.clientX, event\.clientY\) : undefined;/);
    assert.match(source, /if \(active && target && target !== id\) void handleReorder\(id, target\);/);
    assert.match(source, /if \(cardSorting\.current\) return; \/\/ 拖动排序期间保持当前态/); // 不回退胶囊。
    assert.match(source, /if \(cardSorting\.current\) return; \/\/ 拖卡期间不评估透明边距回退。/);
});

test("统一卡头：消息、官方与插件卡共用句柄、动作位和收起按钮", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const IslandCardToolbar = \(/);
    assert.match(source, /className="island-card__handle"/);
    assert.match(source, /className="island-card__actions"/);
    assert.match(source, /aria-label=\{`调整\$\{name\}卡顺序`\}/);
    assert.match(source, /onKeyDown=\{onHandleKeyDown\}/);
    assert.match(source, /label: "打开 DSH"/); // DSH 首个安全动作。
    assert.match(source, /disabled: islandPaused \|\| cardStatus === "failed" \|\| !islandNavigationUri\(actionUri\)/);
    assert.match(source, /data-island-card-id=\{ISLAND_MESSAGE_CARD_ID\}/);
    assert.match(source, /data-island-card-id=\{instance\.componentId\}/);
    assert.match(source, /<IslandCardToolbar[\s\S]*ISLAND_MESSAGE_CARD_ID[\s\S]*?\/>/); // 消息卡也走卡头。
    assert.doesNotMatch(source, /island-message-panel__toggle/); // 不保留消息卡内部旧收起分支。
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    assert.match(css, /\.island-card__toolbar \{[\s\S]*?min-block-size: 32px;/);
    assert.match(css, /\.island-card__handle \{[\s\S]*?inline-size: 56px;[\s\S]*?block-size: 28px;/);
    assert.match(css, /\.island-card__handle span \{[\s\S]*?inline-size: 36px;[\s\S]*?block-size: 4px;/);
    assert.match(css, /\.island-card__action,[\s\S]*?block-size: 28px;/);
    assert.match(css, /\.island-card__toolbar \{[\s\S]*?position: relative;/);
    assert.match(css, /\.island-card__handle \{[\s\S]*?position: absolute;[\s\S]*?inset-inline-start: 50%;[\s\S]*?transform: translate\(-50%, -50%\);/); // 句柄水平居中，不挤压右侧动作区。
});

test("内容容器只按宿主可用区域布局，不自行改窗口尺寸", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /maxBlockSize: work\?\.height/); // 高度上限由宿主工作区给出。
    assert.doesNotMatch(source, /set_island_state\([^)]*expanded[^)]*height/i); // 不在卡内重设窗口高度。
});

test("展开插件用真实收起值，关动效即 settle，折叠在途禁用句柄与动作", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /<PluginSlot[\s\S]*?collapsed=\{cardCollapsed\(instance\.componentId\)\}[\s\S]*?capabilities=/); // 展开插件不再硬编码 false。
    assert.match(source, /if \(!animationsEnabled \|\| target === undefined \|\| Math\.abs\(target - origin\.height\) < 0\.5\) \{\s*settle\(\)/); // 关动效直接落自然高。
    assert.doesNotMatch(source, /if \(mode !== "expanded" \|\| !animationsEnabled\) return;/);
    assert.match(source, /disabled=\{pending\} \/\/ 展开\/收缩动画在途时禁用拖动/);
    assert.match(source, /disabled=\{action\.disabled \|\| pending\}/);
    assert.match(source, /if \(cardSorting\.current\) \{ endCardSort\(\); return; \} \/\/ Esc 先取消排序并回滚/);
});

test("快速连点：旧 revision 早退也成对减少 inflight，最终归零", () => {
    const tracker = { revision: 0, inflight: 0 };
    const first = beginIslandFoldSave(tracker);
    const second = beginIslandFoldSave(tracker);
    assert.deepEqual(tracker, { revision: 2, inflight: 2 });
    assert.equal(isIslandFoldSaveCurrent(tracker, first), false);
    endIslandFoldSave(tracker); // 旧 revision 早退仍然结算。
    assert.deepEqual(tracker, { revision: 2, inflight: 1 });
    assert.equal(isIslandFoldSaveCurrent(tracker, second), true);
    endIslandFoldSave(tracker);
    assert.deepEqual(tracker, { revision: 2, inflight: 0 });
    endIslandFoldSave(tracker);
    assert.equal(tracker.inflight, 0); // 不会减到负数。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /finally \{\s*endIslandFoldSave\(save\);\s*\}/); // 早退与异常都走同一结算。
});

test("DSH hover 进度条用正文色填充并留出文本/动作间距", () => {
    const css = readFileSync(new URL("../../../plugin/DSH 接入/style.css", import.meta.url), "utf8");
    const trackStart = css.indexOf(".plugin-dsh-status-hover__track {");
    const track = css.slice(trackStart, css.indexOf("}", trackStart));
    assert.match(track, /background: transparent;/); // 透明轨由描边定义范围。
    assert.match(track, /border: 1px solid color-mix\(in srgb, var\(--fence-text\) 45%, transparent\);/);
    assert.match(track, /height: 4px;/);
    assert.match(track, /margin-block: 2px;/); // 4px 轨在 16px 行内垂直居中。
    assert.match(track, /align-self: center;/);
    const fillStart = css.indexOf(".plugin-dsh-status-hover__fill {");
    const fill = css.slice(fillStart, css.indexOf("}", fillStart));
    assert.match(fill, /background: var\(--fence-text\);/); // 浅/深主题正文色对比 4.54/5.79:1。
    assert.doesNotMatch(fill, /--fence-active/);
    assert.match(css, /\.plugin-dsh-status-hover__progress \{[\s\S]*?gap: 8px;/); // 与右侧百分比留 8 DIP。
});
