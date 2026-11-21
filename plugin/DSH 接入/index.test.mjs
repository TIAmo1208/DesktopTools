import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
    applyBridgeIdentityChanged,
    applySnapshotFacts,
    balanceFailureText,
    balanceText,
    beginMount,
    boundBridgeIdFor,
    collapsedSummary,
    diffPending,
    fetchBridgeSnapshot,
    formatAmount,
    formatAmountFixed,
    ISLAND_OPEN_URI,
    islandAnchorView,
    islandHoverView,
    islandSummaryArguments,
    isSnapshotFresh,
    isSnapshotTimedFresh,
    mountNeedsBaseline,
    nextPollDelay,
    normalizeBaseUrl,
    normalizePairingCode,
    notifyState,
    openPendingRow,
    pairWithBridge,
    parseSnapshotPayload,
    pendingCountText,
    pendingIdentity,
    pendingMessageKey,
    reducedMotionPreferred,
    reconcilePending,
    shouldPoll,
    shortText,
    statusLabel,
    subscribeState,
    textScrollPlan,
    updateTimeText,
    walletLines,
    walletText,
} from "./index.js";

/** 一份最小合法快照。 */
const snapshot = () => ({
    protocolVersion: 1,
    bridgeId: "bridge-stable",
    bridgeInstanceId: "bridge-instance",
    revision: 7,
    fetchedAt: new Date().toISOString(),
    stale: false,
    balance: { status: "ready", wallets: [{ currency: "CNY", amount: "12.34" }] },
    sessions: [{ id: "s1", title: "重构登录", status: "waiting", pending: "approval" }],
    jobs: [{ id: "j1", label: "构建", status: "running" }],
    pending: [{ id: "p1", kind: "approval", sessionId: "s1", since: "2026-11-20T00:59:00.000Z" }],
});

/** 造一条待决。 */
const pending = id => ({ id, kind: "approval", sessionId: "s1", since: `2026-11-20T00:0${id}:00.000Z` });

test("桥地址只接受本机 http 和显式端口", () => {
    assert.equal(normalizeBaseUrl("http://127.0.0.1:47832"), "http://127.0.0.1:47832");
    assert.equal(normalizeBaseUrl("http://localhost:47832/"), "http://127.0.0.1:47832");
    for (const invalid of ["", "https://127.0.0.1:47832", "http://example.com:47832", "http://127.0.0.1", "http://127.0.0.1:47832/path"]) {
        assert.throws(() => normalizeBaseUrl(invalid), /DSH 桥|端口|回环|路径/);
    }
});

test("配对码归一为五五五五分组", () => {
    assert.equal(normalizePairingCode("abcde fghij klmno pqrst"), "ABCDE-FGHIJ-KLMNO-PQRST");
    assert.throws(() => normalizePairingCode("短码"), /20 位/);
});

test("快照解析保留余额、会话、job 与待决", () => {
    const parsed = parseSnapshotPayload(JSON.stringify(snapshot()));
    assert.equal(parsed.bridgeId, "bridge-stable");
    assert.equal(parsed.balance.status, "ready");
    assert.equal(parsed.balance.wallets[0].amount, "12.34");
    assert.equal(parsed.sessions[0].status, "waiting");
    assert.equal(parsed.jobs[0].status, "running");
    assert.equal(parsed.pending[0].id, "p1");
    const withStats = parseSnapshotPayload({
        ...snapshot(),
        taskStats: { runCount: 2, unreadCount: 3, completedUnreadCount: 1, unionCount: 4 },
    });
    assert.deepEqual(withStats.taskStats, {
        runCount: 2,
        unreadCount: 3,
        completedUnreadCount: 1,
        unionCount: 4,
        completeness: "",
        sampledAt: "",
    });
    assert.throws(() => parseSnapshotPayload({ ...snapshot(), protocolVersion: 2 }), /协议版本/);
    assert.throws(() => parseSnapshotPayload({ ...snapshot(), bridgeId: "" }), /bridgeId/);
});

test("待决身份由 bridgeId、pendingId、since 组成，消息 key 稳定且不超长", () => {
    const first = pendingIdentity("bridge-a", { id: "p1", since: "2026-11-20T00:59:00.000Z" });
    const second = pendingIdentity("bridge-b", { id: "p1", since: "2026-11-20T00:59:00.000Z" });
    const third = pendingIdentity("bridge-a", { id: "p1", since: "2026-11-20T01:00:00.000Z" });
    assert.notEqual(first, second);
    assert.notEqual(first, third);
    assert.equal(pendingMessageKey(first), pendingMessageKey(first));
    assert.ok(pendingMessageKey(first).length <= 128);
    assert.match(pendingMessageKey(first), /^dsh-pending:/);
});

test("新增待决只挑未见过的身份", () => {
    const seen = new Set([pendingIdentity("bridge-a", { id: "p1", since: "s1" })]);
    const fresh = diffPending(seen, "bridge-a", [
        { id: "p1", since: "s1" },
        { id: "p2", since: "s2" },
    ]);
    assert.deepEqual(fresh, [{ id: "p2", since: "s2" }]);
    assert.equal(seen.size, 2);
});

