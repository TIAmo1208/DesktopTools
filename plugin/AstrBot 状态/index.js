/**
 * AstrBot 状态
 * 1. 清单只声明 loopback-channel：一条 listen 通道接 8765 推送，一条 request 通道走 8766 拉取兜底与启停平台
 * 2. 快照解析、模块级单例缓存、按实例去重、30 秒接收判定与 60 秒显示过期全在插件内
 * 3. 六类格子共用一份缓存；每个类型只决定渲染内容，不各开通道
 * 4. 版本字段不进入任何格子
 */

const PLUGIN_ID = "plugin.astrbot-status"; // 清单标识，同时作为样式表键。
const PUSH_CHANNEL = "astrbot-push"; // 接收 8765 推送的通道标识。
const REQUEST_CHANNEL = "astrbot-request"; // 向 8766 拉取与写操作的通道标识。
const PUSH_PORT = 8765; // AstrBot 采集插件的推送端口。
const REQUEST_BASE_URL = "http://127.0.0.1:8766"; // AstrBot 采集插件的反向端口。
const CREDENTIAL_REF = "DesktopTools/astrbot-status"; // Windows Credential Manager 里的本机密钥目标。
const TOKEN_HEADER = "X-Status-Token"; // 双向共用的鉴权头名。
const CONSOLE_URL = "http://127.0.0.1:6185"; // AstrBot WebUI 本机地址。
const DISPLAY_STALE_MS = 60_000; // 显示过期：最近一次快照超过该时长。
const RECEIVE_TOO_LATE_MS = 30_000; // 接收判定：到得太晚的推送不进缓存。
const TICK_MS = 1_000; // 就绪时长与过期文案每秒重算。
const FALLBACK_PULL_MS = 20_000; // 推送不可用或数据过期时的拉取兜底间隔。
const SAMPLE_INTERVAL_SEC = 5; // 契约约定的采集间隔，用于「间隔 5 秒」文案。
const PROBE_LEVELS = new Set(["ok", "warn", "off", "unknown"]); // 快照 v2 的四档结论。
const PLATFORM_ROWS = 6; // 平台连接单套布局最多显示的平台行数，其余折成「还有 N 个」。
const LOG_ROWS = 5; // 运行日志单套布局最多显示的时间轴节点数。

/** 六类格子的稳定键，必须与 manifest.ini 的 [tiles] 一致。 */
export const TILE_TYPES = {
    status: "状态",
    uptime: "就绪时长",
    platforms: "平台连接",
    resources: "资源占用",
    messages: "消息量",
    logs: "运行日志",
};

/** 四档结论的文案与状态点语义。未知不等于离线，探测被拒不等于服务停止。 */
export const LEVEL_VIEW = {
    ok: { label: "在线", className: "ok" },
    off: { label: "离线", className: "off" },
    warn: { label: "探测被拒", className: "warn" },
    unknown: { label: "未知", className: "unknown" },
};

