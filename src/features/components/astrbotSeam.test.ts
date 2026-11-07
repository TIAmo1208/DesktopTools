import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { parseComponentHostEvent } from "../../contracts/component.ts";

// 真实插件模块：测试直接加载产品代码，不复制解析与渲染逻辑。
const plugin = await import("../../../plugin/AstrBot 状态/index.js");

// 插件渲染用自己的 Date.now() 计算过期，固定时刻必须贴近真实时钟。
const NOW_MS = Date.now();
const nowSeconds = Math.floor(NOW_MS / 1000);

/** 生成一份契约 v2 快照，字段与 Python 采集端一致。 */
const snapshotV2 = (overrides: Record<string, unknown> = {}) => ({
    schemaVersion: 2,
    collectedAt: nowSeconds - 3,
    service: { online: true, probe: { level: "ok", checkedAt: nowSeconds - 3 }, startTime: nowSeconds - 3_600 },
    platforms: {
        summary: { total: 4, running: 3, error: 1 },
        items: [
            { id: "qq", displayName: "QQ 主号", status: "running", startedAt: nowSeconds - 7_200, errorCount: 0, lastError: null },
            {
                id: "tg",
                displayName: "Telegram",
                status: "error",
                startedAt: nowSeconds - 600,
                errorCount: 7,
                lastError: { message: "连接被重置", timestamp: nowSeconds - 60 },
            },
        ],
    },
    resources: { cpuPercent: 18.4, memory: { processMb: 412.6, systemMb: 15_320.2 }, threadCount: 62 },
    messages: {
        total: 1284,
        series: [
            { ts: nowSeconds - 3_600, count: 10 },
            { ts: nowSeconds, count: 30 },
        ],
    },
    logs: {
        tail: [
            { ts: nowSeconds - 300, level: "INFO", text: "心跳" },
            { ts: nowSeconds - 120, level: "WARNING", text: "发送超时" },
            { ts: nowSeconds - 60, level: "ERROR", text: "连接关闭" },
        ],
    },
    ...overrides,
});

/** 最小 DOM 节点，够 renderTile 画完一棵内容树并读回文本。 */
class FakeNode {
    className = "";
    textContent = "";
    title = "";
    style: Record<string, string> = {};
    dataset: Record<string, string> = {};
    children: FakeNode[] = [];
    attrs: Record<string, string> = {};
    hidden = false; // 与真实 DOM 一致：hidden 节点不参与可见文本断言。
    clientWidth = 420;
    clientHeight = 300;
    append(...nodes: FakeNode[]) {
        this.children.push(...nodes);
    }
    replaceChildren(...nodes: FakeNode[]) {
        this.children = nodes;
    }
    setAttribute(name: string, value: string) {
        this.attrs[name] = value;
    }
    remove() {}
    addEventListener() {}
    removeEventListener() {}
    querySelector() {
        return null;
    }
    /** 递归拼接可见文本，供断言格子实际显示内容；hidden 节点按不可见处理。 */
    text(): string {
        if (this.hidden) return "";
        return [this.textContent, ...this.children.map((child) => child.text())].join(" ");
    }
}

/** 安装渲染所需的最小浏览器全局；每个测试结束后由调用方清理。 */
const installDom = () => {
    const cleanup: Array<() => void> = [];
    const target = globalThis as Record<string, unknown>;
    const previous = { document: target.document, window: target.window };
    target.document = {
        visibilityState: "visible",
        createElement: () => new FakeNode(),
        querySelector: () => null,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        head: { appendChild: () => undefined },
    };
    target.window = { open: () => null };
    target.ResizeObserver = class {
        observe() {}
        disconnect() {}
    };
    target.IntersectionObserver = class {
        observe() {}
        disconnect() {}
    };
    target.requestAnimationFrame = (callback: () => void) => setTimeout(callback, 0);
    target.cancelAnimationFrame = (handle: number) => clearTimeout(handle);
    cleanup.push(() => {
        target.document = previous.document;
        target.window = previous.window;
    });
    return () => {
        for (const restore of cleanup) restore();
    };
};

interface RawRequestEntry {
    sequence: number;
    receivedAt: number;
    method: string;
    path: string;
    headers: Array<{ name: string; value: string }>;
    body: string;
}