test("轮询成功 15 秒、失败退避 60 秒", () => {
    assert.equal(nextPollDelay(true), 15000);
    assert.equal(nextPollDelay(false), 60000);
});

test("余额状态不把未知显示成 0", () => {
    assert.equal(balanceText({ status: "ready", wallets: [{ currency: "CNY", amount: "12.34" }] }), "CNY 12.34");
    assert.equal(balanceText({ status: "logged-out", wallets: [] }), "账号未登录");
    assert.equal(balanceText({ status: "unauthorized", wallets: [] }), "余额未授权");
    assert.equal(balanceText({ status: "error", wallets: [] }), "余额读取失败");
    assert.doesNotMatch(balanceText({ status: "error", wallets: [] }), /0/);
});

test("状态文案覆盖未配对、身份变化、401、不可达", () => {
    assert.equal(statusLabel("unpaired"), "未配对");
    assert.equal(statusLabel("identity-changed"), "未授权");
    assert.equal(statusLabel("unauthorized"), "未授权");
    assert.equal(statusLabel("unreachable"), "未连接");
    assert.equal(statusLabel("error"), "读取失败");
});

test("配对走宿主兑换并确认 bridgeId，不在插件里读取 token", async () => {
    const calls = [];
    let setCredentialCalled = false;
    const context = {
        exchangeCredential: async (slot, options) => {
            calls.push({ kind: "exchange", slot, options });
            return { stored: true };
        },
        registerLoopbackChannel: async registration => calls.push({ kind: "register", registration }),
        requestLoopback: async () => ({
            status: 200,
            body: JSON.stringify({ ok: true, bridgeId: "bridge-stable", bridgeInstanceId: "bridge-instance" }),
        }),
        setCredential: () => {
            setCredentialCalled = true;
        },
    };
    const result = await pairWithBridge(context, "http://127.0.0.1:47832", "abcde fghij klmno pqrst");
    const exchange = calls.find(call => call.kind === "exchange");
    assert.deepEqual(exchange, {
        kind: "exchange",
        slot: "token",
        options: {
            url: "http://127.0.0.1:47832/pair",
            body: { code: "ABCDE-FGHIJ-KLMNO-PQRST" },
            extract: "token",
        },
    });
    assert.deepEqual(result, { stored: true, bridgeId: "bridge-stable" });
    assert.equal("token" in result, false);
    assert.equal(setCredentialCalled, false);
    await assert.rejects(() => pairWithBridge({}, "http://127.0.0.1:47832", "abcde fghij klmno pqrst"), /plugin-credential-exchange/);
});

test("先无凭据读 health，再用带凭据通道读 snapshot 并核对两个身份", async () => {
    const calls = [];
    const health = { ok: true, protocolVersion: 1, bridgeId: "bridge-stable", bridgeInstanceId: "bridge-instance" };
    const context = {
        registerLoopbackChannel: async registration => calls.push({ kind: "register", registration }),
        requestLoopback: async (channelId, method, path) => {
            calls.push({ kind: "request", channelId, method, path });
            return channelId === "dsh-health"
                ? { status: 200, body: JSON.stringify(health) }
                : { status: 200, body: JSON.stringify(snapshot()) };
        },
    };
    const state = { healthChannelReady: false, bridgeChannelReady: false, boundBridgeId: "bridge-stable" };
    const parsed = await fetchBridgeSnapshot(context, "http://127.0.0.1:47832", state);
    assert.equal(parsed.bridgeId, "bridge-stable");
    assert.deepEqual(calls.map(call => call.kind), ["register", "request", "register", "request"]);
    assert.equal(calls[0].registration.channelId, "dsh-health");
    assert.equal("credentialRef" in calls[0].registration, false);
    assert.equal(calls[1].channelId, "dsh-health");
    assert.equal(calls[2].registration.channelId, "dsh-bridge");
    assert.equal(calls[2].registration.credentialRef, "DesktopTools/plugin/plugin.dsh-status/token");
    assert.equal(calls[2].registration.headerName, "Authorization");
    assert.equal(calls[2].registration.credentialPrefix, "Bearer ");
    assert.equal(calls[3].channelId, "dsh-bridge");
});

test("快照与 health 的 bridgeInstanceId 不一致时拒绝本轮", async () => {
    const health = { ok: true, bridgeId: "bridge-stable", bridgeInstanceId: "instance-a" };
    const context = {
        registerLoopbackChannel: async () => undefined,
        requestLoopback: async channelId => ({
            status: 200,
            body: JSON.stringify(channelId === "dsh-health" ? health : { ...snapshot(), bridgeInstanceId: "instance-b" }),
        }),
    };
    await assert.rejects(
        () => fetchBridgeSnapshot(context, "http://127.0.0.1:47832", {
            healthChannelReady: false,
            bridgeChannelReady: false,
            boundBridgeId: "bridge-stable",
        }),
        error => error.code === "bridge-identity-mismatch",
    );
});

test("持久绑定只对写入时的地址有效，改地址不迁移旧绑定", () => {
    const binding = { bridgeId: "bridge-old", bindingBaseUrl: "http://127.0.0.1:47832" };
    assert.equal(boundBridgeIdFor(binding, "http://127.0.0.1:47832"), "bridge-old");
    assert.equal(boundBridgeIdFor(binding, "http://127.0.0.1:47833"), "");
    assert.equal(boundBridgeIdFor({ bridgeId: "bridge-old" }, "http://127.0.0.1:47832"), "");
});

