import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
    buildMenuItems,
    createStatusStore,
    formatAbsolute,
    formatBareDuration,
    formatDuration,
    isDisplayStale,
    isReceiveTooLate,
    logsView,
    messagesView,
    parseSnapshot,
    platformsView,
    resolveTileType,
    resourcesView,
    sampleSnapshot,
    statusView,
    summaryText,
    uptimeView,
    versionInfo,
} from "./index.js";

const NOW = new Date("2026-11-07T12:00:00+08:00").getTime();
const nowSeconds = Math.floor(NOW / 1000);

/** 生成一份契约 v2 快照。 */
const snapshotV2 = (overrides = {}) => ({
    schemaVersion: 2,
    collectedAt: nowSeconds - 3,
    instance: { id: "astrbot-1", astrbotVersion: "4.5.0" },
    service: {
        online: true,
        probe: { level: "ok", checkedAt: nowSeconds - 3 },
        startTime: nowSeconds - 3_600,
        versions: { astrbot: "4.5.0", webui: "1.2.3", code: "a1b2c3d" },
    },
    platforms: {
        summary: { total: 4, running: 3, error: 1 },
        items: [
            { id: "qq", displayName: "QQ 主号", status: "running", startedAt: nowSeconds - 7_200, errorCount: 0, lastError: null },
            { id: "tg", displayName: "Telegram", status: "error", startedAt: nowSeconds - 600, errorCount: 7, lastError: { message: "连接被重置", timestamp: nowSeconds - 60 } },
        ],
    },
    resources: { cpuPercent: 18.4, memory: { processMb: 412.6, systemMb: 15_320.2 }, threadCount: 62 },
    messages: { total: 1284, series: [{ ts: nowSeconds - 3_600, count: 10 }, { ts: nowSeconds, count: 30 }] },
    logs: {
        tail: [
            { ts: nowSeconds - 300, level: "INFO", text: "心跳" },
            { ts: nowSeconds - 120, level: "WARNING", text: "发送超时" },
            { ts: nowSeconds - 60, level: "ERROR", text: "连接关闭" },
        ],
    },
    ...overrides,
});

/** 读取插件源文件文本，用来确认没有保留旧探活路径。 */
const readSource = (name) => readFileSync(new URL(name, import.meta.url), "utf8");

// 清单只声明 loopback-channel，六类格子各自带默认与最小尺寸。
test("清单声明 loopback-channel 与六类类型的尺寸", () => {
    const manifest = readSource("./manifest.ini");
    assert.match(manifest, /required_capabilities = loopback-channel/);
    assert.doesNotMatch(manifest, /probe-service-status/);
    const tiles = ["status", "uptime", "platforms", "resources", "messages", "logs"];
    for (const key of tiles) assert.match(manifest, new RegExp(`^${key} = `, "m"));
    assert.match(manifest, /^status = 状态$/m, "状态类型显示名与设计文档一致，不再用插件名 AstrBot");
    assert.match(manifest, /^\[tiles\.size\]/m);
    assert.match(manifest, /^status = 168x114, 168x114$/m); // 状态格单档；默认与最小尺寸都在清单里。
    assert.match(manifest, /^messages = 420x300, 420x210$/m);
});