/**
 * 假宿主：按 ADR-0038 契约实现三条命令与事件时序。
 * 通道注册成功后才接受推送；首次读取后才发通知；读到即消费。
 */
const createHost = () => {
    const listeners: Array<(event: unknown) => void> = [];
    const registrations: Array<Record<string, unknown>> = [];
    const reads: Array<{ instanceId: string; channelId: string; afterSequence?: number }> = [];
    const requests: Array<Record<string, unknown>> = [];
    const buffers = new Map<string, RawRequestEntry[]>();
    const counters = new Map<string, number>();
    const subscribed = new Set<string>();
    const activeChannels = new Set<string>(); // 宿主当前占用的实例通道，同名重注册应替换而不是报端口占用。
    let replaced = 0; // 同名通道被替换的次数。
    let eventSequence = 0;
    let registerError: { code: string; message: string } | null = null; // 模拟凭据缺失/端口占用等注册失败。
    let credentialMissing = false; // 模拟 Windows 凭据管理器里没有共享 token。
    let tokenValid = true; // 模拟对端对共享 token 的校验结果。
    let pullBody = ""; // 拉取兜底通常不参与推送断言；需要时可返回完整快照。
    const key = (instanceId: string, channelId: string) => `${instanceId}/${channelId}`;

    /** 只发通知，不携带正文；kind 与后端 event_kind 一致。 */
    const notify = (channelId: string) => {
        const event = { sequence: (eventSequence += 1), kind: `loopback-data:${channelId}`, unixMillis: NOW_MS };
        parseComponentHostEvent(event); // 前端契约先校验，字段不符时测试立即失败。
        for (const listener of [...listeners]) listener(event);
    };

    const invoke = async (command: string, payload: Record<string, unknown> = {}) => {
        const instanceId = String(payload.instanceId ?? "");
        const channelId = String(payload.channelId ?? "");
        if (command === "register_loopback_channel") {
            registrations.push(payload);
            if (credentialMissing) throw { code: "loopback-credential-unavailable", message: "未找到凭据记录 DesktopTools/astrbot-status" };
            if (registerError) throw registerError;
            const itemKey = key(instanceId, channelId);
            if (activeChannels.has(itemKey)) {
                replaced += 1; // 宿主按 channelId 替换旧通道并清空旧缓冲。
                buffers.set(itemKey, []);
            }
            activeChannels.add(itemKey);
            return {
                channelId,
                mode: payload.mode,
                port: payload.mode === "listen" ? payload.port : null,
                baseUrl: payload.mode === "request" ? payload.baseUrl : null,
                registeredAt: nowSeconds,
            };
        }
        if (command === "read_loopback_channel") {
            reads.push({ instanceId, channelId, afterSequence: payload.afterSequence as number | undefined });
            subscribed.add(key(instanceId, channelId));
            const queued = buffers.get(key(instanceId, channelId)) ?? [];
            buffers.set(key(instanceId, channelId), []);
            const after = payload.afterSequence as number | undefined;
            return queued.filter((entry) => after === undefined || entry.sequence > after);
        }
        if (command === "request_loopback") {
            requests.push(payload);
            if (!tokenValid && payload.path === "/v1/snapshot") return { status: 401, headers: [], body: "" }; // 对端拒绝 token。
            return { status: 200, headers: [], body: pullBody }; // 拉取兜底默认不参与推送链路的断言。
        }
        throw new Error(`未知命令：${command}`);
    };

    const listen = async (_name: string, listener: (event: unknown) => void) => {
        listeners.push(listener);
        return () => {
            const index = listeners.indexOf(listener);
            if (index >= 0) listeners.splice(index, 1);
        };
    };

    /** 对端推一条原始正文；订阅尚未建立时只入缓冲。 */
    const push = (instanceId: string, channelId: string, body: string, receivedAt = nowSeconds) => {
        const itemKey = key(instanceId, channelId);
        const sequence = (counters.get(itemKey) ?? 0) + 1;
        counters.set(itemKey, sequence);
        buffers.set(itemKey, [...(buffers.get(itemKey) ?? []), { sequence, receivedAt, method: "POST", path: "/v1/snapshot", headers: [], body }]);
        if (subscribed.has(itemKey)) notify(channelId);
    };

    return {
        invoke,
        listen,
        push,
        registrations,
        reads,
        requests,
        listeners,
        get replaced() {
            return replaced;
        },
        setCredentialMissing: (value: boolean) => (credentialMissing = value),
        setRegisterError: (value: { code: string; message: string } | null) => (registerError = value),
        setTokenValid: (value: boolean) => (tokenValid = value),
        setPullBody: (value: string) => (pullBody = value),
    };
};

