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
    hoverBalance,
    fitHoverBalance,
    observeHoverWidth,
    islandHoverNode,
    islandHoverView,
    islandSummaryArguments,
    currencySymbol,
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
    walletSymbolAmount,
    updateTimeText,
    walletLines,
    walletText,
    settingsEntries,
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

/** 最小 DOM 桩：只实现 islandHoverNode 用到的节点接口，用来断言真实生成的节点结构与属性。 */
const 创建元素桩 = () => ({
    tagName: "",
    className: "",
    title: "",
    textContent: "",
    attributes: {},
    children: [],
    style: {},
    setAttribute(name, value) {
        this.attributes[name] = String(value);
    },
    append(...nodes) {
        for (const node of nodes) this.children.push(node);
    },
    appendChild(node) {
        this.children.push(node);
        return node;
    },
});

/** 安装临时 document 桩渲染一次，结束后恢复全局，避免影响其他用例。 */
const 渲染节点 = view => {
    const 原document = globalThis.document;
    globalThis.document = {
        createElement(tag) {
            const node = 创建元素桩();
            node.tagName = tag.toUpperCase();
            return node;
        },
    };
    try {
        return islandHoverNode(view);
    } finally {
        if (原document === undefined) delete globalThis.document;
        else globalThis.document = 原document;
    }
};

/** 深度优先遍历生成的节点树。 */
const 遍历节点 = (node, visit) => {
    visit(node);
    for (const child of node.children) 遍历节点(child, visit);
};

/** 按类名找第一个节点（修饰类按空格拆分匹配）。 */
const 含类 = (node, className) => {
    let found;
    遍历节点(node, 项 => {
        if (String(项.className).split(/\s+/).includes(className)) found = 项;
    });
    return found;
};

/** 按深度优先顺序收集命中的类名，用来断言读屏顺序。 */
const 类名顺序 = (node, 关注) => {
    const 顺序 = [];
    遍历节点(node, 项 => {
        for (const name of String(项.className).split(/\s+/)) if (关注.includes(name)) 顺序.push(name);
    });
    return 顺序;
};