test("同端口换桥：health 的 bridgeId 变化时不发带凭据请求，重启后仍拒绝", async () => {
    const calls = [];
    const health = { ok: true, bridgeId: "bridge-new", bridgeInstanceId: "bridge-instance" };
    const context = {
        registerLoopbackChannel: async registration => calls.push({ kind: "register", registration }),
        requestLoopback: async (channelId, method, path) => {
            calls.push({ kind: "request", channelId, path });
            return { status: 200, body: JSON.stringify(health) };
        },
    };
    for (let restart = 0; restart < 2; restart += 1) {
        await assert.rejects(
            () => fetchBridgeSnapshot(context, "http://127.0.0.1:47832", {
                healthChannelReady: false,
                bridgeChannelReady: false,
                boundBridgeId: "bridge-old",
            }),
            error => error.code === "bridge-identity-changed" && /重新配对/.test(error.message),
        );
    }
    assert.equal(calls.some(call => call.kind === "register" && call.registration.channelId === "dsh-bridge"), false);
    assert.equal(calls.some(call => call.kind === "request" && call.channelId === "dsh-bridge"), false);
});

test("重新配对写入新 bridgeId 后携密读取恢复", async () => {
    const context = {
        exchangeCredential: async () => ({ stored: true }),
        registerLoopbackChannel: async () => undefined,
        requestLoopback: async channelId => ({
            status: 200,
            body: JSON.stringify(channelId === "dsh-health"
                ? { ok: true, bridgeId: "bridge-new", bridgeInstanceId: "bridge-instance" }
                : { ...snapshot(), bridgeId: "bridge-new" }),
        }),
    };
    const paired = await pairWithBridge(context, "http://127.0.0.1:47832", "abcde fghij klmno pqrst");
    assert.equal(paired.bridgeId, "bridge-new");
    const parsed = await fetchBridgeSnapshot(context, "http://127.0.0.1:47832", {
        healthChannelReady: false,
        bridgeChannelReady: false,
        boundBridgeId: paired.bridgeId,
    });
    assert.equal(parsed.bridgeId, "bridge-new");
});

test("旧代次在 await 返回后丢弃，不再取快照", async () => {
    let checks = 0;
    const guard = () => {
        checks += 1;
        return checks <= 2;
    };
    const calls = [];
    const health = { ok: true, bridgeId: "bridge-stable", bridgeInstanceId: "bridge-instance" };
    const context = {
        registerLoopbackChannel: async () => undefined,
        requestLoopback: async (channelId, method, path) => {
            calls.push(`${channelId}:${path}`);
            return { status: 200, body: JSON.stringify(health) };
        },
    };
    await assert.rejects(
        () => fetchBridgeSnapshot(context, "http://127.0.0.1:47832", {
            healthChannelReady: false,
            bridgeChannelReady: false,
            boundBridgeId: "bridge-stable",
        }, guard),
        error => error.code === "stale-result",
    );
    assert.deepEqual(calls, ["dsh-health:/health"]);
});

test("隐藏时不轮询；恢复先建基线，不补播隐藏期间的待决", () => {
    assert.equal(shouldPoll({ visibilityState: "hidden" }), false);
    assert.equal(shouldPoll({ visibilityState: "visible" }), true);
    const published = [];
    const context = { publishIslandMessage: async message => published.push(message) };
    const state = { baselineDone: false, needsBaseline: true, seen: new Set() };
    const base = snapshot();
    assert.deepEqual(reconcilePending(context, state, base), []);
    assert.equal(published.length, 0);
    state.needsBaseline = true;
    reconcilePending(context, state, { ...base, pending: [...base.pending, pending("2")] });
    assert.equal(published.length, 0);
    state.needsBaseline = false;
    const fresh = reconcilePending(context, state, { ...base, pending: [...base.pending, pending("2"), pending("3")] });
    assert.equal(fresh.length, 1);
    assert.equal(published.length, 1);
    assert.equal(published[0].kind, "waiting");
    assert.equal(published[0].uri, ISLAND_OPEN_URI);
});

test("有 3 条待决 → 卸载 → 重挂：首份只建基线，第 4 条才投递", () => {
    const published = [];
    const context = { publishIslandMessage: async message => published.push(message) };
    const state = { baselineDone: false, needsBaseline: false, seen: new Set() };
    const three = { ...snapshot(), pending: [pending("1"), pending("2"), pending("3")] };
    beginMount(state);
    assert.deepEqual(reconcilePending(context, state, three), []);
    assert.equal(published.length, 0);
    beginMount(state); // 模拟卸载后同实例重挂
    assert.deepEqual(reconcilePending(context, state, three), []);
    assert.equal(published.length, 0);
    const four = { ...three, pending: [...three.pending, pending("4")] };
    const fresh = reconcilePending(context, state, four);
    assert.deepEqual(fresh.map(item => item.id), ["4"]);
    assert.equal(published.length, 1);
    assert.equal(published[0].text, "DSH 有新的待决：重构登录");
    assert.equal(published[0].uri, ISLAND_OPEN_URI);
});