const settle = (milliseconds = 30) => new Promise((resolve) => setTimeout(resolve, milliseconds));

/** 渲染一个格子并返回容器与清理函数。 */
const render = (tileType: string, instanceId: string, menu?: unknown) => {
    const container = new FakeNode();
    const cleanup = plugin.default.renderTile(container, { instanceId, tileType, menu });
    return { container, cleanup };
};

/** 渲染一个带自定义外壳上下文的格子（设置变化、saveSettings 等）。 */
const renderWith = (tileType: string, instanceId: string, context: Record<string, unknown> = {}) => {
    const container = new FakeNode();
    const cleanup = plugin.default.renderTile(container, { instanceId, tileType, settings: {}, saveSettings: async () => undefined, ...context });
    return { container, cleanup };
};

/** 递归收集指定类名的节点。 */
const findByClass = (node: FakeNode, className: string): FakeNode[] => {
    const found: FakeNode[] = [];
    const walk = (current: FakeNode) => {
        if (current.className.split(" ").includes(className)) found.push(current);
        for (const child of current.children) walk(child);
    };
    walk(node);
    return found;
};

// 通道收到推送 → 入站缓冲 → 宿主事件 → 插件读原始正文 → 自行解析 → 六类格子显示。
test("宿主通知驱动六类格子从原始正文更新", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tiles = Object.keys(plugin.TILE_TYPES).map((type) => ({ type, ...render(type, `instance-${type}`) }));
    try {
        await settle();
        const registrations = host.registrations;
        assert.equal(registrations.length, 2, "六类格子共用一组通道");
        assert.deepEqual(
            {
                channelId: registrations[0].channelId,
                mode: registrations[0].mode,
                port: registrations[0].port,
                credentialRef: registrations[0].credentialRef,
                headerName: registrations[0].headerName,
            },
            { channelId: "astrbot-push", mode: "listen", port: 8765, credentialRef: "DesktopTools/astrbot-status", headerName: "X-Status-Token" },
        );
        assert.deepEqual(
            { channelId: registrations[1].channelId, mode: registrations[1].mode, baseUrl: registrations[1].baseUrl },
            { channelId: "astrbot-request", mode: "request", baseUrl: "http://127.0.0.1:8766" },
        );
        assert.equal(store.snapshot(), null, "缓存未命中时不伪造数据");

        host.push("instance-status", "astrbot-push", JSON.stringify(snapshotV2()));
        await settle(60);

        assert.equal(store.snapshot()?.collectedAt, nowSeconds - 3, "原始正文经插件自解析进入缓存");
        const expected: Record<string, string[]> = {
            status: ["在线", "已就绪 1 小时 0 分"],
            uptime: ["1 小时 0 分"],
            platforms: ["平台 3 / 4 在线", "QQ 主号", "Telegram", "错误 ×7"],
            resources: ["18%", "413 MB", "线程 62"],
            messages: ["今日 1,284"],
            logs: ["连接关闭", "发送超时", "间隔 5 秒"],
        };
        for (const { type, container } of tiles) {
            const text = container.text();
            for (const fragment of expected[type]) assert.ok(text.includes(fragment), `${type} 缺少 ${fragment}：${text}`);
            assert.equal(container.children[0].dataset.tileType, type);
        }
    } finally {
        for (const { cleanup } of tiles) cleanup();
        store.dispose();
        restore();
    }
});