test("hover 只读视图：任务区、分数进度、余额与缺失降级", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const running = { ...base, sessions: [{ ...base.sessions[0], status: "running" }] };
    const taskStats = overrides => ({
        runCount: 3,
        unreadCount: 12,
        completedUnreadCount: 4,
        unionCount: 14,
        completeness: "complete",
        sampledAt: base.fetchedAt,
        ...overrides,
    });
    const view = (patch = {}) => islandHoverView({ status: "ready", snapshot: { ...running, ...patch } }, at);

    // 正常态：运行数是主值，未读/完成未读进右列，进度只给分数，余额固定两位。
    const full = view({ taskStats: taskStats() });
    assert.deepEqual(full.task, { label: "运行任务", value: "3" });
    assert.deepEqual(full.counts, [{ label: "未读", value: "12" }, { label: "完成未读", value: "4" }]);
    assert.equal(full.note, undefined);
    assert.deepEqual(full.progress, {
        completed: 4,
        total: 14,
        percent: 29,
        fraction: "4/14",
        label: "待关注任务中完成未读比例，完成 4 项，共 14 项，29%",
    });
    assert.deepEqual(full.balance, { value: "￥12.34" });
    assert.equal(full.status, undefined);

    // 无 taskStats：运行数退回可见会话数，计数未知写「—」，说明写未读状态不可用。
    const noStats = view();
    assert.deepEqual(noStats.task, { label: "可见会话中运行", value: "1" });
    assert.deepEqual(noStats.counts, [{ label: "未读", value: "—" }, { label: "完成未读", value: "—" }]);
    assert.equal(noStats.note, "未读状态不可用");
    assert.equal(noStats.progress, undefined);
    assert.deepEqual(noStats.balance, { value: "￥12.34" });

    // 统计不完整：保留可验证运行数，计数撤成「—」，不画比例，余额照常。
    const missingCompleted = view({ taskStats: taskStats({ completedUnreadCount: undefined }) });
    assert.deepEqual(missingCompleted.task, { label: "运行任务", value: "3" });
    assert.deepEqual(missingCompleted.counts, [{ label: "未读", value: "—" }, { label: "完成未读", value: "—" }]);
    assert.equal(missingCompleted.note, "统计不完整");
    assert.equal(missingCompleted.progress, undefined);
    assert.deepEqual(missingCompleted.balance, { value: "￥12.34" });
    assert.equal(view({ taskStats: taskStats({ unreadCount: 1, completedUnreadCount: 2 }) }).note, "统计不完整");
    assert.equal(view({ taskStats: taskStats({ unionCount: 2, runCount: 3 }) }).note, "统计不完整");
    // 缺 runCount 时只用可见会话数，不当作任务数。
    const missingRun = view({ taskStats: taskStats({ runCount: undefined, unreadCount: 1, completedUnreadCount: 1, unionCount: 2 }) });
    assert.deepEqual(missingRun.task, { label: "可见会话中运行", value: "1" });
    assert.equal(missingRun.note, "统计不完整");
    // completeness 非 complete 或采样超出两轮：整份统计不可用。
    assert.equal(view({ taskStats: taskStats({ completeness: "partial" }) }).note, "未读状态不可用");
    assert.equal(view({ taskStats: taskStats({ completeness: undefined }) }).note, "未读状态不可用");
    assert.equal(view({ taskStats: taskStats({ sampledAt: new Date(Date.parse(base.fetchedAt) - 90000).toISOString() }) }).note, "未读状态不可用");
    // 完成未读为 0 不画比例；四项权威全 0 才是无待关注任务。
    const completedZero = view({ taskStats: taskStats({ completedUnreadCount: 0 }) });
    assert.equal(completedZero.progress, undefined);
    assert.deepEqual(completedZero.counts, [{ label: "未读", value: "12" }, { label: "完成未读", value: "0" }]);
    const noTasks = islandHoverView({
        status: "ready",
        snapshot: { ...running, sessions: [], pending: [], taskStats: taskStats({ runCount: 0, unreadCount: 0, completedUnreadCount: 0, unionCount: 0 }) },
    }, at);
    assert.deepEqual(noTasks.task, { value: "暂无待关注任务" });
    assert.deepEqual(noTasks.counts, [{ label: "未读", value: "0" }, { label: "完成未读", value: "0" }]);
    assert.equal(noTasks.progress, undefined);
    // 会话范围未知或已知没有运行：只写原因，不写 0。
    assert.deepEqual(view({ sessions: undefined, hasSessions: false }).task, { value: "任务状态未知" });
    assert.deepEqual(islandHoverView({ status: "ready", snapshot: { ...running, sessions: [{ ...base.sessions[0], status: "idle" }] } }, at).task, { value: "近期没有运行会话" });

    // 数据陈旧与整卡状态：只写状态与下一步，不沿用历史读数。
    const stale = view({ fetchedAt: "2026-11-20T00:00:00.000Z" });
    assert.deepEqual({ word: stale.status.word }, { word: "数据陈旧" });
    assert.match(stale.status.hint, /^(上次更新 |更新时间未知)/);
    assert.equal(stale.task, undefined);
    assert.equal(stale.counts, undefined);
    assert.equal(stale.balance, undefined);
    assert.equal(view({ stale: true }).status.word, "数据陈旧");
    const 状态表 = [
        [{ status: "unpaired", snapshot: null }, "未配对", "在设置 → 插件 → DSH 接入完成配对"],
        [{ status: "unreachable", snapshot: null }, "未连接", "请确认 DSH 桌面桥正在运行"],
        [{ status: "unauthorized", snapshot: base }, "未授权", "请重新配对"],
        [{ status: "identity-changed", snapshot: base }, "未授权", "请重新配对"],
        [{ status: "logged-out", snapshot: null }, "账号未登录", "在 DSH 登录 DeepSeek 账号"],
        [{ status: "error", snapshot: null }, "读取失败", "请稍后重试"],
        [{ status: "loading", snapshot: null }, "正在读取 DSH", ""],
        [{ status: "ready", snapshot: null }, "正在读取 DSH", ""],
    ];
    for (const [state, word, hint] of 状态表) assert.deepEqual(islandHoverView(state, at), { status: { word, hint } });

    // 余额：多币种只取快照顺序第一个，不求和；无符号币种隐藏整行且不改选下一个。
    assert.deepEqual(view({ balance: { status: "ready", wallets: [{ currency: "CNY", amount: "12.3456" }, { currency: "USD", amount: "3.00" }] } }).balance, { value: "￥12.35" });
    assert.deepEqual(view({ balance: { status: "ready", wallets: [{ currency: "USD", amount: "3" }] } }).balance, { value: "$3.00" });
    assert.equal(view({ balance: { status: "ready", wallets: [{ currency: "XYZ", amount: "9.9" }, { currency: "CNY", amount: "1" }] } }).balance, undefined);
    // 首项不可用时不看第二项：不把 USD 4 显示成 $4.00，也不回退到下一个钱包。
    assert.equal(view({ balance: { status: "ready", wallets: [{ currency: "XYZ", amount: "9.9" }, { currency: "USD", amount: "4" }] } }).balance, undefined);
    assert.equal(view({ balance: { status: "ready", wallets: [{ currency: "CNY", amount: "" }, { currency: "USD", amount: "4" }] } }).balance, undefined);
    assert.equal(view({ balance: { status: "ready", wallets: [{ currency: "CNY", amount: "abc" }, { currency: "USD", amount: "4" }] } }).balance, undefined);
    assert.deepEqual(view({ balance: { status: "ready", wallets: "nope" } }).balance, { reason: "无可用余额" });
    // 已知原因分开写：未登录 / 未授权 / 读取失败 / 无可用余额。
    assert.deepEqual(view({ balance: { status: "logged-out", wallets: [] } }).balance, { reason: "账号未登录" });
    assert.deepEqual(view({ balance: { status: "unauthorized", wallets: [] } }).balance, { reason: "平台未授权余额读取" });
    assert.deepEqual(view({ balance: { status: "error", wallets: [] } }).balance, { reason: "余额读取失败" });
    assert.deepEqual(view({ balance: { status: "mystery", wallets: [] } }).balance, { reason: "余额读取失败" });
    assert.deepEqual(view({ balance: { status: "ready", wallets: [] } }).balance, { reason: "无可用余额" });
    // 余额不可用时任务区与可证进度照常。
    const 余额不可用 = view({ balance: { status: "unauthorized", wallets: [] }, taskStats: taskStats() });
    assert.deepEqual(余额不可用.task, { label: "运行任务", value: "3" });
    assert.equal(余额不可用.progress.fraction, "4/14");

    let notified = 0;
    const unsubscribe = subscribeState(running, () => { notified += 1; });
    notifyState(running);
    assert.equal(notified, 1);
    unsubscribe();
    notifyState(running);
    assert.equal(notified, 1);
});

