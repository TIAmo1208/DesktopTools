/**
 * DSH 接入插件：在浮岛显示 DSH 余额、会话/job 摘要与待决；连接与配对在插件设置里完成。
 *
 * 只读：通过宿主提供的 loopback 通道读取 /health 与 /snapshot；配对经宿主兑换事务
 * 直接落槽，插件不接触明文令牌；不写 DSH 状态、不弹原生 Toast。
 *
 * 所有宿主能力只走 context.*，不访问宿主页面全局桥，保证正式设置窗
 * 也能完成配对。
 */

/** 插件清单标识；凭据命名空间与设置条目都使用它。 */
const PLUGIN_ID = "plugin.dsh-status";

/** 默认 DSH 桥地址；设置项可改。 */
const DEFAULT_BASE_URL = "http://127.0.0.1:47832";

/** 正常/失败轮询间隔（毫秒）。 */
const POLL_OK_MS = 15000;
const POLL_FAIL_MS = 60000;

/** 无凭据健康通道与带凭据快照通道。 */
const HEALTH_CHANNEL = "dsh-health";
const BRIDGE_CHANNEL = "dsh-bridge";

/** 凭据槽与引用；宿主出站时按 credentialPrefix 补 Bearer。 */
const TOKEN_SLOT = "token";
const TOKEN_REF = `DesktopTools/plugin/${PLUGIN_ID}/${TOKEN_SLOT}`;
const TOKEN_PREFIX = "Bearer ";

/** 插件唯一的去向：打开 DSH；锚点、提醒与卡内按钮共用。 */
export const ISLAND_OPEN_URI = "dsh://open";

/** 每个浮岛实例的运行状态；重挂时保留已见待决，避免重复提醒。 */
const instanceStates = new Map();

/** 按实例+端点+方法/路径共享在途请求，避免挂载切换时同一请求发两份。 */
const inFlightRequests = new Map();

/**
 * 取插件设置并按默认值归一。
 * @param {unknown} value 宿主给的设置。
 * @returns {{ baseUrl: string, paired: boolean }} 设置。
 */
function normalizeSettings(value) {
    const source = value !== null && typeof value === "object" ? value : {};
    const baseUrl = typeof source.baseUrl === "string" && source.baseUrl.trim() ? source.baseUrl : DEFAULT_BASE_URL;
    const bridgeId = typeof source.bridgeId === "string" ? source.bridgeId : "";
    const bindingBaseUrl = typeof source.bindingBaseUrl === "string" ? source.bindingBaseUrl : "";
    return { baseUrl, paired: source.paired === true, bridgeId, bindingBaseUrl };
}

/**
 * 取当前地址可用的持久桥绑定；绑定只对写入时的地址有效。
 * @param {any} settings 插件设置。
 * @param {string} baseUrl 当前归一桥地址。
 * @returns {string} 可携密的 bridgeId；没有有效绑定则为空。
 */
export function boundBridgeIdFor(settings, baseUrl) {
    const bindingBaseUrl = typeof settings?.bindingBaseUrl === "string" ? settings.bindingBaseUrl : "";
    const bridgeId = typeof settings?.bridgeId === "string" ? settings.bridgeId : "";
    return bindingBaseUrl === baseUrl ? bridgeId : "";
}

/**
 * 校验并归一 DSH 桥地址：只允许本机 http、显式端口、无路径/查询。
 * @param {unknown} value 原始输入。
 * @returns {string} 归一后的 `http://127.0.0.1:<port>`。
 */