// 六类格子只走通道命令，不保留旧探活能力调用。
test("插件源不再调用探活能力，只调用 loopback 命令", () => {
    const source = readSource("./index.js");
    assert.doesNotMatch(source, /probe-service-status/);
    assert.doesNotMatch(source, /invoke_component_capability/);
    for (const command of ["register_loopback_channel", "read_loopback_channel", "request_loopback"]) assert.match(source, new RegExp(command));
    assert.match(source, /X-Status-Token/);
    assert.match(source, /credentialRef/);
    assert.doesNotMatch(source, /\bfetch\(/); // 不直连 AstrBot。
    assert.doesNotMatch(source, /WebSocket|EventSource/); // 不自行开连接。
    assert.doesNotMatch(source, /createServer|listen\(8765/); // 不自行开端口。
});

// v2 按 probe.level 解析；v1 视为没有 probe；未知版本整份拒绝。
test("自解析 v2、v1 与未知版本", () => {
    const v2 = parseSnapshot(snapshotV2());
    assert.equal(v2.schemaVersion, 2);
    assert.deepEqual(v2.service.probe, { level: "ok", checkedAt: nowSeconds - 3 });

    const v1 = parseSnapshot({ ...snapshotV2(), schemaVersion: 1 });
    assert.equal(v1.service.probe, null);
    assert.equal(statusView(v1, NOW).level, "unknown");

    const warnings = [];
    assert.equal(parseSnapshot({ ...snapshotV2(), schemaVersion: 9 }, (message) => warnings.push(message)), null);
    assert.equal(warnings.length, 1);
});

// 设置页版本：解析主动拉取得到的快照，缺失与过期都给「—」和原因。
test("设置页版本视图解析快照与过期", () => {
    const parsed = parseSnapshot(snapshotV2());
    assert.equal(parsed.instance.astrbotVersion, "4.5.0");
    assert.deepEqual(parsed.service.versions, { astrbot: "4.5.0", webui: "1.2.3", code: "a1b2c3d" });

    const fresh = versionInfo(parsed, NOW);
    assert.equal(fresh.astrbot.value, "4.5.0");
    assert.equal(fresh.webui.value, "1.2.3");
    assert.equal(fresh.code.value, "a1b2c3d");
    assert.equal(fresh.stale, false);

    const stale = versionInfo(parsed, NOW + 120_000);
    assert.equal(stale.stale, true);
    for (const field of [stale.astrbot, stale.webui, stale.code]) {
        assert.equal(field.value, "—");
        assert.equal(field.reason, "数据过期");
    }

    const missing = versionInfo(
        parseSnapshot({ ...snapshotV2(), instance: { astrbotVersion: null }, service: { ...snapshotV2().service, versions: { astrbot: null, webui: null, code: null } } }),
        NOW,
    );
    for (const field of [missing.astrbot, missing.webui, missing.code]) {
        assert.equal(field.value, "—");
        assert.equal(field.reason, "字段未上报");
    }
});

// 设置页版本只主动拉取一次，不读缓冲、不写设置；桌面格子不写设置。
test("设置页版本只主动拉取且桌面格子不写插件设置", () => {
    const source = readSource("./index.js");
    const start = source.indexOf("function renderVersionEntry");
    const body = source.slice(start, source.indexOf("export default"));
    assert.match(body, /requestLoopback/);
    assert.equal((body.match(/context\.requestLoopback\(/g) ?? []).length, 1, "一次打开最多一次主动请求");
    assert.doesNotMatch(body, /peek/i, "不再保留 peek 回退");
    assert.match(body, /REQUEST_CHANNEL/);
    assert.doesNotMatch(body, /read_loopback_channel/);
    assert.doesNotMatch(body, /saveSettings/);
    assert.doesNotMatch(source, /context\.saveSettings\s*\(/);
    assert.match(source, /VERSION_ENTRY_UNAVAILABLE = "需在桌面放置格子后可见"/);
});

// 版本字段只在设置页展示，绝不进入六类格子视图。
test("版本字段不进入任何格子视图", () => {
    const snapshot = parseSnapshot(snapshotV2());
    const views = JSON.stringify(
        [statusView, uptimeView, platformsView, resourcesView, messagesView, logsView].map((view) => view(snapshot, NOW)),
    ).concat(summaryText(snapshot, NOW));
    assert.doesNotMatch(views, /4\.5\.0|1\.2\.3|a1b2c3d/);
});

// 详情 JSON、非对象与非法正文都要安全失败。
test("结构化正文无法解析或缺少采集时刻时返回 null", () => {
    assert.equal(parseSnapshot("{"), null);
    assert.equal(parseSnapshot([]), null);
    assert.equal(parseSnapshot(null), null);
    assert.equal(parseSnapshot({ ...snapshotV2(), collectedAt: null }), null);
});

// 接收判定 30 秒、显示判定 60 秒，边界都按「超过」处理。
test("接收与显示两层过期判定", () => {
    assert.equal(isReceiveTooLate(nowSeconds, nowSeconds - 30), false);
    assert.equal(isReceiveTooLate(nowSeconds, nowSeconds - 31), true);
    assert.equal(isDisplayStale(nowSeconds, NOW), false);
    assert.equal(isDisplayStale(nowSeconds - 60, NOW), false);
    assert.equal(isDisplayStale(nowSeconds - 61, NOW), true);
});

// 四档语义以 probe.level 为准；未知不等于离线，过期只把在线状态压灰，其余内容保留。
test("四档状态、过期只压灰与版本缺失", () => {
    const ok = statusView(parseSnapshot(snapshotV2()), NOW);
    assert.equal(ok.label, "在线");
    assert.equal(ok.levelClass, "ok");
    assert.equal(ok.stale, false);

    const staleSnapshot = parseSnapshot({ ...snapshotV2(), collectedAt: nowSeconds - 61 });
    const stale = statusView(staleSnapshot, NOW);
    assert.equal(stale.level, "ok");
    assert.equal(stale.stale, true);
    assert.equal(stale.levelClass, "unknown", "过期的状态点不再显示在线绿");
    assert.equal(stale.readyText, "已就绪 1 小时 0 分", "过期不改变结论行以外的内容");
    assert.doesNotMatch(stale.dotTitle, /过期/);
    assert.doesNotMatch(stale.readyTitle, /过期/);

    for (const level of ["off", "warn", "unknown"]) {
        const view = statusView(parseSnapshot(snapshotV2({ service: { online: null, probe: { level, checkedAt: nowSeconds }, startTime: null } })), NOW);
        assert.equal(view.level, level);
        assert.notEqual(view.levelClass, "ok");
    }
    const missing = statusView(parseSnapshot({ ...snapshotV2(), schemaVersion: 1 }), NOW);
    assert.equal(missing.label, "未知");
    assert.doesNotMatch(missing.label, /离线/);
});

// 就绪时长用 startTime 现算，不用任何 uptime 字段。
test("就绪时长按 startTime 现算", () => {
    assert.equal(formatDuration(nowSeconds, NOW), "刚刚就绪");
    assert.equal(formatDuration(nowSeconds - 59, NOW), "刚刚就绪");
    assert.equal(formatDuration(nowSeconds - 60, NOW), "已就绪 1 分");
    assert.equal(formatDuration(nowSeconds - 3_600, NOW), "已就绪 1 小时 0 分");
    assert.equal(formatDuration(nowSeconds - 90_000, NOW), "已就绪 1 天 1 小时");
    assert.equal(formatDuration(nowSeconds + 5, NOW), "刚刚就绪");
    assert.equal(formatBareDuration(nowSeconds - 3_600, NOW), "1 小时 0 分");
    assert.equal(formatBareDuration(nowSeconds, NOW), "刚刚");
    assert.equal(formatDuration(null, NOW), "—");

    const view = uptimeView(parseSnapshot(snapshotV2()), NOW);
    assert.equal(view.text, "1 小时 0 分");
    assert.match(view.title, /就绪于/);
    assert.equal(uptimeView(parseSnapshot({ ...snapshotV2(), service: { probe: null, startTime: null } }), NOW).text, "—");
});

// 平台清单整段缺失、空数组与正常清单分别呈现，不把空数组当成缺失。
test("平台连接的空态矩阵与逐行信息", () => {
    const normal = platformsView(parseSnapshot(snapshotV2()), NOW);
    assert.equal(normal.summary, "平台 3 / 4 在线");
    assert.equal(normal.rows.length, 2);
    assert.match(normal.rows[1].error, /错误 ×7/);
    assert.match(normal.rows[1].title, /连接被重置/);

    const missing = platformsView(parseSnapshot({ ...snapshotV2(), platforms: null }), NOW);
    assert.equal(missing.summary, "平台清单暂不可用");
    assert.equal(missing.empty, false, "收到过快照后不再整格空态");
    assert.equal(missing.note, "平台数据未上报");

    const empty = platformsView(parseSnapshot({ ...snapshotV2(), platforms: { summary: { total: 0, running: 0, error: 0 }, items: [] } }), NOW);
    assert.equal(empty.note, "暂无平台");
    assert.equal(empty.empty, false);

    const noSnapshot = platformsView(null, NOW);
    assert.equal(noSnapshot.summary, "平台清单暂不可用");
    assert.equal(noSnapshot.note, "暂无数据");
});

// 资源空字段显示「—」而不是 0；消息量为空数组时不画柱；日志只留异常节点并倒序。
test("资源、消息与日志的字段映射", () => {
    const resources = resourcesView(parseSnapshot(snapshotV2()), NOW);
    assert.equal(resources.cpuText, "18%");
    assert.equal(resources.memoryText, "413 MB");
    assert.equal(resources.threadsText, "线程 62");
    assert.ok(resources.cpuWidth > 0 && resources.memoryWidth > 0);

    const firstSample = resourcesView(parseSnapshot({ ...snapshotV2(), resources: { cpuPercent: null, memory: null, threadCount: null } }), NOW);
    assert.equal(firstSample.cpuText, "—");
    assert.equal(firstSample.memoryText, "—");
    assert.equal(firstSample.threadsText, null);

    const messages = messagesView(parseSnapshot(snapshotV2()), NOW);
    assert.equal(messages.totalText, "今日 1,284");
    assert.equal(messages.bars.length, 24); // 固定 24 个槽位，不随数据点数量伸缩。
    assert.equal(messages.bars.filter((bar) => !bar.empty).length, 2);
    assert.equal(messages.bars[23].height, 100); // 最近一小时靠「现在」一侧。
    assert.equal(messages.bars[22].height, 33); // 上个小时紧挨着它。
    assert.equal(messages.bars[0].empty, true); // 缺失小时留空槽。
    assert.equal(messages.bars[0].height, 0); // 空桶高度为 0，不画柱。
    assert.equal(messagesView(parseSnapshot({ ...snapshotV2(), messages: { total: null, series: [] } }), NOW).emptyChart, true);

    const logs = logsView(parseSnapshot(snapshotV2()), NOW);
    assert.deepEqual(logs.nodes.map((node) => node.level), ["ERROR", "WARNING"]);
    assert.equal(logs.nodes[0].text, "连接关闭");
    assert.match(logs.updatedText, /间隔 5 秒/);

    const staleSnapshot = parseSnapshot({ ...snapshotV2(), collectedAt: nowSeconds - 61 });
    assert.equal(uptimeView(staleSnapshot, NOW).text, "1 小时 0 分", "过期保留最后有效快照");
    assert.equal(platformsView(staleSnapshot, NOW).rows.length, 2);
    assert.equal(resourcesView(staleSnapshot, NOW).cpuText, "18%");
    assert.equal(messagesView(staleSnapshot, NOW).totalText, "今日 1,284");
    const staleLogs = logsView(staleSnapshot, NOW);
    assert.equal(staleLogs.nodes.length, 2);
    assert.match(staleLogs.updatedText, /间隔 5 秒/);
    assert.doesNotMatch(staleLogs.updatedText, /过期/);
});

// 收到过快照但分段缺失时用分段原因，不冒充从未收到数据的整格空态。
test("分段缺失不显示整格暂无数据", () => {
    const snapshot = parseSnapshot({ ...snapshotV2(), resources: null, messages: null, logs: null });
    assert.equal(resourcesView(snapshot, NOW).empty, false);
    assert.equal(messagesView(snapshot, NOW).empty, false);
    assert.equal(messagesView(snapshot, NOW).totalText, "消息数据未上报");
    assert.equal(logsView(snapshot, NOW).empty, false);
    assert.equal(logsView(snapshot, NOW).updatedText, "日志数据未上报");
    assert.equal(platformsView(snapshot, NOW).empty, false);
});

// 尺寸只来自 manifest；插件不再按外框尺寸切档。
test("不再按尺寸切换布局", () => {
    const source = readSource("./index.js");
    assert.doesNotMatch(source, /tierFor|data-tier|clientHeight|clientWidth|ResizeObserver/);
    const manifest = readSource("./manifest.ini");
    assert.match(manifest, /\[tiles\.size\]/);
    for (const type of ["status", "uptime", "platforms", "resources", "messages", "logs"]) {
        assert.match(manifest, new RegExp(`^${type} = \\d+x\\d+, \\d+x\\d+$`, "m"), `${type} 的默认与最小尺寸都由 manifest 声明`);
    }
});

// 类型缺失或未知时回落到状态类型。
test("类型回落", () => {
    assert.equal(resolveTileType(""), "status");
    assert.equal(resolveTileType("unknown"), "status");
    assert.equal(resolveTileType("logs"), "logs");
});

// 插件菜单与外壳「折叠」等通用项分组；写操作只在平台连接格且存在出错平台时出现。
test("右键菜单分组与写操作门控", () => {
    const base = buildMenuItems({ tileType: "status", snapshot: parseSnapshot(snapshotV2()), nowMs: NOW });
    assert.deepEqual(base.map((item) => item.id), ["open-console", "refresh", "copy-summary"]);
    assert.equal(base[0].separatorBefore, true, "第一项前分组，与外壳折叠项隔开");
    assert.equal(base.slice(1).some((item) => item.separatorBefore), false);

    const platforms = buildMenuItems({ tileType: "platforms", snapshot: parseSnapshot(snapshotV2()), nowMs: NOW });
    const write = platforms.filter((item) => item.id.startsWith("toggle-platform:"));
    assert.equal(write.length, 1);
    assert.equal(write[0].label, "启动 Telegram");
    assert.equal(write[0].separatorBefore, true);

    const healthy = buildMenuItems({
        tileType: "platforms",
        snapshot: parseSnapshot({ ...snapshotV2(), platforms: { summary: { total: 1, running: 1, error: 0 }, items: [snapshotV2().platforms.items[0]] } }),
        nowMs: NOW,
    });
    assert.equal(healthy.filter((item) => item.id.startsWith("toggle-platform:")).length, 0);

    const busy = buildMenuItems({ tileType: "status", snapshot: null, busy: true, nowMs: NOW });
    assert.equal(busy.find((item) => item.id === "refresh").disabled, true);
});

// 摘要文案供复制菜单使用；过期不追加界面标记。
test("状态摘要", () => {
    const text = summaryText(parseSnapshot(snapshotV2()), NOW);
    assert.match(text, /AstrBot 在线/);
    assert.match(text, /1 小时 0 分/);
    assert.doesNotMatch(text, /版本/);
    const stale = summaryText(parseSnapshot({ ...snapshotV2(), collectedAt: nowSeconds - 61 }), NOW);
    assert.match(stale, /AstrBot 在线/);
    assert.doesNotMatch(stale, /数据过期/);
});

// 样例数据必须能通过同一份解析器，供宿主桥缺席时预览。
test("样例快照通过解析", () => {
    const parsed = parseSnapshot(sampleSnapshot(NOW));
    assert.ok(parsed);
    assert.equal(statusView(parsed, NOW).label, "在线");
    assert.equal(messagesView(parsed, NOW).bars.length, 24);
});

/** 假宿主桥：记录调用，按队列返回读取结果与拉取正文。 */
const createFakeBridge = () => {
    const calls = [];
    const eventHandlers = [];
    const reads = [];
    let pullBody = snapshotV2();
    let failReadFor = null;
    const invoke = (command, payload) => {
        calls.push({ command, payload });
        if (command === "read_loopback_channel" && payload.instanceId === failReadFor) return Promise.reject({ code: "loopback-channel-not-found" });
        if (command === "register_loopback_channel") {
            return Promise.resolve({
                channelId: payload.channelId,
                mode: payload.mode,
                port: payload.mode === "listen" ? payload.port : null,
                baseUrl: payload.mode === "request" ? payload.baseUrl : null,
                registeredAt: nowSeconds,
            });
        }
        if (command === "read_loopback_channel") return Promise.resolve(reads.shift() ?? []);
        if (command === "request_loopback") return Promise.resolve({ status: 200, headers: [], body: JSON.stringify(pullBody) });
        return Promise.reject(new Error(`未知命令：${command}`));
    };
    const listen = (name, handler) => {
        calls.push({ command: "listen", payload: name });
        eventHandlers.push(handler);
        return Promise.resolve(() => undefined);
    };
    return { calls, eventHandlers, reads, invoke, listen, setPullBody: (value) => (pullBody = value), setFailReadFor: (value) => (failReadFor = value) };
};

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));
const flush = () => new Promise((resolve) => setTimeout(resolve, 5));

// 第一次挂载注册两条通道（8765 listen / 8766 request，X-Status-Token）；重复实例不重复注册。
test("通道注册参数与按实例去重", async () => {
    const bridge = createFakeBridge();
    const store = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detachA1 = store.attach("instance-a", () => {});
    const detachA2 = store.attach("instance-a", () => {});
    const detachB = store.attach("instance-b", () => {});
    await settle();

    const registrations = bridge.calls.filter((call) => call.command === "register_loopback_channel");
    assert.equal(registrations.length, 2);
    const [push, request] = registrations.map((call) => call.payload);
    assert.deepEqual(
        { channelId: push.channelId, mode: push.mode, port: push.port, credentialRef: push.credentialRef, headerName: push.headerName },
        { channelId: "astrbot-push", mode: "listen", port: 8765, credentialRef: "DesktopTools/astrbot-status", headerName: "X-Status-Token" },
    );
    assert.equal(request.channelId, "astrbot-request");
    assert.equal(request.mode, "request");
    assert.equal(request.baseUrl, "http://127.0.0.1:8766");
    assert.equal(request.headerName, "X-Status-Token");

    const reads = bridge.calls.filter((call) => call.command === "read_loopback_channel");
    assert.ok(reads.every((call) => call.payload.instanceId === "instance-a"));
    assert.equal(bridge.calls.filter((call) => call.command === "listen").length, 1);

    detachA1();
    detachA2();
    detachB();
    store.dispose();
});

// 原持有实例释放后由仍在屏幕上的实例接手通道，而不是继续读已释放的句柄。
test("通道随实例释放后由其它实例接手", async () => {
    const bridge = createFakeBridge();
    bridge.reads.push([]);
    const store = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detachA = store.attach("instance-a", () => {});
    const detachB = store.attach("instance-b", () => {});
    await settle();

    bridge.setFailReadFor("instance-a");
    bridge.eventHandlers[0]({ sequence: 1, kind: "loopback-data:astrbot-push", unixMillis: NOW });
    await settle();

    const registrations = bridge.calls
        .filter((call) => call.command === "register_loopback_channel")
        .map((call) => call.payload.instanceId);
    assert.equal(registrations.at(-1), "instance-b");
    assert.equal(bridge.calls.filter((call) => call.command === "listen").length, 2, "接手时重新监听宿主事件");
    detachA();
    detachB();
    store.dispose();
});

// 通知只带通道标识；插件回来读缓冲，30 秒后才到的推送被丢弃。
test("宿主通知触发读取，超时推送不进缓存", async () => {
    const bridge = createFakeBridge();
    bridge.reads.push([]);
    const store = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detach = store.attach("instance-a", () => {});
    await settle();

    bridge.reads.push([
        { sequence: 1, receivedAt: nowSeconds, method: "POST", path: "/v1/snapshot", headers: [], body: JSON.stringify(snapshotV2()) },
        { sequence: 2, receivedAt: nowSeconds, method: "POST", path: "/v1/snapshot", headers: [], body: JSON.stringify(snapshotV2({ collectedAt: nowSeconds - 31 })) },
    ]);
    bridge.eventHandlers[0]({ sequence: 1, kind: "loopback-data:astrbot-push", unixMillis: NOW });
    await settle();

    assert.ok(store.snapshot());
    assert.equal(store.snapshot().collectedAt, nowSeconds - 3);
    const read = bridge.calls.filter((call) => call.command === "read_loopback_channel").at(-1);
    assert.equal(read.payload.afterSequence, 0);
    assert.equal(read.payload.channelId, "astrbot-push");
    detach();
    store.dispose();
});

// 没有推送时经 8766 拉一次快照；恢复可见时也会补拉。
test("拉取兜底", async () => {
    const bridge = createFakeBridge();
    bridge.reads.push([]);
    bridge.setPullBody({ ...snapshotV2(), collectedAt: nowSeconds - 61 }); // 第一次拉到的快照已过期，恢复可见时必须再拉。
    const store = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detach = store.attach("instance-a", () => {});
    await settle();
    await settle();

    const pulls = bridge.calls.filter((call) => call.command === "request_loopback");
    assert.ok(pulls.length >= 1);
    assert.equal(pulls[0].payload.method, "GET");
    assert.equal(pulls[0].payload.path, "/v1/snapshot");
    assert.ok(store.snapshot());

    store.setVisible("instance-a", false);
    store.setVisible("instance-a", true);
    await settle();
    assert.ok(bridge.calls.filter((call) => call.command === "request_loopback").length >= 2);
    detach();
    store.dispose();
});

// 平台启停走 POST /v1/actions，成功后重读快照；失败不回滚也不抛到界面外。
test("平台启停请求与失败路径", async () => {
    const bridge = createFakeBridge();
    bridge.reads.push([]);
    const store = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detach = store.attach("instance-a", () => {});
    await settle();

    const platform = { id: "tg", name: "Telegram", status: "error" };
    const ok = await store.togglePlatform(platform);
    assert.equal(ok.ok, true);
    const action = bridge.calls.find((call) => call.command === "request_loopback" && call.payload.method === "POST");
    assert.equal(action.payload.path, "/v1/actions");
    assert.deepEqual(JSON.parse(action.payload.body), { action: "set_enabled", id: "tg", enabled: true });

    bridge.setPullBody({ ...snapshotV2(), service: { probe: { level: "warn", checkedAt: nowSeconds }, startTime: null } });
    bridge.invoke = (command, payload) => {
        bridge.calls.push({ command, payload });
        if (command === "request_loopback") return Promise.resolve({ status: 409, headers: [], body: JSON.stringify({ ok: false, reason: "unsupported_platform" }) });
        return Promise.resolve([]);
    };
    const storeTwo = createStatusStore({ invoke: bridge.invoke, listen: bridge.listen, now: () => NOW });
    const detachTwo = storeTwo.attach("instance-c", () => {});
    await settle();
    const failed = await storeTwo.togglePlatform(platform);
    assert.equal(failed.ok, false);
    assert.match(failed.error, /unsupported_platform/);

    detach();
    detachTwo();
    store.dispose();
    storeTwo.dispose();
});

// 时间格式化工具本身的边界。
test("时间格式化", () => {
    assert.equal(formatAbsolute(0), "—");
    assert.match(formatAbsolute(nowSeconds), /\d{4}/);
    assert.deepEqual(
        [statusView(null, NOW).label, statusView(null, NOW).empty],
        ["未知", true],
    );
});