test("锚点六档优先级：错误、陈旧、待决、余额、余额不可用、隐藏", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const view = state => islandAnchorView({ ...state, snapshot: state.snapshot ?? base }, at);

    assert.equal(view({ status: "unauthorized" }).text, "未授权");
    assert.equal(view({ status: "identity-changed" }).text, "未授权");
    assert.equal(view({ status: "unreachable" }).text, "未连接");
    assert.equal(view({ status: "error" }).text, "读取失败");
    assert.equal(view({ status: "unauthorized" }).status, "unauthorized");
    assert.equal(view({ status: "unreachable" }).status, "disconnected");
    assert.equal(view({ status: "error" }).status, "failed");

    assert.equal(view({ status: "ready", snapshot: { ...base, fetchedAt: "2026-11-20T00:00:00.000Z" } }).text, "数据陈旧");
    assert.equal(view({ status: "ready", snapshot: { ...base, fetchedAt: "" } }).text, "数据陈旧");

    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [pending("1"), pending("2"), pending("3")] } }).text, "3 项待决");
    assert.equal(islandAnchorView({ status: "ready", snapshot: { ...base, pending: Array.from({ length: 100 }, (_, index) => pending(String(index % 10))) } }, at).text, "99+ 项待决");
    assert.equal(pendingCountText(100), "99+");
    assert.equal(pendingCountText(99), "99");

    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [] } }).text, "CNY 12.34");
    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [], balance: { status: "ready", wallets: [{ currency: "CNY", amount: "1" }, { currency: "USD", amount: "2" }] } } }).text, "余额可读");

    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [], balance: { status: "logged-out", wallets: [] } } }).text, "账号未登录");
    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [], balance: { status: "unauthorized", wallets: [] } } }).text, "余额未授权");
    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [], balance: { status: "error", wallets: [] } } }).text, "余额读取失败");
    assert.equal(view({ status: "ready", snapshot: { ...base, pending: [], balance: { status: "ready", wallets: [] } } }).text, "已连接");

    assert.equal(islandAnchorView({ status: "unpaired", snapshot: null }, at).visible, false);
    assert.equal(islandAnchorView({ status: "loading", snapshot: null }, at).visible, false);
    assert.equal(islandAnchorView({ status: "ready", snapshot: null }, at).visible, false);
    assert.equal(islandAnchorView({ status: "ready", snapshot: base }, at, { paused: true }).visible, false);
});