// 同实例重复挂载只注册一次；30 秒接收判定与旧快照不回退都在事件链路里生效。
test("同一实例覆盖与两层过期判定", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const first = render("status", "instance-a");
    const second = render("status", "instance-a");
    try {
        await settle();
        assert.equal(host.registrations.length, 2, "同一实例的两个挂载点只注册一组通道");

        host.push("instance-a", "astrbot-push", JSON.stringify(snapshotV2({ collectedAt: nowSeconds - 31 })), nowSeconds); // 到得太晚的推送。
        await settle(60);
        assert.equal(store.snapshot(), null, "超过 30 秒才到的推送被丢弃");

        host.push("instance-a", "astrbot-push", JSON.stringify(snapshotV2()));
        await settle(60);
        assert.equal(store.snapshot()?.collectedAt, nowSeconds - 3);
        assert.ok(first.container.text().includes("在线"), "两个挂载点更新同一份缓存");
        assert.ok(second.container.text().includes("在线"));

        host.push("instance-a", "astrbot-push", JSON.stringify(snapshotV2({ collectedAt: nowSeconds - 10, messages: { total: 1, series: [] } })));
        await settle(60);
        assert.equal(store.snapshot()?.collectedAt, nowSeconds - 3, "更旧的快照不覆盖缓存");
        assert.ok(store.state().stale === false);
        assert.equal(store.state().receivedAt, nowSeconds, "缓存命中保留接收时刻");
    } finally {
        first.cleanup();
        second.cleanup();
        store.dispose();
        restore();
    }
});