test("hover 余额：币种符号映射与首钱包规则", () => {
    assert.equal(currencySymbol("cny"), "￥");
    assert.equal(currencySymbol(" USD "), "$");
    assert.equal(currencySymbol("XYZ"), "");
    assert.equal(currencySymbol(undefined), "");
    assert.equal(walletSymbolAmount({ currency: "CNY", amount: "12.3" }), "￥12.30");
    assert.equal(walletSymbolAmount({ currency: "CNY", amount: "-12.345" }), "-￥12.35");
    assert.equal(walletSymbolAmount({ currency: "XYZ", amount: "12.3" }), "");
    assert.equal(walletSymbolAmount({ currency: "CNY", amount: "待定" }), "");
    assert.deepEqual(hoverBalance({ status: "ready", wallets: [{ currency: "USD", amount: "3" }, { currency: "CNY", amount: "1" }] }), { value: "$3.00" });
    assert.equal(hoverBalance({ status: "ready", wallets: [{ currency: "XYZ", amount: "1" }, { currency: "CNY", amount: "2" }] }), undefined);
    assert.deepEqual(hoverBalance({ status: "ready", wallets: [] }), { reason: "无可用余额" });
    assert.deepEqual(hoverBalance({ status: "logged-out", wallets: [] }), { reason: "账号未登录" });
    // 已知原因分开写，未知状态按读取失败；不臆造新原因。
    assert.deepEqual(hoverBalance({ status: "unauthorized", wallets: [] }), { reason: "平台未授权余额读取" });
    assert.deepEqual(hoverBalance({ status: "error", wallets: [] }), { reason: "余额读取失败" });
    assert.deepEqual(hoverBalance(undefined), { reason: "余额读取失败" });
    // 首项失效不跳项：第二项 USD 4 不得被显示成 $4.00。
    assert.equal(hoverBalance({ status: "ready", wallets: [{ currency: "XYZ", amount: "9.9" }, { currency: "USD", amount: "4" }] }), undefined);
    assert.equal(hoverBalance({ status: "ready", wallets: [{ currency: "USD", amount: "" }, { currency: "CNY", amount: "4" }] }), undefined);
});