test("hover 只读视图：运行/未读/进度/余额与缺失降级", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const running = { ...base, sessions: [{ ...base.sessions[0], status: "running" }] };
    const taskStats = overrides => ({
        runCount: 1,
        unreadCount: 3,
        completedUnreadCount: 1,
        unionCount: 4,
        completeness: "complete",
        sampledAt: base.fetchedAt,
        ...overrides,
    });
    const noUnread = islandHoverView({ status: "ready", snapshot: running }, at);
    assert.equal(noUnread.main, "可见会话中运行 1 项");
    assert.equal(noUnread.sub, "未读状态不可用");
    assert.equal(noUnread.progress, undefined);
    assert.equal(noUnread.balance, "余额 CNY 12.34");
    assert.equal(islandHoverView({ status: "ready", snapshot: { ...running, pending: [] } }, at).balance, "余额 CNY 12.34");

    const full = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ runCount: 2 }) },
    }, at);
    assert.equal(full.main, "运行任务 2 项");
    assert.equal(full.sub, "未读 3 项 · 完成未读 1 项");
    assert.deepEqual(full.progress, {
        completed: 1,
        total: 4,
        percent: 25,
        label: "待关注任务中完成未读比例，1 项，共 4 项，25%",
    });

    const missingCompleted = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ completedUnreadCount: undefined }) },
    }, at);
    assert.equal(missingCompleted.progress, undefined);
    assert.equal(missingCompleted.sub, "统计不完整");
    const missingUnread = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ unreadCount: undefined, completedUnreadCount: 1 }) },
    }, at);
    assert.equal(missingUnread.progress, undefined);
    assert.equal(missingUnread.sub, "统计不完整");
    const noUnion = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ unionCount: undefined }) },
    }, at);
    assert.equal(noUnion.main, "运行任务 1 项");
    assert.equal(noUnion.sub, "统计不完整");
    assert.equal(noUnion.progress, undefined);

    const missingUnionWithBadCompleted = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ runCount: 0, unreadCount: 0, completedUnreadCount: 1, unionCount: undefined }) },
    }, at);
    assert.notEqual(missingUnionWithBadCompleted.main, "暂无待关注任务");
    assert.equal(missingUnionWithBadCompleted.sub, "统计不完整");
    assert.equal(missingUnionWithBadCompleted.progress, undefined);

    const missingRunCount = islandHoverView({
        status: "ready",
        snapshot: {
            ...running,
            sessions: [{ ...running.sessions[0], status: "running" }, { ...running.sessions[0], id: "s2", status: "running" }],
            taskStats: taskStats({ runCount: undefined, unreadCount: 1, completedUnreadCount: 1, unionCount: 2 }),
        },
    }, at);
    assert.equal(missingRunCount.main, "可见会话中运行 2 项");
    assert.equal(missingRunCount.sub, "统计不完整");
    assert.equal(missingRunCount.progress, undefined);

    const completedZero = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ runCount: 3, unreadCount: 2, completedUnreadCount: 0, unionCount: 5 }) },
    }, at);
    assert.equal(completedZero.main, "运行任务 3 项");
    assert.equal(completedZero.progress, undefined);

    const partial = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ completeness: "partial" }) },
    }, at);
    assert.equal(partial.main, "可见会话中运行 1 项");
    assert.equal(partial.sub, "未读状态不可用");
    assert.equal(partial.progress, undefined);

    const missingCompleteness = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ completeness: undefined }) },
    }, at);
    assert.equal(missingCompleteness.progress, undefined);
    const staleStats = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ sampledAt: new Date(Date.parse(base.fetchedAt) - 90000).toISOString() }) },
    }, at);
    assert.equal(staleStats.progress, undefined);
    const badCompleted = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ unreadCount: 1, completedUnreadCount: 2 }) },
    }, at);
    assert.equal(badCompleted.main, "运行任务 1 项");
    assert.equal(badCompleted.sub, "统计不完整");
    assert.equal(badCompleted.progress, undefined);
    const badUnion = islandHoverView({
        status: "ready",
        snapshot: { ...running, taskStats: taskStats({ runCount: 3, unionCount: 2 }) },
    }, at);
    assert.equal(badUnion.main, "运行任务 3 项");
    assert.equal(badUnion.sub, "统计不完整");
    assert.equal(badUnion.progress, undefined);

    const noTasks = islandHoverView({
        status: "ready",
        snapshot: {
            ...running,
            sessions: [],
            pending: [],
            taskStats: taskStats({ runCount: 0, unreadCount: 0, completedUnreadCount: 0, unionCount: 0 }),
        },
    }, at);
    assert.equal(noTasks.main, "暂无待关注任务");
    assert.equal(noTasks.sub, "未读 0 项 · 完成未读 0 项");
    assert.equal(noTasks.progress, undefined);
    assert.equal(islandHoverView({
        status: "ready",
        snapshot: { ...running, pending: [], balance: { status: "ready", wallets: [] } },
    }, at).balance, "无可用余额");
    assert.equal(islandHoverView({ status: "unpaired", snapshot: null }, at).main, "未配对");
    assert.equal(islandHoverView({ status: "unreachable", snapshot: null }, at).main, "未连接");
    assert.equal(islandHoverView({ status: "ready", snapshot: { ...running, sessions: undefined, hasSessions: false, pending: [] } }, at).main, "任务状态未知");
    assert.equal(islandHoverView({
        status: "ready",
        snapshot: { ...running, fetchedAt: "2026-11-20T00:00:00.000Z" },
    }, at).main, "数据陈旧");
    assert.equal(islandHoverView({
        status: "ready",
        snapshot: { ...running, pending: [], balance: { status: "ready", wallets: [{ currency: "CNY", amount: "12.3" }] } },
    }, at).balance, "余额 CNY 12.30");

    let notified = 0;
    const unsubscribe = subscribeState(running, () => { notified += 1; });
    notifyState(running);
    assert.equal(notified, 1);
    unsubscribe();
    notifyState(running);
    assert.equal(notified, 1);

    assert.equal(islandHoverView({
        status: "ready",
        snapshot: { ...running, pending: [], balance: { status: "ready", wallets: [{ currency: "CNY", amount: "12.3456" }, { currency: "USD", amount: "3.00" }] } },
    }, at).balance, "多币种余额（2）");
    assert.equal(islandHoverView({ status: "ready", snapshot: { ...base, stale: true } }, at).main, "数据陈旧");
    assert.equal(islandHoverView({ status: "unauthorized", snapshot: base }, at).main, "未授权");
    assert.equal(islandHoverView({ status: "ready", snapshot: { ...running, balance: { status: "ready", wallets: [] } } }, at).balance, "无可用余额");

    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    const hover = source.slice(source.indexOf("export function renderIslandHover"), source.indexOf("/**\n * 状态文案"));
    assert.doesNotMatch(hover, /setTimeout|setInterval|fetch\(|requestLoopback|registerLoopback/);
    assert.doesNotMatch(hover, /role="button"|tabIndex|<button/);
    assert.doesNotMatch(hover, /role="status"/);
    assert.doesNotMatch(hover, /\p{Extended_Pictographic}/u);
    assert.match(hover, /return subscribeState\(state, paintHover\)/);
    assert.match(hover, /aria-valuetext/);
    assert.match(source, /const collapsed = context\.collapsed === true;/);
    assert.doesNotMatch(source, /aria-busy/);
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const hoverCss = css.slice(css.indexOf(".plugin-dsh-status-hover"), css.indexOf(".plugin-dsh-status__collapsed-text"));
    assert.match(hoverCss, /text-overflow:\s*ellipsis/);
    assert.match(hoverCss, /max-width:\s*256px/);
    assert.match(hoverCss, /height:\s*4px/);
    assert.match(hoverCss, /color:\s*var\(--fence-text\)/);
    assert.doesNotMatch(hoverCss, /--fence-muted/);
});