// 退订后宿主不再投递、插件不再读取，界面保持最后一次渲染结果。
test("退订后不再更新", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = render("messages", "instance-a");
    try {
        await settle();
        host.push("instance-a", "astrbot-push", JSON.stringify(snapshotV2()));
        await settle(60);
        assert.ok(tile.container.text().includes("今日 1,284"));
        const readsBefore = host.reads.length;

        tile.cleanup(); // 最后一个挂载点退订后会释放宿主事件监听。
        assert.equal(host.listeners.length, 0, "退订后不再占用宿主事件监听");
        host.push("instance-a", "astrbot-push", JSON.stringify(snapshotV2({ collectedAt: nowSeconds, messages: { total: 99, series: [] } })));
        await settle(60);
        assert.equal(store.snapshot()?.messages.total, 1284, "退订后不再读取缓冲");
        assert.equal(host.reads.length, readsBefore, "退订后不再发起读取");
        assert.ok(tile.container.text().includes("今日 1,284"), "界面保持最后一次渲染结果");
    } finally {
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 过期保留最后有效内容，不加界面标记；只有状态格把在线状态压成未知灰。
test("过期不清空且只压灰状态点", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tiles = Object.keys(plugin.TILE_TYPES).map((type) => ({ type, ...render(type, `instance-${type}`) }));
    try {
        await settle();
        host.push("instance-status", "astrbot-push", JSON.stringify(snapshotV2({ collectedAt: nowSeconds - 61 })), nowSeconds - 61); // 61 秒前的快照按过期处理。
        await settle(60);
        const text = (type: string) => tiles.find((tile) => tile.type === type)?.container.text() ?? "";
        for (const tile of tiles) {
            assert.doesNotMatch(text(tile.type), /数据过期|暂无数据/, `${tile.type} 不应出现过期或空态标记：${text(tile.type)}`);
        }
        assert.ok(text("status").includes("在线"), "结论文字仍保留");
        assert.match(text("status"), /已就绪 1 小时 0 分/, "状态格保留最后快照的就绪表示");
        assert.ok(text("uptime").includes("1 小时 0 分"));
        assert.match(text("platforms"), /平台 3 \/ 4 在线/);
        assert.match(text("platforms"), /QQ 主号/);
        assert.match(text("resources"), /18%|413 MB/);
        assert.match(text("messages"), /今日 1,284/);
        assert.match(text("logs"), /连接关闭|发送超时/);
        assert.ok(findByClass(tiles[0].container, "astrbot-tile__dot--unknown").length > 0, "过期只把状态点压成未知灰");
    } finally {
        for (const tile of tiles) tile.cleanup();
        store.dispose();
        restore();
    }
});

// 刷新失败或空缓冲不写缓存，已有内容原地保留，不出现空态中间帧。
test("刷新失败不清空已有内容", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = render("messages", "instance-refresh");
    try {
        await settle();
        host.push("instance-refresh", "astrbot-push", JSON.stringify(snapshotV2({ messages: { total: 1284, series: [] } })));
        await settle(60);
        assert.ok(tile.container.text().includes("今日 1,284"));

        host.setPullBody("not-json"); // 拉取兜底返回坏数据，模拟刷新中途的失败分支。
        await store.refresh();
        await settle(60);
        assert.ok(tile.container.text().includes("今日 1,284"), "失败刷新不覆盖已有快照");
        assert.doesNotMatch(tile.container.text(), /暂无数据|数据过期/);

        host.push("instance-refresh", "astrbot-push", JSON.stringify(snapshotV2({ collectedAt: nowSeconds - 2, messages: { total: 99, series: [] } })));
        await settle(60);
        assert.ok(tile.container.text().includes("今日 99"), "成功刷新原地替换数值");
        assert.doesNotMatch(tile.container.text(), /暂无数据|数据过期/);
    } finally {
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 消息量固定 24 个槽位，缺失小时留空，最近数据靠「现在」一侧。
test("消息量固定 24 槽并右对齐", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = render("messages", "instance-messages");
    try {
        await settle();
        host.push("instance-messages", "astrbot-push", JSON.stringify(snapshotV2({ messages: { total: 1284, series: [{ ts: nowSeconds - 3600, count: 10 }, { ts: nowSeconds, count: 30 }] } })));
        await settle(60);
        const chart = findByClass(tile.container, "astrbot-tile__chart")[0];
        assert.ok(chart, "需要画出柱状容器");
        const columns = chart.children.filter((child) => child.className.split(" ").includes("astrbot-tile__bar-column"));
        assert.equal(columns.length, 24, "柱数固定 24");
        assert.equal(columns[23].children[0].hidden, false, "最近一小时在最后一格");
        assert.equal(columns[22].children[0].hidden, false, "上个小时紧挨着「现在」");
        assert.equal(columns[0].children[0].hidden, true, "缺失小时留空槽");
        assert.equal(columns.filter((column) => !column.children[0].hidden).length, 2);
    } finally {
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 凭据缺失进入统一未配对态：六类格子同一指引、不显示旧值、不重复注册；设置变化后重试。
test("未配对态给出设置指引并在配对变化后重试", async () => {
    const restore = installDom();
    const host = createHost();
    host.setCredentialMissing(true);
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tiles = Object.keys(plugin.TILE_TYPES).map((type) => ({ type, ...renderWith(type, `unpaired-${type}`) }));
    try {
        await settle();
        assert.equal(store.state().needsPairing, true);
        assert.equal(host.registrations.length, 1, "凭据缺失只尝试一次推送通道，不再注册第二条通道");
        for (const { type, container } of tiles) {
            const text = container.text();
            assert.ok(text.includes("未配对"), `${type} 缺少未配对提示：${text}`);
            assert.ok(text.includes("设置 → 功能格子") && text.includes("token"), `${type} 缺少配对指引：${text}`);
            assert.doesNotMatch(text, /在线|离线|探测被拒|今日 |18%|413 MB|连接关闭|平台 3/, `${type} 不应显示旧值或伪造数据：${text}`);
        }

        const readsBefore = host.reads.length;
        store.setVisible("unpaired-status", false);
        store.setVisible("unpaired-status", true);
        await settle();
        assert.equal(host.registrations.length, 1, "可见性刷新不重复注册");
        assert.equal(host.reads.length, readsBefore, "未配对时不读取缓冲");

        host.setCredentialMissing(false);
        const retried = renderWith("status", "unpaired-status", { settings: { pairingVersion: 1_001 } });
        await settle(60);
        assert.equal(store.state().needsPairing, false, "配对变化后自动重试并切换状态");
        assert.equal(host.registrations.length, 3, "重试时重新注册两条通道");
        assert.ok(!retried.container.text().includes("未配对"));
        retried.cleanup();
    } finally {
        for (const tile of tiles) tile.cleanup();
        store.dispose();
        restore();
    }
});

// 端口占用、地址非法、通道数上限等失败仍按普通空态处理，不混进未配对。
test("其它通道注册失败不进入未配对态", async () => {
    const restore = installDom();
    const host = createHost();
    host.setRegisterError({ code: "loopback-port-in-use", message: "本机端口 8765 已被占用" });
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = renderWith("status", "port-busy");
    try {
        await settle();
        assert.equal(store.state().needsPairing, false, "端口占用不是未配对");
        assert.equal(host.registrations.length, 2, "其它失败仍尝试两条通道");
        const text = tile.container.text();
        assert.ok(!text.includes("未配对"));
        assert.ok(text.includes("未知") && text.includes("采集插件未上报或未安装"), `普通空态保留：${text}`);
    } finally {
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 已配对状态改 token 也立即重注册（同名通道替换），并拉取一次判断是否 401。
test("已配对改 token 立即重注册并进入 token 无效态", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = renderWith("status", "reconnect-1");
    let invalid: ReturnType<typeof renderWith> | undefined;
    let fixed: ReturnType<typeof renderWith> | undefined;
    try {
        await settle();
        assert.equal(host.registrations.length, 2);
        assert.equal(host.listeners.length, 1);
        assert.equal(store.state().tokenInvalid, false);

        host.setTokenValid(false); // 用户保存了错误 token。
        invalid = renderWith("status", "reconnect-1", { settings: { pairingVersion: 2_001 } });
        await settle(80);
        assert.equal(host.registrations.length, 4, "改 token 后无条件重新注册两条通道");
        assert.equal(host.replaced, 2, "同名 channelId 由宿主替换，不报端口占用");
        assert.deepEqual(
            host.registrations.slice(2, 4).map((item) => item.channelId),
            host.registrations.slice(0, 2).map((item) => item.channelId),
            "重注册沿用同一 channelId",
        );
        assert.equal(host.listeners.length, 1, "旧宿主监听已释放，只保留一份");
        assert.equal(store.state().tokenInvalid, true);
        assert.equal(store.state().needsPairing, false);
        const invalidText = invalid.container.text();
        assert.ok(invalidText.includes("token 无效"), `缺少 token 无效提示：${invalidText}`);
        assert.ok(invalidText.includes("重新保存"));
        assert.ok(!invalidText.includes("未配对"), "token 无效不能混成未配对");

        host.setTokenValid(true); // 用户改正 token。
        fixed = renderWith("status", "reconnect-1", { settings: { pairingVersion: 2_002 } });
        await settle(80);
        assert.equal(host.registrations.length, 6, "改正后再次重新注册");
        assert.equal(store.state().tokenInvalid, false, "拉取成功后恢复正常态");
        assert.ok(!fixed.container.text().includes("token 无效"));
    } finally {
        fixed?.cleanup();
        invalid?.cleanup();
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 清除 token 后重新注册失败进入未配对态，且未配对时不再重复注册。
test("清除 token 进入未配对并停止重复注册", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = renderWith("status", "clear-1");
    let cleared: ReturnType<typeof renderWith> | undefined;
    try {
        await settle();
        assert.equal(host.registrations.length, 2);

        host.setCredentialMissing(true); // 用户清除了 token。
        cleared = renderWith("status", "clear-1", { settings: { pairingVersion: 3_001 } });
        await settle(80);
        assert.equal(store.state().needsPairing, true);
        assert.equal(store.state().tokenInvalid, false);
        assert.equal(host.registrations.length, 3, "凭据缺失只尝试一次推送通道");
        const text = cleared.container.text();
        assert.ok(text.includes("未配对"), `清除 token 后显示未配对：${text}`);
        assert.ok(text.includes("设置 → 功能格子"));
        assert.ok(!text.includes("token 无效"), "未配对与 token 无效必须分开");

        store.setVisible("clear-1", false);
        store.setVisible("clear-1", true);
        renderWith("status", "clear-1", { settings: { pairingVersion: 3_001 } }).cleanup();
        await settle();
        assert.equal(host.registrations.length, 3, "未配对时不再发多余注册请求");
    } finally {
        cleared?.cleanup();
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 单格重挂时序：宿主先 cleanup（清空 owner）再带新 pairingVersion 重挂，也要完成重注册与 token 校验。
test("单格 cleanup 后重挂仍校验 token", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    host.setPullBody(JSON.stringify(snapshotV2())); // 拉取兜底返回完整快照，模拟真实对端。
    const first = renderWith("status", "single-tile");
    let second: ReturnType<typeof renderWith> | undefined;
    try {
        await settle(60);
        assert.equal(host.registrations.length, 2);
        assert.equal(host.requests.length, 1, "首次挂载拉取一次有效快照");
        assert.ok(store.snapshot(), "有效快照已进入缓存");

        host.setTokenValid(false); // 用户改成了错误 token。
        first.cleanup(); // 宿主重挂前先执行上一轮清理，单格场景会清空 owner。
        second = renderWith("status", "single-tile", { settings: { pairingVersion: 4_001 } });
        await settle(120);
        assert.equal(host.registrations.length, 4, "重挂后重新注册两条通道");
        assert.equal(host.requests.length, 2, `注册完成后主动拉取一次校验 token：${JSON.stringify(host.requests)}`);
        assert.equal(store.state().tokenInvalid, true, "401 必须进入 token 无效态");
        assert.equal(store.state().needsPairing, false);
        assert.ok(second.container.text().includes("token 无效"), `缺少 token 无效文案：${second.container.text()}`);
    } finally {
        second?.cleanup();
        first.cleanup();
        store.dispose();
        restore();
    }
});

// 数据更新与状态切换复用同一 DOM 骨架，只改文字/属性/显隐，不重建整格或重挂监听。
test("数据更新与状态切换复用同一 DOM 骨架", async () => {
    const restore = installDom();
    const host = createHost();
    const store = plugin.createStatusStore({ invoke: host.invoke, listen: host.listen, now: () => NOW_MS });
    plugin.setStatusStore(store);
    const tile = renderWith("status", "stable-1");
    try {
        await settle(60);
        const root = tile.container.children[0];
        const body = root.children[0];
        const modes = [...body.children];
        const listeners = host.listeners.length;
        assert.equal(modes.length, 3, "未配对/token 无效/正常三态共用固定骨架");

        host.push("stable-1", "astrbot-push", JSON.stringify(snapshotV2()));
        await settle(80);
        assert.equal(tile.container.children[0], root, "数据更新不替换根节点");
        assert.equal(root.children[0], body, "数据更新不替换内容容器");
        for (let index = 0; index < modes.length; index += 1) assert.equal(body.children[index], modes[index], `状态区块 ${index} 保持同一节点`);
        assert.equal(host.listeners.length, listeners, "数据更新不重挂宿主监听");
        assert.ok(tile.container.text().includes("在线"));

        host.setTokenValid(false);
        store.retryChannels("stable-1"); // 同一挂载点上切换状态，不经过 renderTile。
        await settle(100);
        assert.equal(tile.container.children[0], root);
        assert.equal(root.children[0], body);
        for (let index = 0; index < modes.length; index += 1) assert.equal(body.children[index], modes[index], "状态切换不重建骨架");
        assert.equal(modes[2].hidden, true, "正常态区块隐藏而不是移除");
        assert.equal(modes[1].hidden, false, "token 无效区块复用原节点");
        assert.ok(tile.container.text().includes("token 无效"));
    } finally {
        tile.cleanup();
        store.dispose();
        restore();
    }
});

// 设置页保存/清除 token 后写入 pairingVersion，插件据 settings 变化重试，不新增宿主命令。
test("配对变化经插件设置通知格子重试", () => {
    const settingsPage = readFileSync(new URL("../settings/PluginSettingsPage.tsx", import.meta.url), "utf8");
    assert.match(settingsPage, /bumpPairingVersion\(pluginId\)/);
    assert.match(settingsPage, /pairingVersion: Math\.max\(Date\.now\(\)/);
    const pluginSource = readFileSync(new URL("../../../plugin/AstrBot 状态/index.js", import.meta.url), "utf8");
    assert.match(pluginSource, /settings\.pairingVersion/);
    assert.match(pluginSource, /retryChannels/);
});

// 插件只经宿主桥走 loopback 命令，不自己开端口、不直连 AstrBot。
test("插件没有自行开端口或直连原始地址的旁路", () => {
    const source = readFileSync(new URL("../../../plugin/AstrBot 状态/index.js", import.meta.url), "utf8");
    assert.doesNotMatch(source, /\bfetch\s*\(/);
    assert.doesNotMatch(source, /XMLHttpRequest|WebSocket|EventSource|createServer|net\.connect/);
    assert.doesNotMatch(source, /\.listen\(\s*8765/);
    assert.match(source, /port: PUSH_PORT/);
    assert.match(source, /baseUrl: REQUEST_BASE_URL/);
    for (const command of ["register_loopback_channel", "read_loopback_channel", "request_loopback"]) assert.match(source, new RegExp(command));
    assert.doesNotMatch(source, /probe-service-status|invoke_component_capability/);
});