export function normalizeBaseUrl(value) {
    const text = String(value ?? "").trim();
    if (!text) throw new Error("请填写 DSH 桥地址");
    let url;
    try {
        url = new URL(text);
    } catch {
        throw new Error("DSH 桥地址不是合法 URL");
    }
    if (url.protocol !== "http:") throw new Error("DSH 桥地址只支持 http://127.0.0.1:<端口>");
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") throw new Error("DSH 桥地址只允许本机回环地址");
    if (!url.port) throw new Error("DSH 桥地址必须带端口");
    const port = Number(url.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("DSH 桥端口必须在 1–65535");
    if (url.pathname !== "/" || url.search || url.hash || url.username || url.password) throw new Error("DSH 桥地址不能带路径、查询或账号");
    return `http://127.0.0.1:${port}`;
}

/**
 * 归一配对码：去掉分隔符并转大写，必须正好 20 位 base32 字符。
 * @param {unknown} value 用户输入。
 * @returns {string} 形如 `ABCDE-FGHIJ-KLMNO-PQRST`；非法时抛错。
 */
export function normalizePairingCode(value) {
    const raw = String(value ?? "").toUpperCase().replace(/[^A-Z2-7]/g, "");
    if (raw.length !== 20) throw new Error("配对码应为 20 位字符（可带分隔符）");
    return `${raw.slice(0, 5)}-${raw.slice(5, 10)}-${raw.slice(10, 15)}-${raw.slice(15)}`;
}

/**
 * 解析桥快照；只做插件需要的字段归一，非法结构抛错。
 * @param {unknown} value 快照 JSON 文本或对象。
 * @returns {any} 归一快照。
 */
export function parseSnapshotPayload(value) {
    const data = typeof value === "string" ? JSON.parse(value) : value;
    if (data === null || typeof data !== "object") throw new Error("快照不是对象");
    if (data.protocolVersion !== 1) throw new Error(`不支持的桥协议版本：${String(data.protocolVersion)}`);
    if (typeof data.bridgeId !== "string" || !data.bridgeId) throw new Error("快照缺少 bridgeId");
    if (typeof data.bridgeInstanceId !== "string" || !data.bridgeInstanceId) throw new Error("快照缺少 bridgeInstanceId");
    const wallets = Array.isArray(data.balance?.wallets)
        ? data.balance.wallets
            .filter(wallet => wallet && typeof wallet.currency === "string" && typeof wallet.amount === "string")
            .map(wallet => ({ currency: wallet.currency, amount: wallet.amount }))
        : [];
    const balanceStatus = ["ready", "logged-out", "unauthorized", "error"].includes(data.balance?.status)
        ? data.balance.status
        : "error";
    const sessions = Array.isArray(data.sessions)
        ? data.sessions.filter(row => row && typeof row.id === "string").map(row => ({
            id: String(row.id),
            title: typeof row.title === "string" ? row.title : "",
            status: ["running", "idle", "waiting"].includes(row.status) ? row.status : "idle",
            pending: ["approval", "question", "plan", "none"].includes(row.pending) ? row.pending : "none",
        }))
        : [];
    const jobs = Array.isArray(data.jobs)
        ? data.jobs.filter(row => row && typeof row.id === "string").map(row => ({
            id: String(row.id),
            label: typeof row.label === "string" ? row.label : "",
            status: ["running", "done", "failed"].includes(row.status) ? row.status : "failed",
        }))
        : [];
    const pending = Array.isArray(data.pending)
        ? data.pending
            .filter(item => item && typeof item.id === "string" && typeof item.sessionId === "string")
            .map(item => ({
                id: String(item.id),
                kind: ["approval", "question", "plan"].includes(item.kind) ? item.kind : "approval",
                sessionId: String(item.sessionId),
                since: typeof item.since === "string" ? item.since : "",
            }))
        : [];
    const rawStats = data.taskStats !== null && typeof data.taskStats === "object" ? data.taskStats : data;
    const readCount = key => Number.isSafeInteger(rawStats?.[key]) && rawStats[key] >= 0 ? rawStats[key] : undefined;
    const taskStats = {
        runCount: readCount("runCount"),
        unreadCount: readCount("unreadCount"),
        completedUnreadCount: readCount("completedUnreadCount"),
        unionCount: readCount("unionCount"),
        completeness: typeof rawStats?.completeness === "string" ? rawStats.completeness : "",
        sampledAt: typeof rawStats?.sampledAt === "string" ? rawStats.sampledAt : "",
    };
    return {
        protocolVersion: 1,
        bridgeId: data.bridgeId,
        bridgeInstanceId: data.bridgeInstanceId,
        revision: Number.isSafeInteger(data.revision) ? data.revision : 0,
        fetchedAt: typeof data.fetchedAt === "string" ? data.fetchedAt : "",
        stale: data.stale === true,
        balance: { status: balanceStatus, wallets, reason: typeof data.balance?.reason === "string" ? data.balance.reason : "" },
        sessions,
        jobs,
        pending,
        taskStats,
        hasSessions: Array.isArray(data.sessions),
        hasJobs: Array.isArray(data.jobs),
    };
}

/**
 * 待决的稳定身份：bridgeId + pendingId + since。
 * @param {string} bridgeId 桥稳定身份。
 * @param {{ id: string, since: string }} item 待决。
 * @returns {string} 身份。
 */
export function pendingIdentity(bridgeId, item) {
    return `${bridgeId}|${item.id}|${item.since}`;
}

/**
 * 待决消息 key：完整身份按宿主 128 字符上限截断。
 * @param {string} identity 待决身份。
 * @returns {string} 消息 key。
 */
export function pendingMessageKey(identity) {
    return `dsh-pending:${identity}`.slice(0, 128);
}

/**
 * 从已见集合里挑出新的待决，并把它们记入集合。
 * @param {Set<string>} seen 已见身份。
 * @param {string} bridgeId 桥身份。
 * @param {readonly { id: string, since: string }[]} pending 待决列表。
 * @returns {Array<{ id: string, since: string }>} 新增待决。
 */
export function diffPending(seen, bridgeId, pending) {
    const fresh = [];
    for (const item of pending) {
        const identity = pendingIdentity(bridgeId, item);
        if (seen.has(identity)) continue;
        seen.add(identity);
        fresh.push(item);
    }
    return fresh;
}

/**
 * 下一次轮询间隔：成功 15 秒，失败 60 秒。
 * @param {boolean} ok 本轮是否成功。
 * @returns {number} 毫秒。
 */
export function nextPollDelay(ok) {
    return ok ? POLL_OK_MS : POLL_FAIL_MS;
}

/**
 * 页面是否可见；没有 document 的测试环境按不可见处理。
 * @param {any} [documentRef] document 对象。
 * @returns {boolean} 是否应轮询。
 */
export function shouldPoll(documentRef = typeof document === "undefined" ? undefined : document) {
    return documentRef?.visibilityState === "visible";
}

/** 快照超过两个正常轮询周期即视为整体陈旧。 */
const SNAPSHOT_STALE_MS = POLL_OK_MS * 2;

/** 待决类型的展示优先级：审批、计划确认、提问。 */
const PENDING_KIND_ORDER = { approval: 0, plan: 1, question: 2 };

/** 待决类型固定列文案。 */
const PENDING_KIND_LABEL = { approval: "等待审批", plan: "确认计划", question: "提问" };

/** 浮岛摘要与短说明的文本上限（18 个字符；拉丁字符同样不超过上限）。 */
const ISLAND_SHORT_LIMIT = 18;

/**
 * 取合法的钱包列表；币种或金额为空都不参与读数。
 * @param {any} balance 快照余额。
 * @returns {Array<{ currency: string, amount: string }>} 钱包。
 */
export function validWallets(balance) {
    if (balance?.status !== "ready" || !Array.isArray(balance.wallets)) return [];
    return balance.wallets.filter(wallet =>
        wallet
        && typeof wallet.currency === "string"
        && wallet.currency.trim().length > 0
        && typeof wallet.amount === "string"
        && wallet.amount.trim().length > 0);
}

/**
 * 余额金额统一为最多两位小数：按字符串四舍五入，去掉尾零，不用科学计数法。
 * @param {unknown} value 原始金额。
 * @returns {string} 格式化金额；非数字原样返回。
 */
export function formatAmount(value) {
    const text = String(value ?? "").trim();
    if (!/^-?\d+(?:\.\d+)?$/.test(text)) return text;
    const negative = text.startsWith("-");
    const unsigned = negative ? text.slice(1) : text;
    const [whole, fraction = ""] = unsigned.split(".");
    if (fraction.length <= 2) {
        const trimmed = fraction.replace(/0+$/, "");
        return `${negative && (whole !== "0" || trimmed) ? "-" : ""}${whole}${trimmed ? `.${trimmed}` : ""}`;
    }
    let digits = `${whole}${fraction.slice(0, 2)}`;
    if (fraction[2] >= "5") {
        const chars = digits.split("");
        let index = chars.length - 1;
        let carry = 1;
        while (index >= 0 && carry > 0) {
            const next = chars[index].charCodeAt(0) - 48 + carry;
            chars[index] = String(next % 10);
            carry = Math.floor(next / 10);
            index -= 1;
        }
        if (carry > 0) chars.unshift("1");
        digits = chars.join("");
    }
    const integerPart = digits.length > 2 ? digits.slice(0, -2) : "0";
    const fractional = digits.slice(-2).replace(/0+$/, "");
    const sign = negative && (integerPart !== "0" || fractional) ? "-" : "";
    return `${sign}${integerPart}${fractional ? `.${fractional}` : ""}`;
}

/**
 * 余额行固定两位小数；锚点仍可用精简的 formatAmount。
 * @param {unknown} value 原始金额。
 * @returns {string} 固定两位小数的金额。
 */
export function formatAmountFixed(value) {
    const compact = formatAmount(value);
    if (!/^-?\d+(?:\.\d+)?$/.test(compact)) return compact;
    const [whole, fraction = ""] = compact.split(".");
    return `${whole}.${(fraction + "00").slice(0, 2)}`;
}

/**
 * 余额读数：单钱包给币种金额，多钱包只给「余额可读」，不合并币种。
 * @param {any} balance 快照余额。
 * @returns {string} 读数；不可用时为空串。
 */
export function walletText(balance) {
    const wallets = validWallets(balance);
    if (wallets.length === 1) return `${wallets[0].currency} ${formatAmount(wallets[0].amount)}`;
    if (wallets.length > 1) return "余额可读";
    return "";
}

/**
 * 展开态逐币种行：每个钱包一行，不相加也不截断。
 * @param {any} balance 快照余额。
 * @returns {string[]} 每行 `CNY 12.34`。
 */
export function walletLines(balance) {
    return validWallets(balance).map(wallet => `${wallet.currency} ${formatAmountFixed(wallet.amount)}`);
}

/**
 * 余额不可用时的状态文案与说明；不把未知显示成 0。
 * @param {any} balance 快照余额。
 * @returns {{ text: string, detail: string }} 文案。
 */
export function balanceFallback(balance) {
    if (balance?.status === "logged-out") return { text: "账号未登录", detail: "在 DSH 登录 DeepSeek 账号" };
    if (balance?.status === "unauthorized") return { text: "余额未授权", detail: "在 DSH 检查账号授权" };
    if (balance?.status === "error") return { text: "余额读取失败", detail: "余额暂不可用" };
    return { text: "已连接", detail: "无可用余额" };
}

/**
 * 余额局部失败的可读原因；区分 401、平台错误与超时（桥只给 error 时按 stale 保守合并）。
 * @param {any} balance 快照余额。
 * @param {any} snapshot 快照。
 * @returns {string} 原因；正常时为空串。
 */
export function balanceFailureText(balance, snapshot) {
    if (balance?.status === "logged-out") return "账号未登录：在 DSH 登录 DeepSeek 账号";
    if (balance?.status === "unauthorized") return "余额读取失败：账号授权失效（401）";
    if (balance?.status === "error") {
        const reason = String(balance.reason ?? "").toLowerCase();
        if (/timeout|超时/.test(reason)) return "余额读取失败：平台请求超时";
        if (/401|unauthor/.test(reason)) return "余额读取失败：账号授权失效（401）";
        if (/platform|平台/.test(reason)) return "余额读取失败：平台返回错误";
        return snapshot?.stale ? "余额读取失败：平台错误或超时" : "余额读取失败：平台返回错误";
    }
    return "";
}

/**
 * 待决计数：明确超过 99 才显示 99+。
 * @param {number} count 待决数。
 * @returns {string} 计数文本。
 */
export function pendingCountText(count) {
    return count > 99 ? "99+" : String(count);
}

/**
 * 取快照时间戳；无法解析时返回 null。
 * @param {any} snapshot 快照。
 * @returns {number | null} 毫秒。
 */
function snapshotTime(snapshot) {
    if (!snapshot || typeof snapshot.fetchedAt !== "string") return null;
    const value = Date.parse(snapshot.fetchedAt);
    return Number.isFinite(value) ? value : null;
}

/**
 * 时间行：同日 `更新于 14:26`，跨日 `更新于 11/20 14:26`；无有效时间不冒充当前时间。
 * @param {any} snapshot 快照。
 * @param {string} prefix 前缀。
 * @param {number} now 当前毫秒。
 * @returns {string} 时间文案。
 */
export function updateTimeText(snapshot, prefix = "更新于", now = Date.now()) {
    const at = snapshotTime(snapshot);
    if (at === null) return "更新时间未知";
    const date = new Date(at);
    const current = new Date(now);
    const time = date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
    const sameDay = date.getFullYear() === current.getFullYear()
        && date.getMonth() === current.getMonth()
        && date.getDate() === current.getDate();
    return sameDay ? `${prefix} ${time}` : `${prefix} ${date.getMonth() + 1}/${date.getDate()} ${time}`;
}

/**
 * 快照时间是否仍在两个轮询周期内；不看 stale 标记。
 * @param {any} snapshot 快照。
 * @param {number} now 当前毫秒。
 * @returns {boolean} 时间是否新鲜。
 */
export function isSnapshotTimedFresh(snapshot, now = Date.now()) {
    if (!snapshot) return false;
    const at = snapshotTime(snapshot);
    if (at === null) return false;
    const age = now - at;
    return age >= -POLL_OK_MS && age <= SNAPSHOT_STALE_MS;
}

/**
 * 快照是否整体新鲜：stale 标记或时间不新鲜都算陈旧。
 * @param {any} snapshot 快照。
 * @param {number} now 当前毫秒。
 * @returns {boolean} 是否新鲜。
 */
export function isSnapshotFresh(snapshot, now = Date.now()) {
    return snapshot?.stale !== true && isSnapshotTimedFresh(snapshot, now);
}

/**
 * 待决排序：审批、计划确认、提问；同类按时间，再按 ID 稳定排序。
 * @param {any} left 待决。
 * @param {any} right 待决。
 * @returns {number} 顺序。
 */
function comparePending(left, right) {
    const priority = (PENDING_KIND_ORDER[left.kind] ?? 3) - (PENDING_KIND_ORDER[right.kind] ?? 3);
    if (priority !== 0) return priority;
    const since = String(left.since).localeCompare(String(right.since));
    return since !== 0 ? since : String(left.id).localeCompare(String(right.id));
}

/**
 * 会话标题。
 * @param {any} snapshot 快照。
 * @param {string} sessionId 会话标识。
 * @returns {string} 标题或「会话」。
 */
function sessionTitleFor(snapshot, sessionId) {
    return snapshot?.sessions?.find(row => row.id === sessionId)?.title || "会话";
}

/**
 * 最高优先待决的说明：类型 + 会话标题，按 18 字符截断。
 * @param {any} snapshot 快照。
 * @returns {string} 说明。
 */
export function topPendingDetail(snapshot) {
    const item = [...(snapshot?.pending ?? [])].sort(comparePending)[0];
    if (!item) return "等待处理";
    const label = PENDING_KIND_LABEL[item.kind] ?? "待处理";
    return shortText(`${label} · ${sessionTitleFor(snapshot, item.sessionId)}`);
}

/**
 * 截断到浮岛短文本上限（18 个 Unicode 字符）；拉丁字符也不会超过 36 的上限。
 * @param {unknown} value 文本。
 * @param {number} limit 字符上限。
 * @returns {string} 短文本。
 */
export function shortText(value, limit = ISLAND_SHORT_LIMIT) {
    const characters = Array.from(String(value ?? "").trim());
    if (characters.length <= limit) return characters.join("");
    return `${characters.slice(0, Math.max(1, limit - 1)).join("")}…`;
}

/**
 * 错误档位文案：401/换桥 → 未授权，断连 → 未连接，读失败 → 读取失败。
 * @param {string} status 实例状态。
 * @returns {{ text: string, detail: string } | null} 错误文案；非错误档返回 null。
 */
function errorTier(status, error) {
    if (status === "identity-changed") return { text: "未授权", detail: "桥身份变化，请重新配对", status: "unauthorized" };
    if (status === "unauthorized") return { text: "未授权", detail: "配对已失效，请重新配对", status: "unauthorized" };
    if (status === "unreachable") return { text: "未连接", detail: shortText(error || "确认 DSH 与桥插件在运行"), status: "disconnected" };
    if (status === "error") return { text: "读取失败", detail: shortText(error || "请稍后重试"), status: "failed" };
    return null;
}

/**
 * 锚点六档优先级视图；首档满足即返回，隐藏档 visible=false 并应显式写 null。
 * @param {any} state 实例状态。
 * @param {number} now 当前毫秒。
 * @param {{ paused?: boolean }} [options] 关闭/暂停时直接隐藏。
 * @returns {{ visible: boolean, text: string, detail: string }} 视图。
 */
export function islandAnchorView(state, now = Date.now(), options = {}) {
    const hidden = { visible: false, text: "", detail: "", status: undefined };
    if (options.paused) return hidden;
    const error = errorTier(state.status, state.error);
    if (error) return { visible: true, ...error };
    if (state.status === "unpaired" || state.status === "loading" || !state.snapshot || state.status !== "ready") return hidden;
    const snapshot = state.snapshot;
    const timedFresh = isSnapshotTimedFresh(snapshot, now);
    if (!timedFresh || snapshot.stale === true) {
        return { visible: true, text: "数据陈旧", detail: updateTimeText(snapshot, "上次更新", now), status: "paused" };
    }
    const pendingCount = snapshot.pending?.length ?? 0;
    if (pendingCount > 0) {
        return {
            visible: true,
            text: `${pendingCountText(pendingCount)} 项待决`,
            detail: topPendingDetail(snapshot),
            status: "awaiting",
        };
    }
    const wallets = walletText(snapshot.balance);
    if (wallets) {
        const single = validWallets(snapshot.balance).length === 1;
        return {
            visible: true,
            text: wallets,
            detail: single ? updateTimeText(snapshot, "更新于", now) : "多币种余额",
            status: "pending",
        };
    }
    const fallback = balanceFallback(snapshot.balance);
    const failure = balanceFailureText(snapshot.balance, snapshot);
    const status = snapshot.balance.status === "logged-out" || snapshot.balance.status === "unauthorized"
        ? "unauthorized"
        : snapshot.balance.status === "error" ? "failed" : "pending";
    return { visible: true, text: fallback.text, detail: failure || fallback.detail, status };
}

/**
 * 折叠摘要；隐藏档返回空串，调用方必须写 setIslandSummary(null)。
 * @param {any} state 实例状态。
 * @param {number} now 当前毫秒。
 * @returns {string} 摘要。
 */
export function collapsedSummary(state, now = Date.now()) {
    const view = islandAnchorView(state, now);
    return view.visible ? view.text : "";
}

/**
 * 摘要调用参数：可见时带状态与 `dsh://open` 导航；隐藏时只发 null。
 * @param {{ visible: boolean, text: string, status?: string }} view 锚点视图。
 * @returns {{ text: string, status?: string, navigation: { uri: string } } | null} 参数。
 */
export function islandSummaryArguments(view) {
    if (!view.visible) return null;
    return { text: view.text, status: view.status, navigation: { uri: ISLAND_OPEN_URI } };
}

/**
 * hover 纯展示视图：只读当前快照，不发起请求、不持有定时器。
 * @param {any} state 实例状态。
 * @param {number} [now] 当前毫秒。
 * @returns {{ main: string, sub?: string, progress?: { completed: number, total: number, percent: number, label: string }, balance?: string } | null} 视图。
 */
export function islandHoverView(state, now = Date.now()) {
    const status = state.status;
    if (status === "unpaired") return { main: "未配对", sub: "在设置 → 插件 → DSH 接入完成配对" };
    if (status === "loading") return { main: "正在读取 DSH", sub: "" };
    if (status === "identity-changed" || status === "unauthorized") return { main: "未授权", sub: "请重新配对" };
    if (status === "logged-out") return { main: "账号未登录", sub: "在 DSH 登录 DeepSeek 账号" };
    if (status === "unreachable") return { main: "未连接", sub: "请确认 DSH 桌面桥正在运行" };
    if (status === "error") return { main: "读取失败", sub: "请稍后重试" };
    const snapshot = state.snapshot;
    if (status !== "ready" || !snapshot) return { main: "正在读取 DSH", sub: "" };
    if (!isSnapshotFresh(snapshot, now)) {
        return { main: "数据陈旧", sub: updateTimeText(snapshot, "上次更新", now) };
    }
    const stats = snapshot.taskStats ?? {};
    const fetchedAt = Date.parse(snapshot.fetchedAt);
    const sampledAt = Date.parse(stats.sampledAt);
    const statsFresh = Number.isFinite(fetchedAt)
        && Number.isFinite(sampledAt)
        && Math.abs(fetchedAt - sampledAt) <= POLL_OK_MS * 2;
    const statsComplete = stats.completeness === "complete";
    const statsUsable = statsComplete && statsFresh;
    const sessionsKnown = Array.isArray(snapshot.sessions) && snapshot.hasSessions !== false;
    const visibleRunning = sessionsKnown
        ? snapshot.sessions.filter(row => row.status === "running").length
        : undefined;
    const statsRun = statsUsable && Number.isSafeInteger(stats.runCount) ? stats.runCount : undefined;
    const unread = statsUsable && Number.isSafeInteger(stats.unreadCount) ? stats.unreadCount : undefined;
    const completed = statsUsable && Number.isSafeInteger(stats.completedUnreadCount) ? stats.completedUnreadCount : undefined;
    const union = statsUsable && Number.isSafeInteger(stats.unionCount) ? stats.unionCount : undefined;
    const countsComplete = statsRun !== undefined
        && unread !== undefined
        && completed !== undefined
        && union !== undefined;
    const coherent = countsComplete
        && completed <= unread
        && unread <= union
        && statsRun <= union
        && union <= statsRun + unread;
    const noTasks = coherent && statsRun === 0 && unread === 0 && completed === 0 && union === 0;
    const main = noTasks
        ? "暂无待关注任务"
        : statsRun !== undefined
            ? `运行任务 ${statsRun} 项`
            : visibleRunning !== undefined
                ? visibleRunning === 0 ? "近期没有运行会话" : `可见会话中运行 ${visibleRunning} 项`
                : "任务状态未知";
    const sub = !statsUsable
        ? "未读状态不可用"
        : coherent
            ? `未读 ${unread} 项 · 完成未读 ${completed} 项`
            : "统计不完整";
    let progress;
    if (coherent && completed > 0 && union > 0) {
        const percent = Math.max(0, Math.min(100, Math.round(completed / union * 100)));
        progress = {
            completed,
            total: union,
            percent,
            label: `待关注任务中完成未读比例，${completed} 项，共 ${union} 项，${percent}%`,
        };
    }
    const wallets = validWallets(snapshot.balance);
    const balance = wallets.length === 1
        ? `余额 ${wallets[0].currency} ${formatAmountFixed(wallets[0].amount)}`
        : wallets.length > 1
            ? `多币种余额（${wallets.length}）`
            : snapshot.balance?.status === "logged-out"
                ? "账号未登录"
                : snapshot.balance?.status === "unauthorized" || snapshot.balance?.status === "error"
                    ? "余额不可用"
                    : "无可用余额";
    return { main, sub, progress, balance };
}

/**
 * hover 卡内容；与 renderTile 分离，只读状态并订阅现有轮询更新，不发请求。
 * @param {HTMLElement} container 宿主容器。
 * @param {any} context 插件上下文。
 * @returns {() => void} 取消订阅。
 */
export function renderIslandHover(container, context) {
    ensureStyle();
    const state = stateFor(context.instanceId);
    const paintHover = () => {
        const view = islandHoverView(state);
        container.replaceChildren();
        if (!view) return;
        const root = element("div", "plugin-dsh-status-hover");
        root.setAttribute("role", "group");
        root.setAttribute("aria-label", "DSH 任务概览");
        root.title = view.main;
        root.append(element("div", "plugin-dsh-status-hover__main", view.main));
        if (view.sub) root.append(element("div", "plugin-dsh-status-hover__sub", view.sub));
        if (view.progress) {
            const progress = element("div", "plugin-dsh-status-hover__progress");
            progress.setAttribute("role", "progressbar");
            progress.setAttribute("aria-label", view.progress.label);
            progress.setAttribute("aria-valuemin", "0");
            progress.setAttribute("aria-valuemax", "100");
            progress.setAttribute("aria-valuenow", String(view.progress.percent));
            progress.setAttribute("aria-valuetext", view.progress.label);
            const track = element("div", "plugin-dsh-status-hover__track");
            const fill = element("div", "plugin-dsh-status-hover__fill");
            fill.style.width = `${view.progress.percent}%`;
            track.append(fill);
            progress.append(track, element("span", "plugin-dsh-status-hover__progress-value", `${view.progress.percent}%`));
            root.append(progress);
        }
        if (view.balance) root.append(element("div", "plugin-dsh-status-hover__balance", view.balance));
        container.append(root);
    };
    paintHover();
    return subscribeState(state, paintHover);
}

/**
 * 订阅实例状态变化；只用于现有轮询触发的只读重绘，不创建定时器。
 * @param {any} state 实例状态。
 * @param {() => void} listener 监听函数。
 * @returns {() => void} 取消订阅。
 */
export function subscribeState(state, listener) {
    state.observers ??= new Set();
    state.observers.add(listener);
    return () => state.observers.delete(listener);
}

/**
 * 通知状态订阅者；单个监听失败不影响轮询。
 * @param {any} state 实例状态。
 * @returns {void}
 */
export function notifyState(state) {
    for (const listener of [...(state.observers ?? [])]) {
        try {
            listener();
        } catch {
            // 只读视图失败不能打断连接与轮询。
        }
    }
}

/**
 * 状态文案；展开卡的未配对/加载态仍用它。
 * @param {string} status 插件状态。
 * @returns {string} 中文文案。
 */
export function statusLabel(status) {
    if (status === "unpaired") return "未配对";
    if (status === "identity-changed") return "未授权";
    if (status === "unauthorized") return "未授权";
    if (status === "logged-out") return "账号未登录";
    if (status === "unreachable") return "未连接";
    if (status === "loading") return "正在连接";
    if (status === "ready") return "已连接";
    if (status === "error") return "读取失败";
    return "未知";
}

/**
 * 兼容旧调用的余额文本：单钱包 `CNY 12.34`，多钱包「余额可读」。
 * @param {any} balance 快照余额。
 * @returns {string} 文本。
 */
export function balanceText(balance) {
    return walletText(balance) || balanceFallback(balance).text;
}

/**
 * 把新增待决投递到浮岛消息；同一 key 只发一次，恢复时先建基线。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @param {any} snapshot 快照。
 * @returns {Array<any>} 本次新增待决。
 */
export function reconcilePending(context, state, snapshot) {
    if (snapshot.stale === true) {
        // 全局陈旧门禁：不写 seen、不投递，恢复后首帧仍走基线。
        state.needsBaseline = true;
        return [];
    }
    if (!state.baselineDone || state.needsBaseline) {
        for (const item of snapshot.pending) state.seen.add(pendingIdentity(snapshot.bridgeId, item));
        state.baselineDone = true;
        state.needsBaseline = false;
        return [];
    }
    const fresh = diffPending(state.seen, snapshot.bridgeId, snapshot.pending);
    if (typeof context.publishIslandMessage === "function") {
        for (const item of fresh) {
            const identity = pendingIdentity(snapshot.bridgeId, item);
            const title = snapshot.sessions.find(row => row.id === item.sessionId)?.title || "会话";
            const text = `DSH 有新的待决：${title}`.slice(0, 200);
            Promise.resolve(context.publishIslandMessage({
                key: pendingMessageKey(identity),
                kind: "waiting",
                text,
                uri: ISLAND_OPEN_URI,
            })).catch(() => undefined);
        }
    }
    return fresh;
}

/**
 * 格式化宿主能力错误，保留结构化错误码。
 * @param {unknown} error 错误。
 * @returns {string} 可见文本。
 */
export function formatCapabilityError(error) {
    const code = typeof error?.code === "string" && error.code ? `（${error.code}）` : "";
    return `${String(error?.message ?? error)}${code}`;
}

/**
 * 插入插件样式表；Node 测试或没有 document 时跳过。
 * @returns {void}
 */
function ensureStyle() {
    if (typeof document === "undefined") return;
    if (document.querySelector(`link[data-plugin-style="${PLUGIN_ID}"]`) !== null) return;
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = new URL("./style.css", import.meta.url).href;
    link.dataset.pluginStyle = PLUGIN_ID;
    document.head.appendChild(link);
}

/**
 * 创建一个带类名的元素。
 * @param {string} tag 标签。
 * @param {string} className 类名。
 * @param {string} [text] 文本。
 * @returns {HTMLElement} 元素。
 */
function element(tag, className, text) {
    const node = document.createElement(tag);
    node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}

/**
 * 取实例状态；首次创建时建立空基线。
 * @param {string} instanceId 实例标识。
 * @returns {any} 状态。
 */
function stateFor(instanceId) {
    let state = instanceStates.get(instanceId);
    if (state === undefined) {
        state = {
            paired: false,
            baseUrl: "",
            boundBridgeId: "",
            seen: new Set(),
            baselineDone: false,
            needsBaseline: false,
            snapshot: null,
            status: "unpaired",
            error: null,
            healthChannelReady: false,
            bridgeChannelReady: false,
            inFlight: false,
            opening: false,
            dismissedPending: new Set(),
            observers: new Set(),
            stopPolling: false,
            mountEpoch: 0,
            mountSequence: 0,
            endpointEpoch: 0,
            paintTarget: null,
            polling: null,
        };
        instanceStates.set(instanceId, state);
    }
    return state;
}

/**
 * 每次挂载都重建提醒基线：首份成功快照只对账，不补播历史待决。
 * @param {any} state 实例状态。
 * @returns {void}
 */
export function beginMount(state) {
    state.seen = new Set();
    state.baselineDone = false;
    state.needsBaseline = true;
}

/**
 * 是否真正的新挂载；已有轮询会话的折叠/展开切换不重建基线。
 * @param {any} state 实例状态。
 * @returns {boolean} 是否需要 beginMount。
 */
export function mountNeedsBaseline(state) {
    return state.polling === null && state.paintTarget === null;
}

/**
 * 读取实例当前快照时间；无法解析时返回 null。
 * @param {any} snapshot 快照。
 * @returns {number | null} 毫秒。
 */
function fetchedAtMs(snapshot) {
    if (!snapshot || typeof snapshot.fetchedAt !== "string") return null;
    const value = Date.parse(snapshot.fetchedAt);
    return Number.isFinite(value) ? value : null;
}

/**
 * 快照是否已经陈旧（超过两个正常轮询周期）。
 * @param {any} snapshot 快照。
 * @returns {boolean} 是否陈旧。
 */
function isStale(snapshot) {
    const at = fetchedAtMs(snapshot);
    return at === null ? true : Date.now() - at > POLL_OK_MS * 2;
}

/**
 * 注册一条 loopback 通道；实例身份由宿主绑定，插件不传。
 * @param {any} context 插件上下文。
 * @param {Record<string, unknown>} registration 注册参数。
 * @returns {Promise<void>} 完成。
 */
async function registerChannel(context, registration) {
    if (typeof context.registerLoopbackChannel !== "function") throw new Error("插件未获授权 loopback-channel");
    await context.registerLoopbackChannel(registration);
}

/**
 * 经通道发一次请求。
 * @param {any} context 插件上下文。
 * @param {string} channelId 通道。
 * @param {string} method 方法。
 * @param {string} path 路径。
 * @returns {Promise<any>} `{ status, headers, body }`。
 */
async function requestChannel(context, channelId, method, path, baseUrl) {
    if (typeof context.requestLoopback !== "function") throw new Error("插件未获授权 loopback-channel");
    const key = `${context.instanceId}|${baseUrl}|${method}:${path}`;
    const existing = inFlightRequests.get(key);
    if (existing) return existing;
    const request = Promise.resolve().then(() => context.requestLoopback(channelId, method, path));
    inFlightRequests.set(key, request);
    request.then(
        () => {
            if (inFlightRequests.get(key) === request) inFlightRequests.delete(key);
        },
        () => {
            if (inFlightRequests.get(key) === request) inFlightRequests.delete(key);
        },
    );
    return request;
}

/** 代次失效时抛出的结构化错误。 */
function staleResultError() {
    return Object.assign(new Error("本轮请求已过期"), { code: "stale-result" });
}

/** 桥稳定身份与持久绑定不一致时抛出的结构化错误。 */
function bridgeIdentityChangedError() {
    return Object.assign(new Error("桥身份变化，请重新配对"), { code: "bridge-identity-changed" });
}

/**
 * 解析 health 响应；缺少稳定身份视为无效。
 * @param {any} response loopback 响应。
 * @returns {{ bridgeId: string, bridgeInstanceId: string }} 身份。
 */
function parseHealthResponse(response) {
    if (response?.status !== 200) throw new Error(`健康检查失败：HTTP ${response?.status ?? "未知"}`);
    const health = typeof response.body === "string" ? JSON.parse(response.body) : response.body;
    if (!health || health.ok !== true || typeof health.bridgeId !== "string" || typeof health.bridgeInstanceId !== "string") {
        throw Object.assign(new Error("桥健康检查缺少稳定身份"), { code: "bridge-identity-invalid" });
    }
    return { bridgeId: health.bridgeId, bridgeInstanceId: health.bridgeInstanceId };
}

/**
 * 用无凭据通道读取 health；供配对时记录稳定桥身份。
 * @param {any} context 插件上下文。
 * @param {string} baseUrl 桥地址。
 * @returns {Promise<{ bridgeId: string, bridgeInstanceId: string }>} 身份。
 */
export async function readBridgeHealth(context, baseUrl) {
    await registerChannel(context, { channelId: HEALTH_CHANNEL, mode: "request", baseUrl });
    const response = await requestChannel(context, HEALTH_CHANNEL, "GET", "/health", baseUrl);
    return parseHealthResponse(response);
}

/**
 * 先无凭据读 /health 确认桥身份，再用带凭据通道读 /snapshot；每次 await 后校验代次。
 * 携密前要求 health.bridgeId 与持久绑定一致，不一致时不发旧令牌。
 * @param {any} context 插件上下文。
 * @param {string} baseUrl 桥地址。
 * @param {any} state 实例状态（读写通道就绪标记与持久绑定）。
 * @param {() => boolean} guard 本轮结果是否仍有效。
 * @returns {Promise<any>} 快照。
 */
export async function fetchBridgeSnapshot(context, baseUrl, state, guard = () => true) {
    const ensure = () => {
        if (!guard()) throw staleResultError();
    };
    if (!state.healthChannelReady) {
        await registerChannel(context, { channelId: HEALTH_CHANNEL, mode: "request", baseUrl });
        ensure();
        state.healthChannelReady = true;
    }
    const healthResponse = await requestChannel(context, HEALTH_CHANNEL, "GET", "/health", baseUrl);
    ensure();
    const health = parseHealthResponse(healthResponse);
    if (typeof state.boundBridgeId !== "string" || state.boundBridgeId.length === 0) throw bridgeIdentityChangedError();
    if (health.bridgeId !== state.boundBridgeId) throw bridgeIdentityChangedError();
    if (!state.bridgeChannelReady) {
        await registerChannel(context, {
            channelId: BRIDGE_CHANNEL,
            mode: "request",
            baseUrl,
            credentialRef: TOKEN_REF,
            headerName: "Authorization",
            credentialPrefix: TOKEN_PREFIX,
        });
        ensure();
        state.bridgeChannelReady = true;
    }
    const snapshotResponse = await requestChannel(context, BRIDGE_CHANNEL, "GET", "/snapshot", baseUrl);
    ensure();
    if (snapshotResponse?.status === 401) throw Object.assign(new Error("配对令牌已被拒绝"), { code: "unauthorized" });
    if (snapshotResponse?.status !== 200) throw new Error(`快照读取失败：HTTP ${snapshotResponse?.status ?? "未知"}`);
    const snapshot = parseSnapshotPayload(snapshotResponse.body);
    if (snapshot.bridgeId !== health.bridgeId || snapshot.bridgeInstanceId !== health.bridgeInstanceId) {
        throw Object.assign(new Error("快照与本轮健康身份不一致"), { code: "bridge-identity-mismatch" });
    }
    return snapshot;
}

/**
 * 提交一份成功快照：先更新展示事实，全局新鲜时才允许对账与投递。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @param {any} snapshot 快照。
 * @returns {{ fresh: boolean, delivered: Array<any> }} 是否新鲜及投递项。
 */
export function applySnapshotFacts(context, state, snapshot) {
    state.snapshot = snapshot;
    state.error = null;
    state.status = "ready";
    if (snapshot.stale === true) {
        // 整份不可信：只上屏陈旧状态，不写 seen、不发提醒；恢复首帧仍按基线。
        state.needsBaseline = true;
        return { fresh: false, delivered: [] };
    }
    // 本地移除只维持到下一次可信快照：已消失的条目清掉标记，重新出现时恢复展示。
    state.dismissedPending ??= new Set();
    const present = new Set(snapshot.pending.map(item => pendingIdentity(snapshot.bridgeId, item)));
    for (const identity of [...state.dismissedPending]) {
        if (!present.has(identity)) state.dismissedPending.delete(identity);
    }
    return { fresh: true, delivered: reconcilePending(context, state, snapshot) };
}

/**
 * 同步把身份变化标记为未授权并清空旧事实；调用方应立即 paint。
 * @param {any} state 实例状态。
 * @returns {void}
 */
export function markBridgeIdentityChanged(state) {
    state.error = "桥身份变化，请重新配对";
    state.status = "identity-changed";
    state.stopPolling = true;
    state.snapshot = null;
    state.needsBaseline = true;
}

/**
 * 身份变化先上屏、再异步清理通道；清理失败只记日志，不回滚状态。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @param {string} baseUrl 桥地址。
 * @param {() => void} repaint 同步重绘回调。
 * @returns {void}
 */
export function applyBridgeIdentityChanged(context, state, baseUrl, repaint = () => undefined) {
    markBridgeIdentityChanged(state);
    repaint();
    void invalidateBridgeChannel(context, baseUrl).catch(error => {
        console.warn("DSH 插件：旧桥通道清理失败", error);
    });
}

/**
 * 轮询一次并提交状态；旧代次结果在提交前丢弃。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @param {string} baseUrl 桥地址。
 * @param {() => boolean} guard 本轮是否仍有效。
 * @returns {Promise<boolean>} 本轮是否成功。
 */
async function pollOnce(context, state, baseUrl, guard) {
    try {
        const snapshot = await fetchBridgeSnapshot(context, baseUrl, state, guard);
        if (!guard()) return false;
        applySnapshotFacts(context, state, snapshot);
        return true;
    } catch (error) {
        if (!guard()) return false;
        state.healthChannelReady = false;
        state.bridgeChannelReady = false;
        state.needsBaseline = true;
        if (error?.code === "bridge-identity-changed") {
            const target = state.paintTarget;
            applyBridgeIdentityChanged(context, state, baseUrl, () => {
                if (target) paint(target.container, state, target.context);
            });
            return false;
        }
        state.error = formatCapabilityError(error);
        state.status = error?.code === "unauthorized"
            ? "unauthorized"
            : /凭据|credential|token/i.test(state.error)
                ? "unpaired"
                : /对端|连接|通道|宿主桥|超时|未响应|request/i.test(state.error)
                    ? "unreachable"
                    : "error";
        if (state.status === "unauthorized") state.snapshot = null;
        return false;
    }
}

/**
 * 会话聚合行；空快照事实写成明确的空，不用 0 冒充未知。
 * @param {any} snapshot 快照。
 * @returns {string} 摘要。
 */
function sessionActivityText(snapshot) {
    if (snapshot.hasSessions === false) return "会话状态未知";
    const running = snapshot.sessions.filter(row => row.status === "running").length;
    return snapshot.sessions.length > 0 ? `${running} 个会话运行` : "没有运行中的会话";
}

/**
 * 任务聚合行。
 * @param {any} snapshot 快照。
 * @returns {string} 文案。
 */
function jobActivityText(snapshot) {
    if (snapshot.hasJobs === false) return "任务状态未知";
    const running = snapshot.jobs.filter(row => row.status === "running").length;
    return snapshot.jobs.length > 0 ? `${running} 个任务运行` : "没有运行中的任务";
}

/**
 * 会话/任务单行摘要；空快照事实写成明确的空。
 * @param {any} snapshot 快照。
 * @returns {string} 摘要。
 */
function activitySummary(snapshot) {
    return `${sessionActivityText(snapshot)} · ${jobActivityText(snapshot)}`;
}

/** 正在运行的标题滚动控制器；切换条目、折叠分组或重绘时统一复位。 */
const activeTextScrolls = new Set();

/**
 * 计算长标题往返滚动计划；减少动效或距离不足时返回 null。
 * @param {number} distance 实际溢出像素。
 * @param {{ reducedMotion?: boolean, speed?: number, minCycleMs?: number, holdMs?: number }} [options] 参数。
 * @returns {{ distance: number, travelMs: number, holdMs: number, cycleMs: number } | null} 计划。
 */
export function textScrollPlan(distance, options = {}) {
    const reducedMotion = options.reducedMotion === true;
    const speed = options.speed ?? 20; // DIP/s。
    const minCycleMs = options.minCycleMs ?? 4000;
    const initialHoldMs = options.holdMs ?? 600;
    if (reducedMotion || !Number.isFinite(distance) || distance < 4) return null;
    const travelMs = Math.round(distance / speed * 1000);
    const cycleMs = Math.max(minCycleMs, travelMs * 2 + initialHoldMs * 2);
    const holdMs = (cycleMs - travelMs * 2) / 2;
    return { distance: Math.round(distance), travelMs, holdMs, cycleMs };
}

/**
 * 取消所有标题滚动并复位到起点；动画停止并清除内联变换。
 * @returns {void}
 */
export function resetTextScrolls() {
    for (const controller of [...activeTextScrolls]) controller.cancel();
}

/**
 * 是否要求减少动效；无 matchMedia 的环境按不减少处理。
 * @returns {boolean} 是否减少动效。
 */
export function reducedMotionPreferred() {
    return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches === true;
}

/**
 * 给超长标题行挂接 hover 往返滚动；离开/切换/折叠时复位，减少动效时保持省略号。
 * @param {HTMLElement} row 行元素。
 * @param {HTMLElement} box 标题盒。
 * @param {HTMLElement} text 可位移文本。
 * @returns {void}
 */
function attachTitleScroll(row, box, text) {
    let controller = null;
    const stop = () => {
        if (controller) {
            controller.cancel();
            controller = null;
        }
    };
    row.addEventListener("pointerenter", () => {
        if (controller || reducedMotionPreferred()) return;
        const distance = Math.round(Math.max(text.scrollWidth, text.getBoundingClientRect().width) - box.clientWidth);
        const plan = textScrollPlan(distance);
        if (!plan) return;
        resetTextScrolls();
        const state = { animation: null };
        controller = {
            cancel: () => {
                state.animation?.cancel?.();
                state.animation = null;
                text.style.transform = "";
                activeTextScrolls.delete(controller);
            },
        };
        activeTextScrolls.add(controller);
        if (typeof text.animate !== "function") {
            controller.cancel();
            controller = null;
            return;
        }
        const pause = plan.holdMs / plan.cycleMs;
        const atEnd = (plan.holdMs + plan.travelMs) / plan.cycleMs;
        try {
            // 首段 600ms 停顿由动画的起点保持承担，不叠加额外的启动定时器。
            state.animation = text.animate([
                { transform: "translateX(0)", offset: 0 },
                { transform: "translateX(0)", offset: pause },
                { transform: `translateX(-${plan.distance}px)`, offset: atEnd },
                { transform: `translateX(-${plan.distance}px)`, offset: atEnd + pause },
                { transform: "translateX(0)", offset: 1 },
            ], {
                duration: plan.cycleMs,
                iterations: Infinity,
                easing: "linear",
                fill: "both",
            });
        } catch {
            controller?.cancel();
            controller = null;
        }
    });
    row.addEventListener("pointerleave", stop);
    row.addEventListener("pointercancel", stop);
}

/**
 * 渲染展开卡的主体：标题、主行、余额、会话/任务摘要、待决与独立时间行。
 * @param {HTMLElement} root 根节点。
 * @param {any} state 实例状态。
 * @param {any} context 插件上下文。
 * @param {{ visible: boolean, text: string, detail: string, status?: string }} view 锚点视图。
 * @param {number} now 当前毫秒。
 * @param {HTMLElement} container 宿主容器。
 * @returns {void}
 */
function paintExpanded(root, state, context, view, now, container) {
    const snapshot = state.snapshot;
    const head = element("div", "plugin-dsh-status__head");
    head.append(element("span", "plugin-dsh-status__title", "DSH 接入"));
    root.append(head);

    const status = state.status;
    const error = errorTier(status, state.error);
    const timedFresh = isSnapshotTimedFresh(snapshot, now);
    const fresh = status === "ready" && timedFresh && snapshot?.stale !== true;
    const main = element("div", `plugin-dsh-status__main plugin-dsh-status__main--${status}`);
    const mainWallets = snapshot ? walletText(snapshot.balance) : "";
    const noBalance = fresh && (snapshot?.pending?.length ?? 0) === 0 && !mainWallets && snapshot?.balance?.status === "ready";
    const mainText = element("span", "plugin-dsh-status__main-text", noBalance
        ? "无可用余额"
        : view.visible ? view.text : statusLabel(status));
    if (fresh && mainWallets && view.text === mainWallets) mainText.classList.add("plugin-dsh-status__main-text--amount");
    main.append(mainText);
    root.append(main);

    if (status === "unpaired" || status === "loading") {
        root.append(element("p", "plugin-dsh-status__hint", status === "unpaired"
            ? "在设置 → 插件 → DSH 接入完成配对"
            : "正在读取 DSH"));
        return;
    }
    if (error) {
        root.append(element("p", "plugin-dsh-status__hint", error.detail));
        if (snapshot) root.append(element("div", "plugin-dsh-status__time", updateTimeText(snapshot, "上次更新", now)));
        return;
    }
    if (!snapshot) {
        root.append(element("p", "plugin-dsh-status__hint", "无可用事实"));
        return;
    }
    if (!fresh) {
        const historical = snapshot.pending?.length ?? 0;
        root.append(element("p", "plugin-dsh-status__hint", updateTimeText(snapshot, "上次更新", now)));
        if (historical > 0) root.append(element("div", "plugin-dsh-status__history", `上次有 ${pendingCountText(historical)} 项待决`));
        return;
    }

    const pendingItems = snapshot.pending
        .filter(item => !(state.dismissedPending?.has(pendingIdentity(snapshot.bridgeId, item)) ?? false))
        .sort(comparePending);
    const walletRows = walletLines(snapshot.balance);
    if (walletRows.length === 1) {
        if (view.text !== walletText(snapshot.balance)) {
            root.append(element("div", "plugin-dsh-status__balance", `余额 ${walletRows[0]}`));
        }
    } else if (walletRows.length > 1) {
        for (const row of walletRows) {
            root.append(element("div", "plugin-dsh-status__balance", `余额 ${row}`));
        }
    }
    if (snapshot.balance.status !== "ready") {
        const reason = balanceFailureText(snapshot.balance, snapshot);
        if (reason) root.append(element("p", "plugin-dsh-status__balance-reason", reason));
    }
    root.append(element("div", "plugin-dsh-status__summary", activitySummary(snapshot)));
    root.append(element("div", "plugin-dsh-status__divider"));
    if (pendingItems.length === 0) {
        root.append(element("div", "plugin-dsh-status__empty", "没有待决"));
    } else {
        const list = element("div", "plugin-dsh-status__pending");
        for (const item of pendingItems.slice(0, 5)) {
            const title = sessionTitleFor(snapshot, item.sessionId);
            const kind = PENDING_KIND_LABEL[item.kind] ?? "待处理";
            const row = element("button", "plugin-dsh-status__pending-row");
            row.type = "button";
            row.dataset.islandNoDrag = "true";
            row.setAttribute("aria-label", `${title}，${kind}，打开 DSH`);
            const titleBox = element("span", "plugin-dsh-status__pending-title");
            const titleText = element("span", "plugin-dsh-status__pending-title-text", title);
            titleBox.append(titleText);
            row.append(element("span", "plugin-dsh-status__pending-mark"));
            row.append(titleBox);
            row.append(element("span", "plugin-dsh-status__pending-kind", kind));
            row.append(element("span", "plugin-dsh-status__pending-go", "›"));
            attachTitleScroll(row, titleBox, titleText);
            row.addEventListener("click", () => {
                resetTextScrolls();
                void openPendingRow(context, state, item, row);
            });
            list.append(row);
        }
        root.append(list);
        if (pendingItems.length > 5) {
            const more = element("button", "plugin-dsh-status__more", `还有 ${pendingItems.length - 5} 项，在 DSH 中查看`);
            more.type = "button";
            more.dataset.islandNoDrag = "true";
            more.addEventListener("click", () => {
                resetTextScrolls();
                void openDshUri(context, state);
            });
            root.append(more);
        }
    }
    root.append(element("div", "plugin-dsh-status__time", updateTimeText(snapshot, "更新于", now)));
}

/**
 * 渲染卡片；折叠时只输出一行，展开时按线框重排。
 * @param {HTMLElement} container 宿主容器。
 * @param {any} state 实例状态。
 * @param {any} context 插件上下文。
 * @returns {void}
 */
function paint(container, state, context) {
    ensureStyle();
    const now = Date.now();
    const paused = !shouldPoll() || context.islandPaused === true || context.islandRunning === false;
    const view = islandAnchorView(state, now, { paused });
    if (typeof context.setIslandSummary === "function") {
        const summary = islandSummaryArguments(view);
        if (summary === null) context.setIslandSummary(null);
        else context.setIslandSummary(summary.text, summary.status, summary.navigation);
    }
    resetTextScrolls();
    container.replaceChildren();
    const collapsed = context.collapsed === true;
    const root = element("div", `plugin-dsh-status ${collapsed ? "plugin-dsh-status--collapsed" : "plugin-dsh-status--expanded"}`);
    root.title = view.visible ? `${view.text} · ${view.detail}` : statusLabel(state.status);
    if (collapsed) {
        if (view.visible) root.append(element("span", "plugin-dsh-status__collapsed-text", view.text));
    } else {
        paintExpanded(root, state, context, view, now, container);
    }
    container.append(root);
    notifyState(state);
}

/**
 * 打开 DSH 的单一入口：同一次打开只发一次 openUri；失败静默并记一次 warn。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @returns {Promise<boolean>} 是否成功发起。
 */
export async function openDshUri(context, state) {
    if (state.opening === true) return false;
    state.opening = true;
    try {
        if (typeof context.openUri !== "function") throw new Error("插件未获授权打开链接");
        await context.openUri(ISLAND_OPEN_URI);
        return true;
    } catch (error) {
        console.warn("DSH 插件：打开 DSH 失败", error);
        return false;
    } finally {
        state.opening = false;
    }
}

/**
 * 点击待决行：成功本地移除该行，失败保留；不写提示、不整卡重绘。
 * @param {any} context 插件上下文。
 * @param {any} state 实例状态。
 * @param {{ id: string, since: string }} item 待决。
 * @param {HTMLElement} [row] 行元素。
 * @returns {Promise<boolean>} 是否移除。
 */
export async function openPendingRow(context, state, item, row) {
    const bridgeId = state.snapshot?.bridgeId;
    if (!bridgeId) return false;
    const identity = pendingIdentity(bridgeId, item);
    const opened = await openDshUri(context, state);
    if (!opened) return false;
    state.dismissedPending ??= new Set();
    state.dismissedPending.add(identity);
    row?.remove?.();
    return true;
}

/**
 * 停止实例的唯一轮询会话；不触碰基线/去重，供端点变化或真正卸载使用。
 * @param {any} state 实例状态。
 * @returns {void}
 */
function stopPollingSession(state) {
    const session = state.polling;
    if (!session) return;
    session.stopped = true;
    if (session.timer !== null) {
        clearTimeout(session.timer);
        session.timer = null;
    }
    session.documentRef?.removeEventListener?.("visibilitychange", session.onVisibility);
    if (state.polling === session) state.polling = null;
}

/**
 * 创建实例唯一的轮询会话；折叠/展开只更新挂载目标，不重建会话。
 * @param {any} state 实例状态。
 * @param {any} context 当前插件上下文。
 * @param {string} baseUrl 归一桥地址。
 * @returns {void}
 */
function startPollingSession(state, context, baseUrl, mountSequence) {
    const documentRef = typeof document === "undefined" ? undefined : document;
    const session = {
        stopped: false,
        timer: null,
        inFlight: false,
        context,
        baseUrl,
        mountSequence,
        endpointEpoch: state.endpointEpoch,
        documentRef,
        onVisibility: null,
    };
    state.polling = session;
    const guard = () => !session.stopped
        && state.polling === session
        && state.paired
        && !state.stopPolling
        && state.endpointEpoch === session.endpointEpoch
        && state.mountSequence === session.mountSequence
        && shouldPoll();
    const scheduleDelay = ms => {
        if (!guard()) return;
        if (session.timer !== null) clearTimeout(session.timer);
        session.timer = setTimeout(() => {
            session.timer = null;
            void poll();
        }, ms);
    };
    const schedule = ok => scheduleDelay(nextPollDelay(ok));
    const poll = async () => {
        if (!guard()) return;
        if (session.inFlight) return;
        const requestSequence = session.mountSequence;
        const requestGuard = () => guard() && session.mountSequence === requestSequence;
        session.inFlight = true;
        try {
            const ok = await pollOnce(session.context, state, baseUrl, requestGuard);
            if (session.stopped || state.polling !== session || state.endpointEpoch !== session.endpointEpoch) return;
            // 旧挂载的在途结果只丢弃，不缓存、不发提醒；让当前序号补一次请求。
            if (state.mountSequence !== requestSequence) return;
            if (!shouldPoll()) return;
            const target = state.paintTarget;
            if (target) paint(target.container, state, target.context);
            if (!state.stopPolling) schedule(ok);
        } finally {
            session.inFlight = false;
            if (state.polling === session && state.mountSequence !== requestSequence) session.wake?.();
        }
    };
    const wake = immediate => {
        if (!guard() || session.inFlight) return;
        if (immediate) {
            if (session.timer !== null) {
                clearTimeout(session.timer);
                session.timer = null;
            }
            scheduleDelay(0);
        } else if (session.timer === null) {
            scheduleDelay(0);
        }
    };
    session.wake = wake;
    session.onVisibility = () => {
        if (session.stopped) return;
        if (shouldPoll()) {
            if (state.stopPolling) {
                const target = state.paintTarget;
                if (target) paint(target.container, state, target.context);
                return;
            }
            // 恢复时先建基线：本次快照只对账，不补播隐藏期间的待决。
            state.needsBaseline = true;
            scheduleDelay(0);
        } else {
            state.needsBaseline = true;
            if (session.timer !== null) {
                clearTimeout(session.timer);
                session.timer = null;
            }
            if (typeof session.context.setIslandSummary === "function") session.context.setIslandSummary(null);
        }
    };
    documentRef?.addEventListener?.("visibilitychange", session.onVisibility);
    if (shouldPoll()) void poll();
}

/**
 * 渲染浮岛卡；轮询会话跨折叠/展开保留，只更新挂载目标。
 * @param {HTMLElement} container 宿主容器。
 * @param {any} context 插件上下文。
 * @returns {() => void} 清理函数。
 */
export function renderTile(container, context) {
    ensureStyle();
    const settings = normalizeSettings(context.settings);
    const state = stateFor(context.instanceId);
    const mountSequence = ++state.mountSequence;
    const baseUrl = (() => {
        try {
            return normalizeBaseUrl(settings.baseUrl);
        } catch {
            return DEFAULT_BASE_URL;
        }
    })();
    const endpointChanged = state.paired !== settings.paired
        || state.baseUrl !== baseUrl
        || state.boundBridgeId !== boundBridgeIdFor(settings, baseUrl);
    if (endpointChanged) {
        stopPollingSession(state);
        state.endpointEpoch += 1;
        state.paired = settings.paired;
        state.baseUrl = baseUrl;
        // 绑定只对写入时的地址有效；地址变了必须重新配对，不能沿用旧 token。
        state.boundBridgeId = boundBridgeIdFor(settings, baseUrl);
        state.seen = new Set();
        state.dismissedPending = new Set();
        state.baselineDone = false;
        state.needsBaseline = true;
        state.snapshot = null;
        state.status = settings.paired ? "loading" : "unpaired";
        state.error = null;
        state.healthChannelReady = false;
        state.bridgeChannelReady = false;
        state.stopPolling = false;
    }
    const firstMount = mountNeedsBaseline(state);
    if (firstMount) beginMount(state);
    if (settings.paired && state.boundBridgeId.length === 0) {
        state.status = "identity-changed";
        state.error = "桥身份变化，请重新配对";
        state.stopPolling = true;
        state.snapshot = null;
    }
    state.paintTarget = { container, context };
    if (state.polling) {
        state.polling.context = context;
        state.polling.mountSequence = mountSequence;
    }
    paint(container, state, context);
    if (!settings.paired || state.stopPolling) {
        stopPollingSession(state);
        return () => {
            if (state.paintTarget?.container === container) state.paintTarget = null;
        };
    }
    if (!state.polling) startPollingSession(state, context, baseUrl, mountSequence);
    else state.polling.wake?.(false);
    return () => {
        if (state.paintTarget?.container !== container) return;
        state.paintTarget = null;
        const session = state.polling;
        // 折叠/展开的清理与下一次挂载在同一提交内；微任务后才算真正卸载。
        Promise.resolve().then(() => {
            if (state.paintTarget !== null || state.polling !== session) return;
            stopPollingSession(state);
            if (typeof context.setIslandSummary === "function") context.setIslandSummary(null);
        });
    };
}

/**
 * 用一条无凭据通道替换旧快照通道，丢掉宿主缓存的旧 token。
 * @param {any} context 插件上下文。
 * @param {string} baseUrl 桥地址。
 * @returns {Promise<void>} 完成。
 */
async function invalidateBridgeChannel(context, baseUrl) {
    if (typeof context.registerLoopbackChannel !== "function") return;
    try {
        await context.registerLoopbackChannel({ channelId: BRIDGE_CHANNEL, mode: "request", baseUrl });
    } catch (error) {
        // 清理失败只记日志，已上屏的未授权状态不回滚。
        console.warn("DSH 插件：旧桥通道清理失败", error);
    }
}

/**
 * 宿主兑换配对：响应里的秘密由宿主直接落槽，插件只拿脱敏结果。
 * @param {any} context 插件上下文。
 * @param {string} baseUrl 桥地址。
 * @param {string} code 用户输入的配对码。
 * @returns {Promise<{ stored: true, bridgeId: string }>} 脱敏结果与稳定桥身份。
 */
export async function pairWithBridge(context, baseUrl, code) {
    if (typeof context.exchangeCredential !== "function") throw new Error("插件未获授权 plugin-credential-exchange");
    const normalized = normalizePairingCode(code);
    const clearToken = async () => {
        if (typeof context.clearCredential !== "function") return;
        try {
            await context.clearCredential(TOKEN_SLOT);
        } catch {
            // 清理失败由后续重新配对覆盖。
        }
    };
    let before;
    try {
        before = await readBridgeHealth(context, baseUrl);
    } catch (error) {
        throw Object.assign(new Error("配对前无法确认桥身份，请确认桥已启动"), {
            code: "bridge-identity-unconfirmed",
            cause: error,
        });
    }
    const result = await context.exchangeCredential(TOKEN_SLOT, {
        url: `${baseUrl}/pair`,
        body: { code: normalized },
        extract: "token",
    });
    if (!result || result.stored !== true) throw new Error("配对未完成：宿主没有确认凭据已保存");
    let after;
    try {
        // 兑换前后都必须指向同一稳定桥，避免把刚写入的令牌绑到换过的桥。
        after = await readBridgeHealth(context, baseUrl);
    } catch (error) {
        await clearToken();
        throw Object.assign(new Error("配对已写入但无法确认桥身份，请重试"), {
            code: "bridge-identity-unconfirmed",
            cause: error,
        });
    }
    if (after.bridgeId !== before.bridgeId) {
        await clearToken();
        throw Object.assign(new Error("桥身份在配对过程中变化，请重试"), { code: "bridge-identity-changed" });
    }
    return { stored: true, bridgeId: after.bridgeId };
}

/**
 * 渲染「DSH 桥地址」设置条目。
 * @param {HTMLElement} container 宿主容器。
 * @param {any} context 插件上下文。
 * @returns {void}
 */
function renderAddressEntry(container, context) {
    ensureStyle();
    const settings = normalizeSettings(context.settings);
    container.replaceChildren();
    const root = element("div", "plugin-dsh-status-settings");
    const label = element("label", "plugin-dsh-status-settings__label", "DSH 桥地址");
    const input = document.createElement("input");
    input.className = "plugin-dsh-status-settings__input";
    input.type = "url";
    input.value = settings.baseUrl;
    input.placeholder = DEFAULT_BASE_URL;
    const save = element("button", "plugin-dsh-status-settings__button", "保存");
    save.type = "button";
    const status = element("p", "plugin-dsh-status-settings__status", "");
    save.addEventListener("click", () => {
        void (async () => {
            try {
                const baseUrl = normalizeBaseUrl(input.value);
                const previous = (() => {
                    try {
                        return normalizeBaseUrl(settings.baseUrl);
                    } catch {
                        return DEFAULT_BASE_URL;
                    }
                })();
                const sameEndpoint = baseUrl === previous;
                if (!sameEndpoint) {
                    if (typeof context.clearCredential === "function") await context.clearCredential(TOKEN_SLOT);
                    await invalidateBridgeChannel(context, previous);
                }
                await context.saveSettings({
                    ...settings,
                    baseUrl,
                    paired: sameEndpoint ? settings.paired : false,
                    bridgeId: sameEndpoint ? settings.bridgeId : "",
                    bindingBaseUrl: sameEndpoint ? settings.bindingBaseUrl : "",
                });
                status.textContent = "已保存";
            } catch (error) {
                status.textContent = formatCapabilityError(error);
            }
        })();
    });
    label.append(input);
    root.append(label, save, status);
    container.append(root);
}

/**
 * 渲染「配对码」设置条目。
 * @param {HTMLElement} container 宿主容器。
 * @param {any} context 插件上下文。
 * @returns {void}
 */
function renderPairingEntry(container, context) {
    ensureStyle();
    const settings = normalizeSettings(context.settings);
    container.replaceChildren();
    const root = element("div", "plugin-dsh-status-settings");
    const status = element("p", "plugin-dsh-status-settings__status", settings.paired ? "已配对" : "未配对");
    const input = document.createElement("input");
    input.className = "plugin-dsh-status-settings__input";
    input.type = "text";
    input.autocomplete = "off";
    input.placeholder = "ABCDE-FGHIJ-KLMNO-PQRST";
    const pair = element("button", "plugin-dsh-status-settings__button", "配对");
    pair.type = "button";
    const cancel = element("button", "plugin-dsh-status-settings__button plugin-dsh-status-settings__button--secondary", "取消配对");
    cancel.type = "button";
    pair.addEventListener("click", () => {
        void (async () => {
            pair.disabled = true;
            status.textContent = "正在配对…";
            try {
                const baseUrl = normalizeBaseUrl(settings.baseUrl);
                const result = await pairWithBridge(context, baseUrl, input.value);
                await context.saveSettings({ ...settings, baseUrl, paired: true, bridgeId: result.bridgeId, bindingBaseUrl: baseUrl });
                status.textContent = "配对成功";
                input.value = "";
            } catch (error) {
                status.textContent = formatCapabilityError(error);
            } finally {
                pair.disabled = false;
            }
        })();
    });
    cancel.addEventListener("click", () => {
        void (async () => {
            try {
                if (typeof context.clearCredential === "function") await context.clearCredential(TOKEN_SLOT);
                const baseUrl = (() => {
                    try {
                        return normalizeBaseUrl(settings.baseUrl);
                    } catch {
                        return DEFAULT_BASE_URL;
                    }
                })();
                await invalidateBridgeChannel(context, baseUrl);
                await context.saveSettings({ ...settings, paired: false, bridgeId: "", bindingBaseUrl: "" });
                status.textContent = "已取消配对";
            } catch (error) {
                status.textContent = formatCapabilityError(error);
            }
        })();
    });
    root.append(input, pair, cancel, status);
    container.append(root);
}

/** 设置页条目：桥地址与配对码。 */
export const settingsEntries = [
    { id: "dsh-bridge-address", name: "DSH 桥地址", render: renderAddressEntry },
    { id: "dsh-pairing", name: "配对码", render: renderPairingEntry },
];

export default { renderTile, renderIslandHover, settingsEntries };