test("同一端点+路径共享在途请求，挂载切换不重复发", async () => {
    const calls = [];
    let resolveHealth;
    let resolveSnapshot;
    const healthPromise = new Promise(resolve => { resolveHealth = resolve; });
    const snapshotPromise = new Promise(resolve => { resolveSnapshot = resolve; });
    const health = { ok: true, bridgeId: "bridge-stable", bridgeInstanceId: "bridge-instance" };
    const context = {
        instanceId: "island.plugin.dsh-status",
        registerLoopbackChannel: async () => undefined,
        requestLoopback: (channelId, method, path) => {
            calls.push(path);
            return path === "/health" ? healthPromise : snapshotPromise;
        },
    };
    const stateA = { healthChannelReady: false, bridgeChannelReady: false, boundBridgeId: "bridge-stable" };
    const stateB = { healthChannelReady: false, bridgeChannelReady: false, boundBridgeId: "bridge-stable" };
    let firstGuard = true;
    const first = fetchBridgeSnapshot(context, "http://127.0.0.1:47832", stateA, () => firstGuard);
    const second = fetchBridgeSnapshot(context, "http://127.0.0.1:47832", stateB);
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(calls, ["/health"]);
    resolveHealth({ status: 200, body: JSON.stringify(health) });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(calls.filter(path => path === "/health").length, 1);
    assert.equal(calls.filter(path => path === "/snapshot").length, 1);
    firstGuard = false; // 旧挂载在快照返回前失效，结果只允许新挂载接收。
    resolveSnapshot({ status: 200, body: JSON.stringify(snapshot()) });
    await assert.rejects(first, error => error.code === "stale-result");
    const parsed = await second;
    assert.equal(parsed.bridgeId, "bridge-stable");
    assert.equal(calls.length, 2);
});

test("换桥先同步上屏，清理通道延迟不阻塞未授权", () => {
    let release;
    const context = {
        registerLoopbackChannel: () => new Promise(resolve => { release = resolve; }),
    };
    const state = {
        snapshot: { pending: [pending("1")] },
        status: "ready",
        stopPolling: false,
        needsBaseline: false,
        paintTarget: null,
    };
    let painted = 0;
    applyBridgeIdentityChanged(context, state, "http://127.0.0.1:47832", () => {
        painted += 1;
        assert.equal(state.status, "identity-changed");
        assert.equal(state.snapshot, null);
        assert.equal(state.stopPolling, true);
    });
    assert.equal(painted, 1);
    assert.equal(state.status, "identity-changed");
    assert.equal(state.snapshot, null);
    release();
});

test("全局 stale 不对账、不投递、不写 seen；恢复首帧基线后再投新增", () => {
    const published = [];
    const context = { publishIslandMessage: async message => published.push(message) };
    const state = { baselineDone: true, needsBaseline: false, seen: new Set() };
    const base = snapshot();
    const stale = { ...base, stale: true, pending: [pending("1")] };
    const staleResult = applySnapshotFacts(context, state, stale);
    assert.equal(staleResult.fresh, false);
    assert.equal(published.length, 0);
    assert.equal(state.seen.has(pendingIdentity(stale.bridgeId, stale.pending[0])), false);
    assert.equal(state.needsBaseline, true);

    const recovered = applySnapshotFacts(context, state, { ...base, pending: [pending("1")] });
    assert.equal(recovered.delivered.length, 0); // 恢复首帧只建基线。
    assert.equal(published.length, 0);
    assert.equal(state.seen.size, 1);

    const next = applySnapshotFacts(context, state, { ...base, pending: [pending("1"), pending("2")] });
    assert.deepEqual(next.delivered.map(item => item.id), ["2"]);
    assert.equal(published.length, 1);
});

test("陈旧、时间与短文本", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    assert.equal(isSnapshotFresh(base, at), true);
    assert.equal(isSnapshotFresh({ ...base, stale: true }, at), false);
    assert.match(updateTimeText(base, "更新于", at), /^更新于 \d{2}:\d{2}$/);
    assert.equal(updateTimeText({ ...base, fetchedAt: "" }, "上次更新", at), "更新时间未知");
    assert.ok(Array.from(shortText("一二三四五六七八九十一二三四五六七八九十")).length <= 18);
    assert.ok(shortText("abcdefghijklmnopqrstuvwxyz").length <= 18);
    assert.equal(walletText({ status: "ready", wallets: [{ currency: "CNY", amount: "12.34" }] }), "CNY 12.34");
    assert.equal(walletText({ status: "ready", wallets: [] }), "");
});