const styleUrl = new URL("./style.css", import.meta.url).href; // 同级样式表资产。
// 纯逻辑测试在 Node 里直接导入本模块，那里没有 document。
if (typeof document !== "undefined" && !document.querySelector(`link[data-plugin-style="${PLUGIN_ID}"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", PLUGIN_ID);
    document.head.appendChild(link);
}

/** 未知类型回落到状态类型，不假设宿主一定给了合法值。 */
export const resolveTileType = (tileType) => (typeof tileType === "string" && tileType in TILE_TYPES ? tileType : "status");

/** 有限数字，否则 null。 */
const finiteOrNull = (value) => (typeof value === "number" && Number.isFinite(value) ? value : null);

/** 非负整数，否则 null。 */
const intOrNull = (value) => (Number.isSafeInteger(value) && value >= 0 ? value : null);

/** 短文本，超长截断到契约上限。 */
const textOrNull = (value, limit = 500) => (typeof value === "string" ? value.slice(0, limit) : null);

/** 本地时钟文案，用于悬停与日志时间列。 */
export function formatClock(seconds) {
    if (!Number.isFinite(seconds) || seconds <= 0) return "—";
    return new Date(seconds * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
}

/** 本地绝对时间文案，用于悬停完整时间。 */
export function formatAbsolute(seconds) {
    if (!Number.isFinite(seconds) || seconds <= 0) return "—";
    return new Date(seconds * 1000).toLocaleString("zh-CN", { hour12: false });
}

/** 千分位数字；空值给「—」。 */
export function formatNumber(value) {
    return Number.isFinite(value) ? value.toLocaleString("zh-CN") : "—";
}

/**
 * 就绪时长：startTime 是核心加载完成时刻，不是进程启动时刻。
 * 文案只说「已就绪」，clock 回拨时压到 0。
 */
export function formatDuration(startTime, nowMs) {
    if (!Number.isFinite(startTime) || startTime <= 0) return "—";
    const seconds = Math.max(0, Math.floor((nowMs - startTime * 1000) / 1000));
    if (seconds < 60) return "刚刚就绪";
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `已就绪 ${minutes} 分`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `已就绪 ${hours} 小时 ${minutes % 60} 分`;
    return `已就绪 ${Math.floor(hours / 24)} 天 ${hours % 24} 小时`;
}

/** 状态与就绪时长格子里的大号时长：不带「已就绪」前缀。 */
export function formatBareDuration(startTime, nowMs) {
    const text = formatDuration(startTime, nowMs);
    return text === "刚刚就绪" ? "刚刚" : text.replace(/^已就绪 /, "");
}

/** 显示过期：最近一次有效快照的 collectedAt 距今超过 60 秒。 */
export function isDisplayStale(collectedAt, nowMs) {
    return Number.isFinite(collectedAt) && collectedAt > 0 ? nowMs - collectedAt * 1000 > DISPLAY_STALE_MS : false;
}

/** 接收判定：推送到达时刻比采集时刻晚超过 30 秒，当作坏数据丢掉。 */
export function isReceiveTooLate(receivedAt, collectedAt) {
    return Number.isFinite(receivedAt) && Number.isFinite(collectedAt) && receivedAt - collectedAt > RECEIVE_TOO_LATE_MS / 1000;
}

/** 解析快照 v2 的 service.probe；v1 或缺失一律给 null（按未知展示）。 */
const parseProbe = (value) => {
    if (!value || typeof value !== "object") return null;
    if (!PROBE_LEVELS.has(value.level)) return null;
    const checkedAt = intOrNull(value.checkedAt); // 判定时刻。
    return checkedAt === null ? null : { level: value.level, checkedAt };
};

/** 平台状态只认四种；未识别的映射为 unknown，不自造新值。 */
const parsePlatformStatus = (value) =>
    value === "running" || value === "error" || value === "stopped" || value === "unknown" ? value : value == null ? null : "unknown";

/** 归一化版本段：字段缺失给 null，不用空串冒充。 */
const parseVersions = (value) => {
    if (!value || typeof value !== "object") return null;
    return { astrbot: textOrNull(value.astrbot), webui: textOrNull(value.webui), code: textOrNull(value.code) };
};

/** 归一化平台清单；整段 null 与空数组保持不同语义。 */
const parsePlatforms = (value) => {
    if (!value || typeof value !== "object") return null;
    const summary = value.summary && typeof value.summary === "object"
        ? { total: intOrNull(value.summary.total), running: intOrNull(value.summary.running), error: intOrNull(value.summary.error) }
        : null;
    const items = Array.isArray(value.items)
        ? value.items.map((item) => {
              const lastError = item && item.lastError && typeof item.lastError === "object"
                  ? { message: textOrNull(item.lastError.message) ?? "", timestamp: intOrNull(item.lastError.timestamp) }
                  : null;
              return {
                  id: typeof item?.id === "string" ? item.id : null,
                  name: textOrNull(item?.displayName) ?? textOrNull(item?.type) ?? "未命名平台",
                  status: parsePlatformStatus(item?.status),
                  startedAt: intOrNull(item?.startedAt),
                  errorCount: intOrNull(item?.errorCount),
                  lastError,
              };
          })
        : null;
    return { summary, items };
};

/** 归一化资源段。 */
const parseResources = (value) => {
    if (!value || typeof value !== "object") return null;
    const memory = value.memory && typeof value.memory === "object"
        ? { processMb: finiteOrNull(value.memory.processMb), systemMb: finiteOrNull(value.memory.systemMb) }
        : null;
    return { cpuPercent: finiteOrNull(value.cpuPercent), memory, threadCount: intOrNull(value.threadCount) };
};

/** 归一化消息段：series 只保留最近 24 个有效点并按时间正序。 */
const parseMessages = (value) => {
    if (!value || typeof value !== "object") return null;
    const series = Array.isArray(value.series)
        ? value.series
              .map((point) => ({ ts: intOrNull(point?.ts), count: intOrNull(point?.count) }))
              .filter((point) => point.ts !== null && point.count !== null)
              .sort((left, right) => left.ts - right.ts)
              .slice(-24)
        : null;
    return { total: intOrNull(value.total), series };
};

/** 归一化日志段：只保留契约允许的级别，正文截断到 500 字。 */
const parseLogs = (value) => {
    if (!value || typeof value !== "object") return null;
    const tail = Array.isArray(value.tail)
        ? value.tail
              .filter((item) => item && typeof item === "object")
              .map((item) => ({ ts: intOrNull(item.ts), level: typeof item.level === "string" ? item.level.toUpperCase() : null, text: textOrNull(item.text) }))
              .filter((item) => item.ts !== null && item.text !== null && ["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"].includes(item.level))
              .slice(-200)
        : null;
    return { tail };
};

/**
 * 自解析快照。schemaVersion 1 视为没有 probe（未知），2 按 probe.level；
 * 其它版本整份按无效处理，由调用方按未知展示并记一次告警。
 */
export function parseSnapshot(raw, onWarn = () => {}) {
    let value = raw;
    if (typeof value === "string") {
        try {
            value = JSON.parse(value);
        } catch {
            onWarn("快照正文不是合法 JSON");
            return null;
        }
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) {
        onWarn("快照不是对象");
        return null;
    }
    const version = value.schemaVersion;
    if (version !== 1 && version !== 2) {
        onWarn(`未识别的快照版本：${String(version)}`);
        return null;
    }
    const collectedAt = intOrNull(value.collectedAt);
    if (collectedAt === null) {
        onWarn("快照缺少采集时刻，无法判定新鲜度");
        return null; // 契约要求 collectedAt 必填；缺了就不许进入缓存。
    }
    const instance = value.instance && typeof value.instance === "object" ? { astrbotVersion: textOrNull(value.instance.astrbotVersion) } : null;
    const service = value.service && typeof value.service === "object"
        ? {
              probe: version >= 2 ? parseProbe(value.service.probe) : null,
              startTime: intOrNull(value.service.startTime),
              versions: parseVersions(value.service.versions),
          }
        : { probe: null, startTime: null, versions: null };
    return {
        schemaVersion: version,
        collectedAt,
        instance,
        service,
        platforms: parsePlatforms(value.platforms),
        resources: parseResources(value.resources),
        messages: parseMessages(value.messages),
        logs: parseLogs(value.logs),
    };
}

/** 固定样例：浏览器预览或宿主桥缺席时驱动六类格子渲染，真实数据一到即可替换。 */
export function sampleSnapshot(nowMs) {
    const now = Math.floor(nowMs / 1000);
    return {
        schemaVersion: 2,
        collectedAt: now - 3,
        service: { probe: { level: "ok", checkedAt: now - 3 }, startTime: now - 3 * 3600 - 25 * 60 },
        platforms: {
            summary: { total: 4, running: 3, error: 1 },
            items: [
                { id: "aiocqhttp", name: "QQ 主号", status: "running", startedAt: now - 86_400, errorCount: 0, lastError: null },
                { id: "telegram", name: "Telegram", status: "error", startedAt: now - 7_200, errorCount: 7, lastError: { message: "连接被对端重置：Connection reset by peer", timestamp: now - 600 } },
                { id: "discord", name: "Discord", status: "running", startedAt: now - 1_800, errorCount: 0, lastError: null },
                { id: "webchat", name: "WebChat", status: "stopped", startedAt: null, errorCount: 0, lastError: null },
            ],
        },
        resources: { cpuPercent: 18.4, memory: { processMb: 412.6, systemMb: 15_320.2 }, threadCount: 62 },
        messages: {
            total: 1284,
            series: Array.from({ length: 24 }, (_, index) => ({ ts: now - (23 - index) * 3600, count: Math.round(20 + 40 * Math.abs(Math.sin(index / 2.4))) })),
        },
        logs: {
            tail: [
                { ts: now - 300, level: "INFO", text: "心跳正常" },
                { ts: now - 240, level: "WARNING", text: "Telegram 平台消息发送超时：retry scheduled" },
                { ts: now - 120, level: "ERROR", text: "Traceback (most recent call last): ConnectionResetError: [WinError 10054] 远程主机强迫关闭了一个现有的连接。" },
            ],
        },
    };
}

/** 状态类型：四档结论与就绪时长；过期只把在线状态压成未知灰。 */
export function statusView(snapshot, nowMs) {
    const probe = snapshot?.service?.probe ?? null; // v1 或缺失时按未知。
    const level = probe && PROBE_LEVELS.has(probe.level) ? probe.level : "unknown";
    const view = LEVEL_VIEW[level];
    const stale = isDisplayStale(snapshot?.collectedAt, nowMs);
    const startTime = snapshot?.service?.startTime ?? null;
    return {
        level,
        label: view.label,
        levelClass: stale ? "unknown" : view.className, // 过期只把在线状态压成未知灰，其余内容保持。
        stale,
        empty: !snapshot,
        readyText: level === "ok" ? formatDuration(startTime, nowMs) : "—",
        dotTitle: probe ? `判定于 ${formatAbsolute(probe.checkedAt)} · probe.level=${level}` : "暂无服务判定：没有收到带 probe 的快照",
        readyTitle: startTime ? `就绪于 ${formatAbsolute(startTime)}` : "暂无就绪时刻",
    };
}

/** 就绪时长类型：一行大号时长；过期沿用最后一次快照。 */
export function uptimeView(snapshot, nowMs) {
    const startTime = snapshot?.service?.startTime ?? null;
    return {
        empty: !snapshot,
        text: startTime ? formatBareDuration(startTime, nowMs) : "—",
        title: startTime ? `就绪于 ${formatAbsolute(startTime)}` : "暂无就绪时刻",
    };
}

/** 平台连接：汇总行、逐平台状态与错误次数；整段缺失与空清单分开。 */
export function platformsView(snapshot, nowMs) {
    const platforms = snapshot?.platforms ?? null;
    if (!snapshot) return { empty: true, summary: "平台清单暂不可用", rows: [], note: "暂无数据" };
    if (!platforms) return { empty: false, summary: "平台清单暂不可用", rows: [], note: "平台数据未上报" };
    const summary = platforms.summary;
    const summaryText =
        summary && summary.running !== null && summary.total !== null ? `平台 ${formatNumber(summary.running)} / ${formatNumber(summary.total)} 在线` : "平台清单暂不可用";
    const items = platforms.items;
    const rows = (items ?? []).map((item) => {
        const status = item.status ?? "unknown";
        const statusLabel = { running: "运行中", error: "错误", stopped: "已停止", unknown: "未知" }[status] ?? "未知";
        const uptime = status === "running" && item.startedAt ? formatBareDuration(item.startedAt, nowMs) : "—";
        const error = item.errorCount ? `错误 ×${formatNumber(item.errorCount)}` : "";
        const title = item.lastError
            ? `${item.lastError.message}（${formatAbsolute(item.lastError.timestamp)}）`
            : `${item.name} · ${statusLabel}${uptime !== "—" ? ` · 已运行 ${uptime}` : ""}`;
        return { id: item.id, name: item.name, status, statusLabel, uptime, error, title, hasError: status === "error" };
    });
    return {
        empty: false,
        summary: summaryText,
        rows,
        note: items === null ? "平台数据未上报" : items.length === 0 ? "暂无平台" : "",
    };
}

/** 资源占用：CPU、进程内存、线程数；空字段显示「—」而不是 0。 */
export function resourcesView(snapshot, nowMs) {
    const resources = snapshot?.resources ?? null;
    if (!snapshot) return { empty: true, cpuText: "—", cpuWidth: 0, memoryText: "—", memoryWidth: 0, threadsText: null };
    if (!resources) return { empty: false, cpuText: "—", cpuWidth: 0, memoryText: "—", memoryWidth: 0, threadsText: null };
    const cpu = resources.cpuPercent;
    const processMb = resources.memory?.processMb ?? null;
    const systemMb = resources.memory?.systemMb ?? null;
    const memoryWidth = processMb === null ? 0 : systemMb && systemMb > 0 ? Math.min(100, (processMb / systemMb) * 100) : Math.min(100, (processMb / 2048) * 100);
    return {
        empty: false,
        cpuText: cpu === null ? "—" : `${Math.round(cpu)}%`,
        cpuWidth: cpu === null ? 0 : Math.min(100, Math.max(0, cpu)),
        memoryText: processMb === null ? "—" : `${formatNumber(Math.round(processMb))} MB`,
        memoryWidth,
        threadsText: resources.threadCount === null ? null : `线程 ${formatNumber(resources.threadCount)}`,
    };
}

/** 消息量：今日合计与 24 点柱状；缺失小时留空槽，最近数据靠「现在」一侧。 */
export function messagesView(snapshot, nowMs) {
    const messages = snapshot?.messages ?? null;
    if (!snapshot) return { empty: true, totalText: "暂无数据", bars: [], peakText: "", emptyChart: true };
    if (!messages) return { empty: false, totalText: "消息数据未上报", bars: [], peakText: "", emptyChart: true };
    const series = messages.series ?? [];
    const currentHour = Math.floor(nowMs / 1000 / 3600); // 槽位对齐当前小时，最后一个槽是「现在」。
    const slots = new Array(24).fill(null); // 固定 24 个小时槽位，缺失小时留空。
    for (const point of series) {
        const offset = currentHour - Math.floor(point.ts / 3600);
        if (offset < 0 || offset > 23) continue;
        slots[23 - offset] = point;
    }
    const max = slots.reduce((value, point) => (point ? Math.max(value, point.count) : value), 0);
    const bars = slots.map((point) =>
        point === null
            ? { empty: true, ts: null, count: null, height: 0, title: "" }
            : { empty: false, ts: point.ts, count: point.count, height: max > 0 ? Math.max(4, Math.round((point.count / max) * 100)) : 4, title: `${formatClock(point.ts)} · ${formatNumber(point.count)} 条` },
    );
    const peak = slots.reduce((best, point) => (point && point.count > (best?.count ?? -1) ? point : best), null);
    return {
        empty: false,
        totalText: messages.total === null ? "消息数据未上报" : `今日 ${formatNumber(messages.total)}`,
        bars,
        peakText: peak ? `峰值 ${formatNumber(peak.count)}（${formatClock(peak.ts)}）` : "",
        emptyChart: series.length === 0,
        chartHint: series.length === 0 ? "逐小时趋势未上报" : "", // 趋势缺失时给说明，格子不留一整块空白。
    };
}

/** 运行日志：只取异常级别，按时间倒序排成时间轴节点。 */
export function logsView(snapshot, nowMs) {
    const tail = snapshot?.logs?.tail ?? null;
    if (!snapshot) return { empty: true, nodes: [], updatedText: "暂无数据" };
    if (!tail) return { empty: false, nodes: [], updatedText: "日志数据未上报" };
    const nodes = tail
        .filter((item) => item.level === "ERROR" || item.level === "CRITICAL" || item.level === "WARNING")
        .sort((left, right) => right.ts - left.ts)
        .map((item) => ({
            ts: item.ts,
            level: item.level === "WARNING" ? "WARNING" : "ERROR",
            time: formatClock(item.ts),
            text: item.text,
            levelClass: item.level === "WARNING" ? "warning" : "error",
        }));
    const collectedAt = snapshot?.collectedAt;
    return {
        empty: false,
        nodes,
        updatedText: collectedAt ? `${formatClock(collectedAt)} 更新 · 间隔 ${SAMPLE_INTERVAL_SEC} 秒` : "暂无更新时间",
    };
}

/** 一行状态摘要，供菜单复制与测试使用。 */
export function summaryText(snapshot, nowMs) {
    const status = statusView(snapshot, nowMs);
    const uptime = uptimeView(snapshot, nowMs);
    const collected = snapshot?.collectedAt ? ` · 更新于 ${formatAbsolute(snapshot.collectedAt)}` : "";
    return `AstrBot ${status.label}${uptime.text !== "—" ? ` · ${uptime.text}` : ""}${collected}`;
}

/**
 * 右键菜单分组：导航、只读动作、写操作。
 * 写操作只在平台连接格出现；没有错误平台时整组隐藏。
 */
export function buildMenuItems({ tileType, snapshot, busy = false, nowMs = Date.now(), onOpenConsole = () => {}, onRefresh = () => {}, onCopy = () => {}, onTogglePlatform = () => {} }) {
    const items = [
        { id: "open-console", label: "打开 AstrBot 控制台", separatorBefore: true, onSelect: onOpenConsole }, // 与外壳「折叠」等通用项分组。
        { id: "refresh", label: "刷新一次", disabled: busy, onSelect: onRefresh },
        { id: "copy-summary", label: "复制状态摘要", onSelect: onCopy },
    ];
    if (tileType !== "platforms") return items;
    const view = platformsView(snapshot, nowMs);
    const errors = view.rows.filter((row) => row.hasError && row.id); // 只在可启停的出错平台上提供入口。
    errors.forEach((row, index) => {
        const enabled = row.status !== "running"; // 出错或已停止的平台按「启动」处理。
        items.push({
            id: `toggle-platform:${row.id ?? index}`,
            label: `${enabled ? "启动" : "停止"} ${row.name}`,
            disabled: busy,
            separatorBefore: index === 0,
            onSelect: () => onTogglePlatform(row),
        });
    });
    return items;
}

/** 预览或测试里没有宿主桥时使用的固定样例。 */
const pageBridge = () => globalThis.window?.__DESKTOP_TOOLS_NATIVE__;

/**
 * 模块级单例缓存：同一插件的多个类型实例共用一份快照、两条通道与一个兜底定时器。
 * 按 instanceId 去重：重复挂载同一实例不重复注册；后续实例只读缓存。
 */
export function createStatusStore({ invoke, listen, now = () => Date.now(), sample = null } = {}) {
    let snapshot = sample ?? null; // 最近一次有效快照。
    let receivedAt = sample ? Math.floor(now() / 1000) : null; // 入缓存时刻，Unix 秒。
    let ownerId = null; // 实际注册通道的实例；读取缓冲必须用它。
    let afterSequence = 0; // 已消费到的宿主序号。
    let stopEvents; // 宿主事件退订函数。
    let registering; // 进行中的通道注册。
    let pushReady = false; // listen 通道是否可用。
    let requestReady = false; // request 通道是否可用。
    let needsPairing = false; // 凭据未配对：停止重复注册，等设置变化后再试。
    let tokenInvalid = false; // 已配对但 token 被对端拒绝（401），等设置变化后再校验。
    let requesting; // 进行中的拉取。
    let actionPending = false; // 启停请求进行中，菜单项整体禁用。
    let fallbackTimer; // 拉取兜底定时器，全部实例退订后清掉。
    let lastError = null; // 最近一次结构化失败。
    let disposed = false; // 释放后拒绝迟到结果。
    const subscribers = new Map(); // instanceId -> 渲染回调集合。
    const visible = new Map(); // instanceId -> 是否可见。
    const warnings = new Set(); // 同一条解析告警只记一次。

    /** 通知所有可见实例重绘。 */
    const notify = () => {
        if (disposed) return;
        for (const [instanceId, listeners] of subscribers) {
            if (visible.get(instanceId) === false) continue; // 不可见时暂停视觉刷新。
            for (const listener of listeners) listener();
        }
    };

    /** 同一类解析告警只记一次，避免每秒刷日志。 */
    const warn = (message) => {
        if (warnings.has(message)) return;
        warnings.add(message);
        console.warn(`AstrBot 状态：${message}`);
    };

    /** 凭据未配置是可恢复的业务状态，不按普通失败反复 warn。 */
    const isCredentialUnavailable = (error) => error?.code === "loopback-credential-unavailable" || String(error?.message ?? error ?? "").includes("loopback-credential-unavailable");

    /**
     * 配对变化（保存/修改/清除 token）后重注册两条通道，并主动拉取一次校验 token。
     * 不依赖 needsPairing：已配对时改 token 也必须立即生效。同名 channelId 由宿主替换旧通道。
     */
    const retryChannels = (instanceId = "") => {
        if (disposed) return;
        if (!ownerId && instanceId) ownerId = instanceId; // 重挂时清理已清空 owner，本次实例先接管再注册。
        needsPairing = false;
        tokenInvalid = false;
        lastError = null;
        pushReady = false;
        requestReady = false;
        afterSequence = 0;
        stopEvents?.(); // 释放旧宿主事件监听，由新一轮注册重新建立。
        stopEvents = undefined;
        if (fallbackTimer) clearInterval(fallbackTimer);
        fallbackTimer = undefined;
        const run = async () => {
            if (registering) await registering.catch(() => undefined); // 等旧注册结束再同名替换。
            if (disposed) return;
            await ensureChannels();
            if (!disposed && requestReady) await requestSnapshot({ validateToken: true });
            notify();
        };
        void run();
        notify();
    };

    /** 通道随实例释放后由仍在屏幕上的实例接手；只是滚出视口时不抢端口。 */
    const promoteOwner = (error) => {
        const detail = `${error?.code ?? ""} ${error?.message ?? error ?? ""}`;
        if (!detail.includes("loopback-channel-not-found")) return false;
        const next = [...subscribers.keys()].find((id) => id !== ownerId);
        if (!next) return false;
        ownerId = next;
        pushReady = false;
        requestReady = false;
        afterSequence = 0;
        stopEvents?.();
        stopEvents = undefined;
        void ensureChannels();
        return true;
    };

    /** 接收一份快照；过期或倒退的数据不覆盖缓存。 */
    const ingest = (body, at, applyReceiveRule = true) => {
        const parsed = parseSnapshot(body, warn);
        if (!parsed) return false;
        if (applyReceiveRule && isReceiveTooLate(at, parsed.collectedAt)) {
            warn("推送超过 30 秒才到，已丢弃"); // 只判推送；拉取响应本来就可能是稍旧的快照。
            return false;
        }
        if (snapshot?.collectedAt && parsed.collectedAt && parsed.collectedAt < snapshot.collectedAt) return false; // 旧快照不回退。
        snapshot = parsed;
        receivedAt = at ?? Math.floor(now() / 1000);
        tokenInvalid = false; // 能收到数据说明对端已接受当前 token。
        lastError = null;
        return true;
    };

    /** 读取 listen 通道缓冲；读到即消费，序号只增不减。注册期间用捕获的实例，避免 owner 切换后读错实例。 */
    const readPush = async (instanceId = ownerId) => {
        if (!invoke || !instanceId || !pushReady || disposed) return;
        try {
            const entries = await invoke("read_loopback_channel", { instanceId, channelId: PUSH_CHANNEL, afterSequence });
            let changed = false;
            for (const entry of Array.isArray(entries) ? entries : []) {
                if (Number.isSafeInteger(entry?.sequence)) afterSequence = Math.max(afterSequence, entry.sequence);
                if (typeof entry?.body === "string" && ingest(entry.body, entry.receivedAt)) changed = true;
            }
            if (changed) notify();
        } catch (error) {
            lastError = String(error?.message ?? error);
            if (!promoteOwner(error)) warn(`读取推送失败：${lastError}`);
        }
    };

    /** 经 8766 拉取一次快照，作为推送缺失时的兜底。注册期间用捕获的实例。 */
    const requestSnapshot = async ({ validateToken = false, instanceId = ownerId } = {}) => {
        if (!invoke || !instanceId || !requestReady || requesting || disposed) return;
        requesting = (async () => {
            try {
                const response = await invoke("request_loopback", { instanceId, channelId: REQUEST_CHANNEL, method: "GET", path: "/v1/snapshot" });
                if (response?.status === 401) {
                    tokenInvalid = true; // 对端明确拒绝 token：给可感知状态，不静默失败。
                    needsPairing = false;
                    lastError = "共享 token 无效";
                    notify();
                    return false;
                }
                if (!response || response.status < 200 || response.status >= 300) {
                    lastError = `拉取失败：HTTP ${response?.status ?? "未知"}`;
                    return false;
                }
                if (tokenInvalid) {
                    tokenInvalid = false;
                    notify();
                }
                return ingest(response.body, Math.floor(now() / 1000), false); // 拉取不套用推送的 30 秒接收判定。
            } catch (error) {
                lastError = String(error?.message ?? error);
                promoteOwner(error);
                return false;
            } finally {
                requesting = undefined;
            }
        })().then((changed) => {
            if (changed || validateToken) notify();
            return changed;
        });
        return requesting;
    };

    /** 推送或数据过期时补一次拉取。 */
    const maybeRefresh = () => {
        if (!requestReady || needsPairing || tokenInvalid) return; // 未配对或 token 无效时等设置变化再试。
        if (!snapshot || isDisplayStale(snapshot.collectedAt, now())) void requestSnapshot();
    };

    /** 拉取兜底：全部实例不可见或数据新鲜时不发请求。 */
    const ensureFallback = () => {
        if (fallbackTimer || !requestReady || disposed) return;
        fallbackTimer = setInterval(() => {
            if (!subscribers.size) return;
            const anyVisible = [...visible.values()].some(Boolean);
            if (anyVisible) maybeRefresh();
        }, FALLBACK_PULL_MS);
        fallbackTimer.unref?.(); // 测试进程不被兜底定时器挂住。
    };

    /** 注册两条通道；失败只记原因，另一条仍可用。 */
    const ensureChannels = async () => {
        if (!invoke || !ownerId || registering || disposed || needsPairing) return registering; // 未配对时等设置变化再试。
        registering = (async () => {
            const instanceId = ownerId;
            try {
                stopEvents ??= await listen?.("component-host-event", (event) => handleHostEvent(event));
            } catch (error) {
                lastError = String(error?.message ?? error);
            }
            try {
                await invoke("register_loopback_channel", {
                    instanceId,
                    channelId: PUSH_CHANNEL,
                    mode: "listen",
                    port: PUSH_PORT,
                    credentialRef: CREDENTIAL_REF,
                    headerName: TOKEN_HEADER,
                });
                pushReady = true;
                afterSequence = 0;
                await readPush(instanceId); // 先读一次缓冲，宿主随后才开始发通知。
            } catch (error) {
                pushReady = false;
                lastError = String(error?.message ?? error);
                if (isCredentialUnavailable(error)) {
                    needsPairing = true; // 两条通道共用同一凭据，不再尝试第二条、也不重复注册。
                    notify();
                    return;
                }
                warn(`推送通道注册失败：${lastError}`);
            }
            try {
                await invoke("register_loopback_channel", {
                    instanceId,
                    channelId: REQUEST_CHANNEL,
                    mode: "request",
                    baseUrl: REQUEST_BASE_URL,
                    credentialRef: CREDENTIAL_REF,
                    headerName: TOKEN_HEADER,
                });
                requestReady = true;
                ensureFallback();
            } catch (error) {
                requestReady = false;
                lastError = String(error?.message ?? error);
                if (isCredentialUnavailable(error)) {
                    needsPairing = true;
                    notify();
                    return;
                }
                warn(`拉取通道注册失败：${lastError}`);
            }
            if (!needsPairing && !tokenInvalid && (!snapshot || isDisplayStale(snapshot.collectedAt, now()))) void requestSnapshot({ instanceId }); // 首次注册后用捕获实例兜底一次，不跟随全局 owner。
        })().finally(() => {
            registering = undefined;
        });
        return registering;
    };

    /** 宿主通知：事件本身不带正文，按通道标识回来读一次。 */
    const handleHostEvent = (event) => {
        if (disposed || typeof event?.kind !== "string") return;
        if (event.kind !== `loopback-data:${PUSH_CHANNEL}`) return;
        void readPush();
    };

    /** 挂一个实例；第一个实例负责注册通道，同一实例重复挂载只保留一个订阅。 */
    const attach = (instanceId, listener) => {
        let listeners = subscribers.get(instanceId);
        if (!listeners) {
            listeners = new Set();
            subscribers.set(instanceId, listeners);
        }
        listeners.add(listener);
        if (!visible.has(instanceId)) visible.set(instanceId, true);
        if (!ownerId) {
            ownerId = instanceId;
            void ensureChannels();
        } else {
            maybeRefresh();
        }
        return () => {
            const current = subscribers.get(instanceId);
            if (!current) return;
            current.delete(listener);
            if (current.size > 0) return;
            subscribers.delete(instanceId);
            visible.delete(instanceId);
            if (subscribers.size) return; // 持有者的格子滚出视口时保留旧通道，读失败再接手，避免抢占端口。
            ownerId = null;
            stopEvents?.();
            stopEvents = undefined;
            if (fallbackTimer) clearInterval(fallbackTimer);
            fallbackTimer = undefined;
        };
    };

    /** 可见性变化：隐藏时暂停视觉刷新，恢复时按需补一次拉取。 */
    const setVisible = (instanceId, next) => {
        if (!subscribers.has(instanceId)) return;
        visible.set(instanceId, next);
        if (next) {
            notify();
            maybeRefresh();
        }
    };

    /** 主动刷新：先读推送缓冲，再拉一次快照。 */
    const refresh = async () => {
        await readPush();
        await requestSnapshot();
        notify();
    };

    /** 平台启停走 8766 的 POST /v1/actions，成功后立即重读快照。 */
    const togglePlatform = async (platform) => {
        if (actionPending) return { ok: false, error: "启停请求进行中" };
        if (!invoke || !ownerId || !requestReady) return { ok: false, error: "拉取通道未就绪" };
        actionPending = true;
        notify();
        try {
            const enabled = platform.status !== "running";
            const response = await invoke("request_loopback", {
                instanceId: ownerId,
                channelId: REQUEST_CHANNEL,
                method: "POST",
                path: "/v1/actions",
                body: JSON.stringify({ action: "set_enabled", id: platform.id, enabled }),
            });
            let payload = null;
            try {
                payload = response?.body ? JSON.parse(response.body) : null;
            } catch {
                payload = null;
            }
            if (!response || response.status < 200 || response.status >= 300 || payload?.ok === false) {
                lastError = payload?.detail ?? payload?.reason ?? `启停失败：HTTP ${response?.status ?? "未知"}`;
                return { ok: false, error: lastError };
            }
            await requestSnapshot();
            return { ok: true };
        } catch (error) {
            lastError = String(error?.message ?? error);
            return { ok: false, error: lastError };
        } finally {
            actionPending = false;
            notify();
        }
    };

    return {
        attach,
        setVisible,
        refresh,
        togglePlatform,
        readPush,
        requestSnapshot,
        ingest,
        handleHostEvent,
        retryChannels,
        snapshot: () => snapshot,
        state: () => ({ snapshot, receivedAt, stale: isDisplayStale(snapshot?.collectedAt, now()), actionPending, lastError, needsPairing, tokenInvalid, pushReady, requestReady }),
        dispose: () => {
            disposed = true;
            stopEvents?.();
            if (fallbackTimer) clearInterval(fallbackTimer);
            subscribers.clear();
            visible.clear();
        },
    };
}

/** 正式页面共用一份 store；没有宿主桥时用固定样例驱动预览。 */
let sharedStore = null;
export function statusStore() {
    if (sharedStore) return sharedStore;
    const bridge = pageBridge();
    sharedStore = bridge
        ? createStatusStore({ invoke: (command, payload) => bridge.invoke(command, payload), listen: (name, handler) => bridge.listen(name, handler), now: () => Date.now() })
        : createStatusStore({ now: () => Date.now(), sample: sampleSnapshot(Date.now()) });
    return sharedStore;
}

/** 测试或预览可替换单例。 */
export function setStatusStore(store) {
    sharedStore = store;
}

/** 创建 DOM 节点的小工具。 */
function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}

/** 文本、标题、类名与尺寸只在变化时写入，避免每秒重绘触发无谓的样式与布局。 */
const setText = (node, value) => {
    const text = value ?? "";
    if (node.textContent !== text) node.textContent = text;
};
const setTitle = (node, value) => {
    const text = value ?? "";
    if (node.title !== text) node.title = text;
};
const setClass = (node, value) => {
    if (node.className !== value) node.className = value;
};
const setHidden = (node, hidden) => {
    if (node.hidden !== hidden) node.hidden = hidden;
};
const setWidth = (node, value) => {
    const text = `${Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0))}%`;
    if (node.style.width !== text) node.style.width = text;
};
const setHeight = (node, value) => {
    const text = `${Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0))}%`;
    if (node.style.height !== text) node.style.height = text;
};
/** 共用的空态/原因行，只换文字与显隐。 */
function createNotice() {
    const node = element("div", "astrbot-tile__notice");
    const message = element("p", "astrbot-tile__empty", "");
    const reason = element("p", "astrbot-tile__reason", "");
    node.append(message, reason);
    setHidden(node, true);
    return {
        node,
        show(text, reasonText = "") {
            setText(message, text);
            setText(reason, reasonText);
            setHidden(reason, !reasonText);
            setHidden(node, false);
        },
        hide() {
            setHidden(node, true);
        },
    };
}

/** 未配对 / token 无效的固定提示；不渲染任何快照数据。 */
function createModeSection(title, hint, invalid = false) {
    const node = element("div", "astrbot-tile__mode");
    node.append(
        element("div", `astrbot-tile__pairing-title${invalid ? " astrbot-tile__pairing-title--invalid" : ""}`, title),
        element("p", "astrbot-tile__pairing-hint", hint),
    );
    setHidden(node, true);
    return { node };
}

/** 状态：结论点、结论、就绪时长与过期标记共用一条稳定的行。 */
function createStatusContent(content, notice) {
    const line = element("div", "astrbot-tile__level");
    const dot = element("span", "astrbot-tile__dot");
    const verdict = element("span", "astrbot-tile__verdict");
    const suffix = element("span", "astrbot-tile__level-suffix");
    line.append(dot, verdict, suffix);
    const emptyReason = element("p", "astrbot-tile__reason", "采集插件未上报或未安装");
    setHidden(emptyReason, true);
    content.append(line, emptyReason);
    return {
        update(snapshot, nowMs) {
            const view = statusView(snapshot, nowMs);
            setClass(line, `astrbot-tile__level astrbot-tile__level--${view.levelClass}`);
            setClass(dot, `astrbot-tile__dot astrbot-tile__dot--${view.levelClass}`);
            setTitle(dot, view.dotTitle);
            setText(verdict, view.label);
            const text = view.readyText !== "—" ? view.readyText : "";
            setText(suffix, text ? ` · ${text}` : "");
            setTitle(suffix, view.readyTitle);
            setHidden(suffix, !text);
            setHidden(emptyReason, !view.empty);
            notice.hide();
        },
    };
}

/** 就绪时长：一行大号时长，过期沿用最后一次快照。 */
function createUptimeContent(content, notice) {
    const text = element("div", "astrbot-tile__uptime");
    content.append(text);
    return {
        update(snapshot, nowMs) {
            const view = uptimeView(snapshot, nowMs);
            if (view.empty) {
                setHidden(content, true);
                notice.show("暂无数据", "采集插件未上报或未安装");
                return;
            }
            setHidden(content, false);
            notice.hide();
            setText(text, view.text);
            setTitle(text, view.title);
        },
    };
}

/** 平台连接：汇总、固定 6 行槽位与「还有 N 个」；行内子节点只更新文字与状态点。 */
function createPlatformsContent(content, notice) {
    const summary = element("div", "astrbot-tile__summary");
    const list = element("div", "astrbot-tile__platforms");
    const rows = [];
    for (let index = 0; index < PLATFORM_ROWS; index += 1) {
        const row = element("div", "astrbot-tile__platform");
        const dot = element("span", "astrbot-tile__dot");
        const name = element("span", "astrbot-tile__platform-name");
        const uptime = element("span", "astrbot-tile__platform-uptime");
        const error = element("span", "astrbot-tile__platform-error");
        row.append(dot, name, uptime, error);
        setHidden(row, true);
        list.append(row);
        rows.push({ row, dot, name, uptime, error });
    }
    const overflow = element("p", "astrbot-tile__reason", "");
    const note = element("p", "astrbot-tile__reason", "");
    setHidden(overflow, true);
    setHidden(note, true);
    content.append(summary, list, overflow, note);
    return {
        update(snapshot, nowMs) {
            const view = platformsView(snapshot, nowMs);
            if (view.empty) {
                setHidden(content, true);
                notice.show("暂无数据", "采集插件未上报或未安装");
                return;
            }
            setHidden(content, false);
            setText(summary, view.summary);
            const limit = PLATFORM_ROWS;
            rows.forEach((slot, index) => {
                const visible = index < limit && index < view.rows.length;
                setHidden(slot.row, !visible);
                if (!visible) return;
                const data = view.rows[index];
                setTitle(slot.row, data.title);
                setClass(slot.dot, `astrbot-tile__dot astrbot-tile__dot--${data.status}`);
                setText(slot.name, data.name);
                setText(slot.uptime, data.uptime);
                setText(slot.error, data.error || "");
                setHidden(slot.error, !data.error);
            });
            setText(overflow, `还有 ${Math.max(0, view.rows.length - limit)} 个`);
            setHidden(overflow, view.rows.length <= limit);
            setText(note, view.note || "");
            setHidden(note, !view.note);
            notice.hide();
        },
    };
}

/** 资源占用：两条固定横条与线程数文字。 */
function createResourcesContent(content, notice) {
    const makeRow = (label) => {
        const row = element("div", "astrbot-tile__resource");
        const labelEl = element("span", "astrbot-tile__resource-label", label);
        const track = element("div", "astrbot-tile__bar");
        const fill = element("span", "astrbot-tile__bar-fill");
        track.append(fill);
        const value = element("span", "astrbot-tile__resource-value");
        row.append(labelEl, track, value);
        return { row, fill, value };
    };
    const cpu = makeRow("CPU");
    const memory = makeRow("内存");
    const threads = element("div", "astrbot-tile__threads");
    const sampling = element("p", "astrbot-tile__reason", "进程自身占用 · 5 秒采样");
    content.append(cpu.row, memory.row, threads, sampling);
    return {
        update(snapshot, nowMs) {
            const view = resourcesView(snapshot, nowMs);
            if (view.empty) {
                setHidden(content, true);
                notice.show("暂无数据", "采集插件未上报或未安装");
                return;
            }
            setHidden(content, false);
            notice.hide();
            setText(cpu.value, view.cpuText);
            setWidth(cpu.fill, view.cpuWidth);
            setText(memory.value, view.memoryText);
            setWidth(memory.fill, view.memoryWidth);
            setText(threads, view.threadsText || "");
            setHidden(threads, !view.threadsText);
            setHidden(sampling, false);
        },
    };
}

/** 消息量：固定 24 个柱槽，空槽隐藏填充而不重建列。 */
function createMessagesContent(content, notice) {
    const total = element("div", "astrbot-tile__total");
    const chart = element("div", "astrbot-tile__chart");
    const columns = [];
    for (let index = 0; index < 24; index += 1) {
        const column = element("span", "astrbot-tile__bar-column");
        const fill = element("span", "astrbot-tile__bar-fill");
        setHidden(fill, true);
        column.append(fill);
        chart.append(column);
        columns.push({ column, fill });
    }
    const labels = element("div", "astrbot-tile__chart-labels");
    labels.append(
        element("span", "astrbot-tile__chart-label", "24 小时前"),
        element("span", "astrbot-tile__chart-label", "12 小时前"),
        element("span", "astrbot-tile__chart-label", "现在"),
    );
    const peak = element("p", "astrbot-tile__reason", "");
    setHidden(peak, true);
    content.append(total, chart, labels, peak);
    return {
        update(snapshot, nowMs) {
            const view = messagesView(snapshot, nowMs);
            if (view.empty) {
                setHidden(content, true);
                notice.show("暂无数据", "采集插件未上报或未安装");
                return;
            }
            setHidden(content, false);
            notice.hide();
            setText(total, view.totalText);
            setHidden(chart, false); // 趋势缺失也保留 24 槽骨架，格子不留空白。
            setHidden(labels, false);
            view.bars.forEach((bar, index) => {
                const slot = columns[index];
                setTitle(slot.column, bar.empty ? "" : bar.title);
                setHidden(slot.fill, bar.empty);
                if (!bar.empty) setHeight(slot.fill, bar.height);
            });
            setText(peak, view.peakText || view.chartHint);
            setHidden(peak, !(view.peakText || view.chartHint));
        },
    };
}

/** 运行日志：固定 5 个时间轴节点槽位。 */
function createLogsContent(content, notice) {
    const timeline = element("div", "astrbot-tile__timeline");
    const rows = [];
    for (let index = 0; index < LOG_ROWS; index += 1) {
        const row = element("div", "astrbot-tile__node");
        const time = element("time", "astrbot-tile__node-time");
        const axis = element("span", "astrbot-tile__node-axis");
        const dot = element("span", "astrbot-tile__node-dot");
        axis.append(dot);
        const entry = element("div", "astrbot-tile__node-entry");
        const level = element("span", "astrbot-tile__node-level");
        const text = element("span", "astrbot-tile__node-text");
        entry.append(level, text);
        row.append(time, axis, entry);
        setHidden(row, true);
        timeline.append(row);
        rows.push({ row, time, dot, entry, level, text });
    }
    const updated = element("p", "astrbot-tile__updated");
    content.append(timeline, updated);
    return {
        update(snapshot, nowMs) {
            const view = logsView(snapshot, nowMs);
            if (view.empty) {
                setHidden(content, true);
                notice.show("暂无数据", "采集插件未上报或未安装");
                setText(updated, view.updatedText);
                return;
            }
            setHidden(content, false);
            if (!view.nodes.length) {
                setHidden(timeline, true);
                notice.hide();
                setText(updated, view.updatedText);
                return;
            }
            notice.hide();
            setHidden(timeline, false);
            const limit = LOG_ROWS;
            rows.forEach((slot, index) => {
                const visible = index < limit && index < view.nodes.length;
                setHidden(slot.row, !visible);
                if (!visible) return;
                const data = view.nodes[index];
                setText(slot.time, data.time);
                setClass(slot.dot, `astrbot-tile__node-dot astrbot-tile__node-dot--${data.levelClass}`);
                setClass(slot.level, `astrbot-tile__node-level astrbot-tile__node-level--${data.levelClass}`);
                setText(slot.level, data.level);
                setText(slot.text, data.text);
                setTitle(slot.entry, data.text);
            });
            setText(updated, view.updatedText);
        },
    };
}

/** 按类型创建一次骨架；之后只更新文字、属性与显隐。 */
const contentFactories = {
    status: createStatusContent,
    uptime: createUptimeContent,
    platforms: createPlatformsContent,
    resources: createResourcesContent,
    messages: createMessagesContent,
    logs: createLogsContent,
};

function createTileViewer(root, tileType) {
    root.dataset.tileType = tileType;
    const body = element("div", "astrbot-tile__body");
    const pairing = createModeSection("未配对", "去「设置 → 功能格子 → AstrBot 状态」粘贴共享 token，配对后自动重连。");
    const invalid = createModeSection(
        "token 无效",
        "共享 token 未通过校验，请在「设置 → 功能格子 → AstrBot 状态」重新保存。",
        true,
    );
    const normal = element("div", "astrbot-tile__normal");
    const content = element("div", "astrbot-tile__content");
    const notice = createNotice();
    normal.append(content, notice.node);
    body.append(pairing.node, invalid.node, normal);
    root.append(body);
    const view = contentFactories[tileType](content, notice);
    return {
        update(state, nowMs) {
            const pairingOn = state.needsPairing === true;
            const invalidOn = !pairingOn && state.tokenInvalid === true;
            setHidden(pairing.node, !pairingOn);
            setHidden(invalid.node, !invalidOn);
            setHidden(normal, pairingOn || invalidOn);
            if (pairingOn || invalidOn) return;
            view.update(state.snapshot, nowMs);
        },
    };
}

/** 打开 AstrBot 控制台；宿主命令失败时退回窗口导航。 */
function openConsole() {
    const bridge = pageBridge();
    const task = bridge?.invoke?.("open_collect_link", { url: CONSOLE_URL });
    if (task?.catch) task.catch(() => globalThis.window?.open?.(CONSOLE_URL, "_blank", "noopener"));
    else globalThis.window?.open?.(CONSOLE_URL, "_blank", "noopener");
}

/** 复制一行状态摘要；剪贴板不可用时记录失败原因。 */
function copySummary(store) {
    const text = summaryText(store.snapshot(), Date.now());
    const clipboard = globalThis.navigator?.clipboard;
    if (clipboard?.writeText) clipboard.writeText(text).catch((error) => console.warn("AstrBot 状态：复制失败", error));
}

/** 菜单签名：相同就不重复注册，避免每秒重绘时抖动宿主菜单。 */
function menuSignature(items) {
    return JSON.stringify(items.map((item) => [item.id, item.label, item.disabled === true]));
}

/** 设置页的一行只读信息；reason 说明当前为什么显示「—」。 */
function settingsRow(label, value, reason = "") {
    const row = element("div", "astrbot-settings__row");
    row.append(element("span", "astrbot-settings__label", label));
    const cell = element("span", "astrbot-settings__value", value);
    if (reason) cell.append(element("span", "astrbot-settings__reason", reason));
    row.append(cell);
    return row;
}

/** 已处理到设置页写入的配对版本；变化时重试通道注册。 */
let seenPairingVersion = 0;

/** 设置页无法定位桌面实例时的说明。 */
export const VERSION_ENTRY_UNAVAILABLE = "需在桌面放置格子后可见";

/**
 * 设置页版本视图：从主动拉取到的快照里读版本。
 * 过期或字段缺失一律「—」并给原因；不消费桌面格子的缓冲数据。
 */
export function versionInfo(snapshot, nowMs) {
    const stale = Boolean(snapshot) && isDisplayStale(snapshot.collectedAt, nowMs);
    const versions = snapshot?.service?.versions && typeof snapshot.service.versions === "object" ? snapshot.service.versions : null;
    const pick = (value) => {
        if (stale) return { value: "—", reason: "数据过期" };
        const text = typeof value === "string" && value.trim() ? value.trim() : "";
        return text ? { value: text, reason: "" } : { value: "—", reason: "字段未上报" };
    };
    return {
        stale,
        collectedAt: snapshot?.collectedAt ?? null,
        astrbot: pick(snapshot?.instance?.astrbotVersion ?? versions?.astrbot),
        webui: pick(versions?.webui),
        code: pick(versions?.code),
    };
}

/**
 * 设置二级页的版本信息：打开时用 request 通道主动拉取一次快照。
 * 两条路径都不消费桌面格子的缓冲，也不写插件设置；页面打开时取一次，之后只重算过期。
 */
function renderVersionEntry(container, context) {
    container.replaceChildren();
    const makeRow = (label) => {
        const row = element("div", "astrbot-settings__row");
        const value = element("span", "astrbot-settings__value", "—");
        const reason = element("span", "astrbot-settings__reason", "");
        row.append(element("span", "astrbot-settings__label", label), value, reason);
        return { row, set(nextValue, nextReason = "") { setText(value, nextValue); setText(reason, nextReason); } };
    };
    const astrbot = makeRow("AstrBot 版本");
    const webui = makeRow("WebUI 版本");
    const code = makeRow("代码版本");
    const list = element("div", "astrbot-settings__rows");
    list.append(astrbot.row, webui.row, code.row, settingsRow("WebUI 地址", CONSOLE_URL));
    const hint = element("p", "astrbot-settings__hint", "");
    container.append(list, hint);

    /** 没有快照或无法读取时，四行统一给原因，不伪造 0 或空串。 */
    const showUnavailable = (reason, message) => {
        astrbot.set("—", reason);
        webui.set("—", reason);
        code.set("—", reason);
        setText(hint, message);
    };
    const paint = (snapshot) => {
        const view = versionInfo(snapshot, Date.now());
        astrbot.set(view.astrbot.value, view.astrbot.reason);
        webui.set(view.webui.value, view.webui.reason);
        code.set(view.code.value, view.code.reason);
        setText(
            hint,
            view.stale
                ? `数据过期：最近一次采集于 ${formatAbsolute(view.collectedAt)}，超过 60 秒未更新。`
                : `数据更新于 ${formatAbsolute(view.collectedAt)}。`,
        );
    };

    if (!context?.instanceId || typeof context.requestLoopback !== "function") {
        showUnavailable(VERSION_ENTRY_UNAVAILABLE, "桌面没有该插件的格子，设置页读不到通道；共享 token 仍可在上方单独配对。");
        return () => undefined;
    }

    let disposed = false;
    let snapshot = null;
    const load = async () => {
        let tokenRejected = false;
        let failed = false;
        try {
            const response = await context.requestLoopback(REQUEST_CHANNEL, "GET", "/v1/snapshot");
            if (response?.status === 401) tokenRejected = true;
            else if (response && response.status >= 200 && response.status < 300) snapshot = parseSnapshot(response.body);
            else failed = true;
        } catch {
            failed = true;
        }
        if (disposed) return;
        if (tokenRejected) {
            showUnavailable("共享 token 无效", "请在上方重新配对 token 后再打开本页。");
            return;
        }
        if (snapshot) paint(snapshot);
        else showUnavailable("暂无数据", failed ? "AstrBot 未运行、采集插件未安装或拉取通道不可用。" : "快照格式无法解析或采集插件未上报版本。");
    };
    showUnavailable("读取中", "正在向 AstrBot 拉取最近快照…");
    void load();
    const timer = setInterval(() => {
        if (!disposed && snapshot) paint(snapshot); // 只重算过期，不轮询数据。
    }, 5_000);
    return () => {
        disposed = true;
        clearInterval(timer);
    };
}

/**
 * 一个实例的格子内容。
 * 可见时每秒重绘时间相关文案；宿主数据变化、通知或恢复可见时立即重绘。
 */
export default {
    renderTile(container, context) {
        const store = statusStore();
        const pairingVersion = Number(context.settings && typeof context.settings === "object" ? context.settings.pairingVersion : 0);
        if (Number.isFinite(pairingVersion) && pairingVersion > seenPairingVersion) {
            seenPairingVersion = pairingVersion; // 设置页保存或清除 token 后重试一次注册。
            store.retryChannels(context.instanceId); // 单格重挂时清理已清空 owner，由本次实例接管并完成校验。
        }
        const tileType = resolveTileType(context.tileType);
        const root = element("div", "astrbot-tile");
        container.append(root);
        const viewer = createTileViewer(root, tileType); // 骨架只建一次，刷新只改文字/属性/显隐。
        let visible = true; // 观测器回调前的初值。
        let disposed = false; // 清理后拒绝迟到回调。
        let lastMenu = ""; // 上一次注册的菜单签名。
        let tickTimer = null; // 每秒重算就绪时长与过期文案。
        let frame = 0; // 合并同一轮多次刷新通知。

        /** 合并重绘请求，一帧最多画一次。 */
        const schedulePaint = () => {
            if (disposed || !visible || frame) return;
            frame = requestAnimationFrame(() => {
                frame = 0;
                if (disposed || !visible) return;
                viewer.update(store.state(), Date.now());
                registerMenu();
            });
        };

        /** 菜单随数据与进行中的动作更新；内容与顺序不变时跳过。 */
        const registerMenu = () => {
            if (typeof context.menu?.register !== "function") return;
            const state = store.state();
            const items = buildMenuItems({
                tileType,
                snapshot: state.snapshot,
                busy: state.actionPending,
                onOpenConsole: openConsole,
                onRefresh: () => void store.refresh().then(schedulePaint),
                onCopy: () => copySummary(store),
                onTogglePlatform: (row) =>
                    void store.togglePlatform(row).then((result) => {
                        if (!result.ok) console.warn(`AstrBot 状态：启停平台失败：${result.error}`);
                        schedulePaint();
                    }),
            });
            const signature = menuSignature(items);
            if (signature === lastMenu) return;
            lastMenu = signature;
            context.menu.register(items);
        };

        /** 启动秒级重算。 */
        const startTick = () => {
            if (tickTimer !== null) return;
            tickTimer = setInterval(() => schedulePaint(), TICK_MS);
        };

        /** 停掉秒级重算。 */
        const stopTick = () => {
            if (tickTimer === null) return;
            clearInterval(tickTimer);
            tickTimer = null;
        };

        const detach = store.attach(context.instanceId, schedulePaint);
        const viewObserver = new IntersectionObserver((entries) => {
            const next = entries[entries.length - 1]?.isIntersecting ?? true; // 以最新一条为准。
            if (next === visible) return;
            visible = next;
            store.setVisible(context.instanceId, visible);
            if (visible) {
                schedulePaint();
                startTick();
            } else {
                stopTick();
            }
        });
        viewObserver.observe(root);

        /** 窗口隐藏时不刷新，回到前台立即补一次。 */
        const handleVisibility = () => {
            const next = document.visibilityState !== "hidden"; // 隐藏时不刷新。
            if (next === visible) return;
            visible = next;
            store.setVisible(context.instanceId, visible);
            if (visible) {
                schedulePaint();
                startTick();
            } else {
                stopTick();
            }
        };
        document.addEventListener("visibilitychange", handleVisibility);

        viewer.update(store.state(), Date.now());
        registerMenu();
        startTick();

        return () => {
            disposed = true;
            if (frame) cancelAnimationFrame(frame);
            stopTick();
            detach();
            viewObserver.disconnect();
            document.removeEventListener("visibilitychange", handleVisibility);
            root.remove();
        };
    },

    settingsEntries: [
        {
            id: "versions",
            name: "连接与版本",
            /** 版本与 WebUI 地址只读展示；打开时拉取一次，关闭时清理定时器。 */
            render(container, context) {
                return renderVersionEntry(container, context);
            },
        },
    ],

    /** 设置页据此渲染共享 token 配对入口；宿主不写死插件标识。 */
    loopbackCredential: {
        target: CREDENTIAL_REF,
        headerName: TOKEN_HEADER,
        title: "共享 token",
        hint: "在 AstrBot 的 astrbot_plugin_desktop_status 配置里复制 token，粘贴后保存到 Windows 凭据管理器。已保存的 token 不会回显；保存或清除后立即重连，无需重启应用。",
    },
};