test("hover DOM：任务区/计数/进度/余额的真实结构与读屏数值", () => {
    const base = snapshot();
    const at = Date.parse(base.fetchedAt) + 1000;
    const view = islandHoverView({
        status: "ready",
        snapshot: {
            ...base,
            sessions: [{ ...base.sessions[0], status: "running" }],
            taskStats: { runCount: 3, unreadCount: 12, completedUnreadCount: 4, unionCount: 14, completeness: "complete", sampledAt: base.fetchedAt },
            balance: { status: "ready", wallets: [{ currency: "CNY", amount: "12.3" }] },
        },
    }, at);
    const 根 = 渲染节点(view);
    assert.equal(根.attributes.role, "group");
    assert.equal(根.attributes["aria-label"], "DSH 任务概览");
    assert.equal(含类(根, "plugin-dsh-status-hover__task-label").textContent, "运行任务");
    const 主值 = 含类(根, "plugin-dsh-status-hover__task-value");
    assert.equal(主值.textContent, "3");
    assert.match(主值.className, /__task-value--numeric/);
    assert.deepEqual(含类(根, "plugin-dsh-status-hover__counts").children.map(节点 => 节点.textContent), ["未读 12", "完成未读 4"]);
    const 进度 = 含类(根, "plugin-dsh-status-hover__progress");
    assert.equal(进度.attributes.role, "progressbar");
    assert.equal(进度.attributes["aria-valuemin"], "0");
    assert.equal(进度.attributes["aria-valuemax"], "100");
    assert.equal(进度.attributes["aria-valuenow"], "29");
    assert.equal(进度.attributes["aria-label"], "完成未读进度");
    assert.match(进度.attributes["aria-valuetext"], /完成 4 项，共 14 项，29%/);
    assert.equal(含类(根, "plugin-dsh-status-hover__fraction").textContent, "4/14");
    assert.equal(含类(根, "plugin-dsh-status-hover__fill").style.width, "29%");
    assert.equal(含类(根, "plugin-dsh-status-hover__balance-label").textContent, "余额");
    assert.equal(含类(根, "plugin-dsh-status-hover__balance-value").textContent, "￥12.30");
    // 读屏顺序：任务区 → 计数 → 进度 → 余额。
    assert.deepEqual(类名顺序(根, ["plugin-dsh-status-hover__task", "plugin-dsh-status-hover__counts", "plugin-dsh-status-hover__progress", "plugin-dsh-status-hover__balance"]), [
        "plugin-dsh-status-hover__task",
        "plugin-dsh-status-hover__counts",
        "plugin-dsh-status-hover__progress",
        "plugin-dsh-status-hover__balance",
    ]);

    // 只读：不得出现可点击/可聚焦节点，也不得出现 button/link/status 角色与内联事件。
    const 全部 = [];
    遍历节点(根, 节点 => 全部.push(节点));
    for (const 节点 of 全部) {
        assert.ok(!["BUTTON", "A", "INPUT", "SELECT", "TEXTAREA"].includes(节点.tagName), `不得出现可交互元素 ${节点.tagName}`);
        assert.equal(节点.attributes.tabindex, undefined);
        assert.equal(节点.attributes.href, undefined);
        assert.equal(节点.onclick, undefined);
        assert.ok(!["button", "link", "status"].includes(节点.attributes.role), `不得出现 role=${节点.attributes.role}`);
    }

    // 整卡状态只写状态词与下一步；余额行按需缺席。
    const 陈旧 = 渲染节点({ status: { word: "数据陈旧", hint: "上次更新 14:32" } });
    assert.equal(陈旧.children[0].textContent, "数据陈旧");
    assert.equal(陈旧.children[1].textContent, "上次更新 14:32");
    assert.equal(含类(陈旧, "plugin-dsh-status-hover__balance"), undefined);
    const 无符号 = islandHoverView({
        status: "ready",
        snapshot: { ...base, balance: { status: "ready", wallets: [{ currency: "XYZ", amount: "9.9" }, { currency: "CNY", amount: "1" }] } },
    }, at);
    const 无符号根 = 渲染节点(无符号);
    assert.equal(含类(无符号根, "plugin-dsh-status-hover__balance"), undefined);
    assert.equal(含类(无符号根, "plugin-dsh-status-hover__task-value").className, "plugin-dsh-status-hover__task-value");
    assert.equal(含类(无符号根, "plugin-dsh-status-hover__note").textContent, "未读状态不可用");
});