test("全局陈旧与余额局部失败分开处理", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const local = {
        ...base,
        stale: false,
        balance: { status: "error", wallets: [], reason: "timeout" },
        pending: [pending("1")],
    };
    assert.equal(isSnapshotTimedFresh(local, at), true);
    assert.equal(isSnapshotFresh(local, at), true);
    assert.equal(islandAnchorView({ status: "ready", snapshot: local }, at).text, "1 项待决");
    assert.match(balanceFailureText(local.balance, local), /超时/);

    const global = { ...local, stale: true };
    assert.equal(isSnapshotFresh(global, at), false);
    assert.equal(islandAnchorView({ status: "ready", snapshot: global }, at).text, "数据陈旧");
    assert.equal(islandAnchorView({ status: "ready", snapshot: { ...base, stale: true } }, at).text, "数据陈旧");
    assert.match(balanceFailureText({ status: "unauthorized", wallets: [] }, local), /401/);
    assert.match(balanceFailureText({ status: "error", wallets: [], reason: "platform" }, local), /平台返回错误/);
    assert.match(balanceFailureText({ status: "error", wallets: [] }, global), /平台错误或超时/);
});

test("未授权状态优先于旧余额/待决，并清空旧事实", () => {
    const old = { ...snapshot(), pending: [pending("1")] };
    const view = islandAnchorView({ status: "identity-changed", error: "桥身份变化，请重新配对", snapshot: old });
    assert.equal(view.text, "未授权");
    assert.doesNotMatch(view.text, /CNY|待决/);
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    const helper = source.slice(source.indexOf("export function markBridgeIdentityChanged"), source.indexOf("export function applyBridgeIdentityChanged"));
    assert.match(helper, /state\.snapshot = null/);
    assert.match(source, /applyBridgeIdentityChanged\(context, state, baseUrl/);
});

test("多币种逐行、不相加；主行余额可读", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const balance = { status: "ready", wallets: [{ currency: "CNY", amount: "12.34" }, { currency: "USD", amount: "3.00" }] };
    assert.deepEqual(walletLines(balance), ["CNY 12.34", "USD 3.00"]);
    assert.equal(walletText(balance), "余额可读");
    assert.equal(islandAnchorView({ status: "ready", snapshot: { ...base, pending: [], balance } }, at).text, "余额可读");
});

test("收起/展开复用轮询会话，不重建基线", () => {
    assert.equal(mountNeedsBaseline({ polling: null, paintTarget: null }), true);
    assert.equal(mountNeedsBaseline({ polling: {}, paintTarget: null }), false);
    assert.equal(mountNeedsBaseline({ polling: {}, paintTarget: {} }), false);
});

test("hover 只读展示且不创建轮询", () => {
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /renderIslandHover/);
    assert.match(source, /plugin-dsh-status-hover/);
    assert.doesNotMatch(source, /setInterval/);
    assert.match(source, /islandSummaryArguments\(view\)/);
    assert.match(source, /context\.setIslandSummary\(summary\.text, summary\.status, summary\.navigation\)/);
    assert.match(source, /context\.setIslandSummary\(null\)/);
    assert.match(source, /uri: ISLAND_OPEN_URI/);
    assert.match(source, /state\.polling\.wake/);
    assert.match(source, /if \(firstMount\) beginMount\(state\)/);
    assert.match(source, /mountSequence/);
    assert.match(source, /state\.mountSequence !== requestSequence/);
    assert.match(source, /inFlightRequests/);
    const renderStart = source.indexOf("export function renderTile");
    const renderEnd = source.indexOf("async function invalidateBridgeChannel", renderStart);
    assert.ok(renderStart > 0 && renderEnd > renderStart);
    assert.doesNotMatch(source.slice(renderStart, renderEnd), /setTimeout|setInterval/);
});

test("折叠态摘要身份变化优先，其次待决数，否则余额或状态", () => {
    assert.equal(collapsedSummary({ status: "identity-changed", snapshot: { pending: [{ id: "p1" }], balance: { status: "ready", wallets: [] } } }), "未授权");
    assert.equal(collapsedSummary({ status: "ready", snapshot: { ...snapshot(), pending: [{ id: "p1" }, { id: "p2" }] } }), "2 项待决");
    assert.equal(collapsedSummary({ status: "ready", snapshot: { ...snapshot(), pending: [] } }), "CNY 12.34");
    assert.equal(collapsedSummary({ status: "unpaired", snapshot: null }), "");
});

test("待决行点击成功移除一条且无提示，失败保留", async () => {
    let calls = 0;
    let release;
    const context = {
        openUri: () => {
            calls += 1;
            return new Promise(resolve => { release = resolve; });
        },
    };
    const item = { id: "p1", since: "s1" };
    const state = { snapshot: { bridgeId: "bridge-a" }, opening: false, dismissedPending: new Set() };
    let removed = 0;
    const row = { remove: () => { removed += 1; } };
    const first = openPendingRow(context, state, item, row);
    const second = openPendingRow(context, state, item, row);
    assert.equal(calls, 1);
    assert.equal(await second, false);
    release();
    assert.equal(await first, true);
    assert.equal(removed, 1);
    assert.equal(state.dismissedPending.has(pendingIdentity("bridge-a", item)), true);
    assert.equal("openMessage" in state, false);

    const originalWarn = console.warn;
    console.warn = () => undefined;
    try {
        let failedRemoved = 0;
        const failedState = { snapshot: { bridgeId: "bridge-a" }, opening: false, dismissedPending: new Set() };
        const failed = await openPendingRow(
            { openUri: async () => { throw new Error("no"); } },
            failedState,
            item,
            { remove: () => { failedRemoved += 1; } },
        );
        assert.equal(failed, false);
        assert.equal(failedRemoved, 0);
        assert.equal(failedState.dismissedPending.size, 0);
    } finally {
        console.warn = originalWarn;
    }
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /row\.addEventListener\("click"/);
    assert.match(source, /openPendingRow\(context, state, item, row\)/);
    assert.doesNotMatch(source, /已打开 DSH，请在 DSH 中处理/);
    assert.doesNotMatch(source, /无法打开 DSH，请在 DSH 中处理/);
    assert.doesNotMatch(source, /plugin-dsh-status__feedback/);
});

test("本地移除标记保留到下一次快照确认消失", () => {
    const base = snapshot();
    const item = base.pending[0];
    const identity = pendingIdentity(base.bridgeId, item);
    const state = { baselineDone: true, needsBaseline: false, seen: new Set(), dismissedPending: new Set([identity]) };
    applySnapshotFacts({}, state, { ...base, pending: [item] });
    assert.equal(state.dismissedPending.has(identity), true);
    applySnapshotFacts({}, state, { ...base, pending: [] });
    assert.equal(state.dismissedPending.has(identity), false);
});

test("金额最多两位小数、去尾零、大数不用科学计数法", () => {
    assert.equal(formatAmount("12.3456"), "12.35");
    assert.equal(formatAmount("12.30"), "12.3");
    assert.equal(formatAmount("12"), "12");
    assert.equal(formatAmount("12.999"), "13");
    assert.equal(formatAmount("0.005"), "0.01");
    assert.equal(formatAmount("12345678901234567890.999"), "12345678901234567891");
    assert.doesNotMatch(formatAmount("12345678901234567890.99"), /e/i);
    assert.equal(walletText({ status: "ready", wallets: [{ currency: "CNY", amount: "12.3456" }] }), "CNY 12.35");
    assert.deepEqual(walletLines({ status: "ready", wallets: [{ currency: "CNY", amount: "12.30" }, { currency: "USD", amount: "3.456" }] }), ["CNY 12.30", "USD 3.46"]);
    assert.equal(formatAmountFixed("12.3"), "12.30");
    assert.equal(formatAmountFixed("12"), "12.00");
    assert.equal(formatAmountFixed("12.3456"), "12.35");
});

test("展开卡无分组折叠，待决最多 5 条", () => {
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.doesNotMatch(source, /createGroup|group-title|groupOpen|toggleGroup|aria-expanded|content\.hidden/);
    assert.match(source, /pendingItems\.slice\(0, 5\)/);
    assert.match(source, /element\("div", "plugin-dsh-status__pending"\)/);
});

test("长标题滚动计划：距离阈值、20 DIP/s、端点停顿与减少动效", () => {
    assert.equal(textScrollPlan(3), null);
    assert.equal(textScrollPlan(100, { reducedMotion: true }), null);
    const plan = textScrollPlan(100);
    assert.equal(plan.travelMs, 5000);
    assert.equal(plan.cycleMs, 11200);
    assert.equal(plan.holdMs, 600);
    const shortPlan = textScrollPlan(4);
    assert.equal(shortPlan.travelMs, 200);
    assert.equal(shortPlan.cycleMs, 4000);
    assert.ok(shortPlan.holdMs >= 600);
    assert.equal(reducedMotionPreferred(), false);
});

test("卡片无卡内滚动，待决行保留长标题滚动", () => {
    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    assert.doesNotMatch(css, /overflow-y:\s*auto|overflow:\s*auto/);
    assert.doesNotMatch(css, /plugin-dsh-status__group/);
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /attachTitleScroll\(row, titleBox, titleText\)/);
    assert.match(source, /pointerenter/);
    assert.match(source, /pointerleave/);
    assert.match(source, /text\.animate/);
    assert.match(source, /prefers-reduced-motion/);
    assert.match(source, /resetTextScrolls\(\)/);
});

test("摘要带 dsh://open 导航，隐藏档只传 null", () => {
    assert.deepEqual(islandSummaryArguments({ visible: true, text: "1 项待决", status: "awaiting" }), {
        text: "1 项待决",
        status: "awaiting",
        navigation: { uri: ISLAND_OPEN_URI },
    });
    assert.equal(islandSummaryArguments({ visible: false, text: "", status: undefined }), null);
});

test("打开地址与上下文约束", () => {
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /dsh:\/\/open/);
    assert.doesNotMatch(source, /"dsh:open"/);
    assert.doesNotMatch(source, /__DESKTOP_TOOLS_NATIVE__/);
    assert.doesNotMatch(source, /data\.token/);
    assert.match(source, /islandSummaryArguments\(view\)/);
    assert.match(source, /context\.setIslandSummary\(summary\.text, summary\.status, summary\.navigation\)/);
    assert.match(source, /context\.setIslandSummary\(null\)/);
    assert.match(source, /uri: ISLAND_OPEN_URI/);
    const manifest = readFileSync(new URL("./manifest.ini", import.meta.url), "utf8");
    assert.match(manifest, /icon = icon\.svg/);
    const icon = readFileSync(new URL("./icon.svg", import.meta.url), "utf8");
    assert.match(icon, /viewBox="0 0 24 24"/);
});