test("hover 只读实现约束：无定时器与点击目标，256 宽、4px 轨与既有令牌", () => {
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    const hover = source.slice(source.indexOf("export function renderIslandHover"), source.indexOf("export function subscribeState"));
    assert.ok(hover.length > 0);
    assert.doesNotMatch(hover, /setTimeout|setInterval|fetch\(|requestLoopback|registerLoopback/);
    assert.doesNotMatch(hover, /role="button"|tabIndex|<button/);
    assert.doesNotMatch(hover, /role="status"/);
    assert.doesNotMatch(hover, /\p{Extended_Pictographic}/u);
    assert.match(hover, /const unsubscribe = subscribeState\(state, paintHover\)/);
    assert.match(hover, /const stopWidth = observeHoverWidth\(container, \(\) => \{/); // 容器宽度变化后重测余额行。
    assert.match(hover, /return \(\) => \{\s*unsubscribe\(\);\s*stopWidth\(\);\s*\}/); // 订阅与宽度观察一起清理。
    assert.match(hover, /aria-valuetext/);
    assert.match(source, /const collapsed = context\.collapsed === true;/);
    assert.doesNotMatch(source, /aria-busy/);

    const css = readFileSync(new URL("./style.css", import.meta.url), "utf8");
    const hoverCss = css.slice(css.indexOf(".plugin-dsh-status-hover"), css.indexOf(".plugin-dsh-status__collapsed-text"));
    assert.match(hoverCss, /max-width:\s*256px/);
    assert.match(hoverCss, /height:\s*4px/);
    assert.match(hoverCss, /font-size:\s*26px/);
    assert.match(hoverCss, /font-weight:\s*600/);
    assert.match(hoverCss, /color:\s*var\(--fence-text\)/);
    assert.match(hoverCss, /color:\s*var\(--fence-muted\)/);
    assert.match(hoverCss, /border-block-start:\s*1px solid var\(--fence-border\)/);
    assert.match(hoverCss, /\.plugin-dsh-status-hover__balance \{[^}]*flex-wrap:\s*wrap/); // 金额放不下先换到余额区下一行。
    const 金额规则 = hoverCss.slice(hoverCss.indexOf(".plugin-dsh-status-hover__balance-value,"), hoverCss.indexOf(".plugin-dsh-status-hover__balance-value {"));
    assert.doesNotMatch(金额规则, /text-overflow|overflow:\s*hidden|white-space:\s*nowrap/); // 不把金额截成省略号，溢出交给 fitHoverBalance 撤行。
    assert.doesNotMatch(hoverCss, /#(?:[0-9a-fA-F]{3,8})\b/); // 不写死颜色
    assert.doesNotMatch(hoverCss, /rgba?\(/);
    assert.doesNotMatch(hoverCss, /overflow(?:-[xy])?:\s*(?:auto|scroll)/); // 卡内不滚动
    assert.doesNotMatch(hoverCss, /--fence-(?!text|muted|border)[a-z-]+/); // 只消费既有三枚令牌
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

test("hover 余额行：232 宽长金额换行后仍溢出则整行撤下，不截断", () => {
    const 视图 = { task: { label: "运行任务", value: "3" }, counts: [{ label: "未读", value: "12" }, { label: "完成未读", value: "4" }], balance: { value: "￥1234567890.12" } };
    const 根 = 渲染节点(视图);
    const 行 = 含类(根, "plugin-dsh-status-hover__balance");
    const 金额 = 含类(行, "plugin-dsh-status-hover__balance-value");
    // 宿主还没完成布局（可用宽度为 0）时不改 DOM，等下次重绘再量。
    assert.equal(fitHoverBalance(根), false);
    assert.equal(行.style.display, undefined);
    // 232 DIP 下换到余额区独立一行仍放不下：整行撤下，标签与分隔线一起撤掉。
    行.clientWidth = 232;
    金额.scrollWidth = 240;
    assert.equal(fitHoverBalance(根), true);
    assert.equal(行.style.display, "none");
    assert.equal(行.hidden, true);
    // 放得下（含半 DIP 容差）保持原样。
    const 放得下根 = 渲染节点(视图);
    const 放得下行 = 含类(放得下根, "plugin-dsh-status-hover__balance");
    放得下行.clientWidth = 232;
    含类(放得下行, "plugin-dsh-status-hover__balance-value").scrollWidth = 96;
    assert.equal(fitHoverBalance(放得下根), false);
    assert.equal(放得下行.style.display, undefined);
    // 原因行同样按可用宽度判断。
    const 原因根 = 渲染节点({ ...视图, balance: { reason: "平台未授权余额读取" } });
    const 原因行 = 含类(原因根, "plugin-dsh-status-hover__balance");
    原因行.clientWidth = 96;
    含类(原因行, "plugin-dsh-status-hover__balance-reason").scrollWidth = 120;
    assert.equal(fitHoverBalance(原因根), true);
    // 首钱包不可用（没有余额行）时无副作用。
    assert.equal(fitHoverBalance(渲染节点({ ...视图, balance: undefined })), false);
    // 渲染入口必须在挂载到文档之后再测量。
    const source = readFileSync(new URL("./index.js", import.meta.url), "utf8");
    assert.match(source, /container\.append\(node\);\s*fitHoverBalance\(node\);/);
});

test("hover 原始首项：解析保留快照首项，视图不回退第二个钱包", () => {
    const 解析 = (wallets) => parseSnapshotPayload({ ...snapshot(), balance: { status: "ready", wallets } });
    const 视图 = (解析结果) => islandHoverView({ status: "ready", snapshot: 解析结果 }, Date.parse(解析结果.fetchedAt) + 1000);
    // [null, CNY 19]：原始首项是 null，整行隐藏（旧实现过滤后会把第二项显示成 ￥19.00）。
    const 空首项 = 解析([null, { currency: "CNY", amount: "19" }]);
    assert.equal(空首项.balance.firstWallet, null);
    assert.deepEqual(空首项.balance.wallets, [{ currency: "CNY", amount: "19" }]); // 其它卡的有效钱包列表保持原样。
    assert.equal(walletText(空首项.balance), "CNY 19");
    assert.equal(视图(空首项).balance, undefined);
    // 首项金额不是字符串（17）：隐藏，不换下一项。
    const 数字首项 = 解析([{ currency: "CNY", amount: 17 }, { currency: "CNY", amount: "19" }]);
    assert.equal(数字首项.balance.firstWallet, null);
    assert.equal(视图(数字首项).balance, undefined);
    // 首项币种未知：隐藏，不回退第二项。
    const 未知币种 = 解析([{ currency: "XYZ", amount: "1" }, { currency: "CNY", amount: "19" }]);
    assert.deepEqual(未知币种.balance.firstWallet, { currency: "XYZ", amount: "1" });
    assert.equal(视图(未知币种).balance, undefined);
    // 首项金额非法：隐藏。
    assert.equal(视图(解析([{ currency: "CNY", amount: "abc" }, { currency: "USD", amount: "4" }])).balance, undefined);
    // 首项有效：只显示首项金额。
    assert.deepEqual(视图(解析([{ currency: "CNY", amount: "17" }, { currency: "USD", amount: "4" }])).balance, { value: "￥17.00" });
    // 空钱包与「首项非法」区分：前者写无可用余额。
    assert.deepEqual(视图(解析([])).balance, { reason: "无可用余额" });
    // 非 ready 状态不受首项影响。
    const 未授权 = parseSnapshotPayload({ ...snapshot(), balance: { status: "unauthorized", wallets: [null] } });
    assert.deepEqual(视图(未授权).balance, { reason: "平台未授权余额读取" });
    // 直接构造的余额（没有 firstWallet）仍按 wallets[0] 判定，兼容旧调用。
    assert.deepEqual(hoverBalance({ status: "ready", wallets: [{ currency: "CNY", amount: "1" }, { currency: "USD", amount: "2" }] }), { value: "￥1.00" });
});

test("hover 余额行：容器宽度变化后重测，撤行可恢复（0→232、232→256）", () => {
    const 观察器 = [];
    const 原ResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
        constructor(回调) {
            this.回调 = 回调;
            this.已断开 = false;
            观察器.push(this);
        }
        observe() {}
        disconnect() {
            this.已断开 = true;
        }
    };
    try {
        const 根 = 渲染节点({ task: { label: "运行任务", value: "3" }, counts: [{ label: "未读", value: "12" }, { label: "完成未读", value: "4" }], balance: { value: "￥1234567890.12" } });
        const 行 = 含类(根, "plugin-dsh-status-hover__balance");
        const 金额 = 含类(行, "plugin-dsh-status-hover__balance-value");
        金额.scrollWidth = 240;
        let 宽 = 0;
        const 容器 = { children: [], replaceChildren() { this.children = []; }, append(node) { this.children.push(node); }, getBoundingClientRect: () => ({ left: 0, top: 0, width: 宽, height: 0 }) };
        容器.append(根);
        let 重测 = 0;
        const 停止 = observeHoverWidth(容器, () => {
            重测 += 1;
            行.clientWidth = 宽; // 模拟布局：行可用宽度随容器变化。
            fitHoverBalance(根);
        });
        assert.equal(观察器.length, 1);
        // 挂载时 0 宽：量不到就不撤行。
        assert.equal(fitHoverBalance(根), false);
        // 0 → 232：金额 240 放不下，撤行。
        宽 = 232;
        观察器[0].回调();
        assert.equal(重测, 1);
        assert.equal(行.style.display, "none");
        // 同宽重复回调（高度抖动）不重复重测。
        观察器[0].回调();
        assert.equal(重测, 1);
        // 232 → 256：放得下，行恢复。
        宽 = 256;
        观察器[0].回调();
        assert.equal(重测, 2);
        assert.equal(行.style.display, "");
        assert.equal(Boolean(行.hidden), false);
        停止();
        assert.equal(观察器[0].已断开, true); // 卸载时断开观察器。
        // 没有 ResizeObserver 的宿主不报错，返回空清理函数。
        delete globalThis.ResizeObserver;
        const 空清理 = observeHoverWidth({ getBoundingClientRect: () => ({ width: 200 }) }, () => {});
        assert.equal(typeof 空清理, "function");
        空清理();
    } finally {
        if (原ResizeObserver === undefined) delete globalThis.ResizeObserver;
        else globalThis.ResizeObserver = 原ResizeObserver;
    }
});

/** 带监听与 disabled 的元素桩，用来驱动设置条目的真实交互。 */
const 创建交互桩 = () => ({
    tagName: "",
    className: "",
    title: "",
    textContent: "",
    type: "",
    value: "",
    disabled: false,
    hidden: false,
    attributes: {},
    children: [],
    style: {},
    dataset: {},
    listeners: {},
    setAttribute(name, value) {
        this.attributes[name] = String(value);
    },
    append(...nodes) {
        for (const node of nodes) this.children.push(node);
    },
    appendChild(node) {
        this.children.push(node);
        return node;
    },
    replaceChildren(...nodes) {
        this.children = [...nodes];
    },
    addEventListener(name, handler) {
        (this.listeners[name] ??= []).push(handler);
    },
    click() {
        for (const handler of this.listeners.click ?? []) handler();
    },
});

/** 深度优先找第一个满足条件的节点。 */
const 找节点 = (node, 判定) => {
    if (判定(node)) return node;
    for (const child of node.children ?? []) {
        const found = 找节点(child, 判定);
        if (found) return found;
    }
    return undefined;
};

/** 安装 document 桩并渲染一次 DSH 配对条目。 */
const 渲染配对条目 = context => {
    const 原document = globalThis.document;
    globalThis.document = {
        head: 创建交互桩(),
        querySelector: () => null, // 只测设置条目本身，样式表插入按「未插入」处理。
        createElement(tag) {
            const node = 创建交互桩();
            node.tagName = tag.toUpperCase();
            return node;
        },
    };
    try {
        const container = 创建交互桩();
        settingsEntries.find(entry => entry.id === "dsh-pairing").render(container, context);
        return container;
    } finally {
        if (原document === undefined) delete globalThis.document;
        else globalThis.document = 原document;
    }
};

test("无实例时配对/取消配对不可执行，也不写本地状态（t113）", async () => {
    const 记录 = { 保存: [], 清凭据: 0 };
    const 上下文 = {
        instanceId: "", // 宿主没有可用的格子或浮岛实例。
        settings: { baseUrl: "http://127.0.0.1:47832", paired: true, bridgeId: "bridge-stable" },
        saveSettings: async value => {
            记录.保存.push(value);
        },
        // 无实例时宿主不下发任何能力。
    };
    const container = 渲染配对条目(上下文);
    const 按钮 = [];
    找节点(container, node => {
        if (node.tagName === "BUTTON") 按钮.push(node);
        return false;
    });
    assert.equal(按钮.length, 2, "应有配对与取消配对两个按钮");
    assert.ok(按钮.every(button => button.disabled === true), "无实例时两个按钮都禁用");
    const 提示 = 找节点(container, node => String(node.className).includes("plugin-dsh-status-settings__hint"));
    assert.ok(提示, "应有依赖说明");
    assert.match(提示.textContent, /需先添加该格子或启用浮岛卡/);
    assert.equal(提示.hidden, false, "无实例时说明可见");
    按钮.forEach(button => button.click());
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(记录.保存.length, 0, "无实例点击不得写 paired:false 或任何设置");
    const 状态 = 找节点(container, node => String(node.className).includes("plugin-dsh-status-settings__status"));
    assert.equal(状态.textContent, "已配对", "状态文字保持服务端事实，不显示「已取消配对」");
});

test("有实例时取消配对仍清凭据、清回环并写 paired:false（t113 回归）", async () => {
    const 记录 = { 保存: [], 清凭据: 0, 回环: [] };
    const 上下文 = {
        instanceId: "island.plugin.dsh-status",
        settings: { baseUrl: "http://127.0.0.1:47832", paired: true, bridgeId: "bridge-stable" },
        clearCredential: async () => {
            记录.清凭据 += 1;
        },
        registerLoopbackChannel: async registration => {
            记录.回环.push(registration);
        },
        saveSettings: async value => {
            记录.保存.push(value);
        },
    };
    const container = 渲染配对条目(上下文);
    const 取消 = (() => {
        let found;
        找节点(container, node => {
            if (node.tagName === "BUTTON" && node.textContent === "取消配对") found = node;
            return false;
        });
        return found;
    })();
    assert.equal(取消.disabled, false, "有实例时可取消配对");
    取消.click();
    for (let i = 0; i < 20 && 记录.保存.length === 0; i += 1) await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(记录.清凭据, 1);
    assert.ok(记录.回环.length >= 1, "应调用回环通道清理");
    assert.equal(记录.保存.length, 1);
    assert.equal(记录.保存[0].paired, false);
    assert.equal(记录.保存[0].bridgeId, "");
});
