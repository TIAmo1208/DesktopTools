import { ISLAND_ANCHOR_KEYS, islandAnchorVisible, type ApplicationPreferences } from "../../contracts/settings.ts";

/**
 * 浮岛内容层纯模型：锚点身份与顺序、十种基础状态、叠加分层与接力节奏、hover 选中、
 * 折叠与空态、拖放接收。渲染层只消费这里的结果，不自己判定语义；
 * 模块不依赖 React、不读 DOM、不持有定时器，时间一律由调用方传入。
 */

/** 锚点绘制区域边长（DIP）；圆形只作蒙版，`20 × 20` 安全区由内容与视觉层保证。 */
export const ISLAND_ANCHOR_SIZE = 28;
/** 相邻锚点之间的间距（DIP），沿贴边方向。 */
export const ISLAND_ANCHOR_GAP = 12;
/** 胶囊最多并排的锚点数（沿贴边方向）；第 4 枚起折成一枚「＋N」圆。 */
export const ISLAND_ANCHOR_SLOTS = 3;
/** 通知条目的保留时长，由 islandMessages 的合并投影消费；确认和失败不超时。 */
export const ISLAND_MESSAGE_ANCHOR_TTL_MS = 30_000;
/** 锚点插入与内容展开的时长，与渲染层 WAAPI、island.css 的 `--island-anchor-grow` 共用；取生长节奏 `280`–`320ms` 的定点值。 */
export const ISLAND_ANCHOR_GROW_MS = 300;
/** 旧内容收回的时长，与渲染层 WAAPI、island.css 的 `--island-anchor-shrink` 共用；取收缩节奏 `200`–`280ms` 的定点值。 */
export const ISLAND_ANCHOR_SHRINK_MS = 240;
/** hover 下方最多两行：主信息与补充信息。 */
export const ISLAND_HOVER_LINE_LIMIT = 2;
/** hover 下方最多三个控件。 */
export const ISLAND_HOVER_CONTROL_LIMIT = 3;

/** 程序内置的基础状态集；插件扩展状态须同样补齐视觉差异、短词与去向。 */
export type IslandStatus = "pending" | "running" | "paused" | "completed" | "failed" | "cancelled" | "awaiting" | "disconnected" | "unauthorized" | "indeterminate";
/** 结束后的去向：停留、收成圆点（只剩锚点）或直接消失。 */
export type IslandStatusDisposition = "linger" | "dot" | "vanish";
/** 锚点内的符号，渲染层映射到既有图标，不新增图形资源。 */
export type IslandStatusSymbol = "pause" | "check" | "cross" | "alert" | "question" | "link-off" | "lock";
/** 状态的静态视觉差异：环（弧长随进度）、无总量不定弧或符号。 */
export type IslandStatusVisual = { kind: "ring"; source: "empty" | "live" } | { kind: "sweep" } | { kind: "glyph"; symbol: IslandStatusSymbol };
/** 锚点实际绘制的图形：归一化弧长、不定弧或符号。 */
export type IslandAnchorGlyph = { kind: "ring"; arc: number } | { kind: "sweep" } | { kind: "glyph"; symbol: IslandStatusSymbol };
/** 一种基础状态的完整声明：短词、视觉差异与结束后的去向，缺一不算状态。 */
export interface IslandStatusSpec
{
    status: string;
    word: string; // 一句话短词，固定用词，不用近义词替代。
    visual: IslandStatusVisual;
    disposition: IslandStatusDisposition;
}

export const ISLAND_STATUS_SPECS: Readonly<Record<IslandStatus, IslandStatusSpec>> = {
    pending: { status: "pending", word: "待开始", visual: { kind: "ring", source: "empty" }, disposition: "linger" },
    running: { status: "running", word: "进行中", visual: { kind: "ring", source: "live" }, disposition: "linger" },
    paused: { status: "paused", word: "暂停", visual: { kind: "glyph", symbol: "pause" }, disposition: "linger" },
    completed: { status: "completed", word: "完成", visual: { kind: "glyph", symbol: "check" }, disposition: "dot" },
    failed: { status: "failed", word: "失败", visual: { kind: "glyph", symbol: "alert" }, disposition: "linger" },
    cancelled: { status: "cancelled", word: "取消", visual: { kind: "glyph", symbol: "cross" }, disposition: "vanish" },
    awaiting: { status: "awaiting", word: "待确认", visual: { kind: "glyph", symbol: "question" }, disposition: "linger" },
    disconnected: { status: "disconnected", word: "未连接", visual: { kind: "glyph", symbol: "link-off" }, disposition: "linger" },
    unauthorized: { status: "unauthorized", word: "未授权", visual: { kind: "glyph", symbol: "lock" }, disposition: "linger" },
    indeterminate: { status: "indeterminate", word: "运行中", visual: { kind: "sweep" }, disposition: "linger" },
};

/** 状态集不封闭：内置十态之外的状态由插件按同一形状声明，内置状态不能被改写或省略。 */
export const islandStatusSpec = (status: string, extended: readonly IslandStatusSpec[] = []): IslandStatusSpec | undefined =>
    (ISLAND_STATUS_SPECS as Readonly<Record<string, IslandStatusSpec>>)[status] ?? extended.find((spec) => spec.status === status);

/** 把进度夹到归一化区间，非法数值按起点处理，不让坏数据进入绘制。 */
const clampProgress = (progress: number): number => (Number.isFinite(progress) ? Math.min(1, Math.max(0, progress)) : 0);

/**
 * 状态变化取消未完成动画并直接落终态：返回值是归一化进度，只由当前状态决定，
 * 不保留上一帧的中间值；无总量运行中没有归一化进度。
 */
export const islandStatusProgress = (status: IslandStatus, progress: number): number | undefined =>
{
    switch (status)
    {
        case "completed":
        case "failed":
            return 1; // 结束在满环，不继续播放未完成的动画。
        case "cancelled":
        case "disconnected":
        case "unauthorized":
            return 0; // 未完成即结束，直接落空环。
        case "pending":
            return 0;
        case "running":
        case "paused":
        case "awaiting":
            return clampProgress(progress);
        case "indeterminate":
            return undefined;
    }
};

/** 锚点内的绘制图形；同样是状态的纯函数，状态一变即落终态，不做插值。 */
export const islandAnchorGlyph = (status: IslandStatus, progress = 0): IslandAnchorGlyph =>
{
    const visual = ISLAND_STATUS_SPECS[status].visual;
    if (visual.kind === "glyph") return { kind: "glyph", symbol: visual.symbol };
    if (visual.kind === "sweep") return { kind: "sweep" };
    return { kind: "ring", arc: visual.source === "empty" ? 0 : clampProgress(progress) };
};

/** 内容的叠加层级；同一锚点只承载同一来源的一层内容。 */
export type IslandContentLayer = "drop" | "confirm" | "message" | "event" | "resident";
/** 显示优先级：拖放 > 确认 > 消息 > 普通事件 > 常态。下标越小越优先。 */
export const ISLAND_LAYER_ORDER: readonly IslandContentLayer[] = ["drop", "confirm", "message", "event", "resident"];
/** 层级优先级比较值；未知层级按最低处理，不让插件内容抢占前台。 */
export const islandLayerRank = (layer: IslandContentLayer): number =>
{
    const rank = ISLAND_LAYER_ORDER.indexOf(layer);
    return rank < 0 ? ISLAND_LAYER_ORDER.length : rank;
};

/** 一个内容单元：一行主信息加一个可选附加项，不跨锚点拼第二行。 */
export interface IslandContentUnit
{
    status: string; // 内置状态用 `IslandStatus`；插件扩展状态按同一形状声明后由 `islandStatusSpec` 解析。
    primary: string; // 中文上限 18 字、英文上限 36 字符，超出由渲染层省略。
    secondary?: string; // 附加项：整数百分比、`99+` 或 `mm:ss`，由 islandAnchorValue 放行。
}

/** 附加项的白名单结果：整数百分比、`99+` 计数或 `mm:ss` 剩余时间。 */
export type IslandAnchorValue =
    | { kind: "percent"; value: number } // 0–100 的整数百分比。
    | { kind: "count" } // 固定的 `99+` 计数。
    | { kind: "time"; seconds: number }; // mm:ss 剩余时间的总秒数。

/** 附加项白名单：裸整数、`N+` 与其它形式一律不接受，渲染层据此决定是否显示附加项。 */
export const islandAnchorValue = (text: string): IslandAnchorValue | undefined =>
{
    if (text === "99+") return { kind: "count" };
    if (/^(?:100|[0-9]{1,2})%$/.test(text)) return { kind: "percent", value: Number.parseInt(text, 10) };
    const time = /^([0-9]{2}):([0-5][0-9])$/.exec(text);
    if (time) return { kind: "time", seconds: Number(time[1]) * 60 + Number(time[2]) };
    return undefined;
};

/** 锚点承载的功能身份：hover 顶部用功能图标区分锚点，状态符号移到内容行左侧。 */
export type IslandFunctionKind = "music" | "message" | "plugin" | "event";

/** 一个锚点：身份由层级与来源决定，顺序按插入先后，收起与重开都不换位。 */
export interface IslandAnchor
{
    id: string;
    source: string;
    layer: IslandContentLayer;
    updatedAtMs: number; // 最近一次换内容时刻；合并通知锚点取最新条目时刻。
    kind?: IslandFunctionKind; // 功能身份；缺省按普通事件处理。
    unit?: IslandContentUnit; // 纯锚点没有内容单元。
    count?: number; // 合并通知条数，超过 99 表示「99+」。
}

/** 功能身份缺省为普通事件；插件卡没有单独身份时也走事件图标。 */
export const islandAnchorKind = (anchor: IslandAnchor): IslandFunctionKind => anchor.kind ?? "event";

/** 新到达的内容；同一来源同一层级复用锚点，其余情况在右侧插入。 */
export interface IslandAnchorArrival
{
    source: string;
    layer: IslandContentLayer;
    nowMs: number;
    kind?: IslandFunctionKind; // 功能身份；消息、常驻卡与插件各自标注。
    unit?: IslandContentUnit;
}

/** 锚点身份：同来源换层级即换锚点，同层级同来源复用同一个锚点。 */
export const islandAnchorId = (layer: IslandContentLayer, source: string): string => `${layer}:${source}`;

/** 插入或替换内容：同来源同层级原地换内容，不同来源按插入序追加到右侧。 */
export const islandArrive = (anchors: readonly IslandAnchor[], arrival: IslandAnchorArrival): IslandAnchor[] =>
{
    const id = islandAnchorId(arrival.layer, arrival.source);
    const index = anchors.findIndex((anchor) => anchor.id === id);
    if (index < 0)
        return [...anchors, { id, source: arrival.source, layer: arrival.layer, updatedAtMs: arrival.nowMs, kind: arrival.kind, unit: arrival.unit }];
    return anchors.map((anchor, position) => (position === index ? { ...anchor, updatedAtMs: arrival.nowMs, kind: arrival.kind ?? anchor.kind, unit: arrival.unit, count: undefined } : anchor));
};

/** 锚点行：最多并排三枚，其余折进「＋N」；被折叠的锚点在展开里仍按原顺序排列。 */
export interface IslandAnchorRow
{
    visible: readonly IslandAnchor[];
    overflow: number; // 折叠数量；0 表示不出现「＋N」圆。
    folded: readonly IslandAnchor[];
}

/** 折叠：不改变顺序，只决定胶囊里露出几枚。 */
export const islandAnchorRow = (anchors: readonly IslandAnchor[]): IslandAnchorRow =>
    anchors.length <= ISLAND_ANCHOR_SLOTS
        ? { visible: [...anchors], overflow: 0, folded: [] }
        : { visible: anchors.slice(0, ISLAND_ANCHOR_SLOTS), overflow: anchors.length - ISLAND_ANCHOR_SLOTS, folded: anchors.slice(ISLAND_ANCHOR_SLOTS) };

/** 胶囊正面：一排锚点或零锚点的空态时钟；两处都仍可单击展开。 */
export type IslandCapsuleFace =
    | { kind: "anchors"; visible: readonly IslandAnchor[]; overflow: number; expandable: true }
    | { kind: "clock"; text: string; expandable: true };

/** 空态时钟文本 `hh:mm`，由调用方给的时刻固定，不在模型里走表。 */
export const islandClockLabel = (nowMs: number): string =>
{
    const date = new Date(Number.isFinite(nowMs) ? nowMs : 0);
    return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
};

/** 零锚点显示空态圆锚点（当前时间），仍可单击展开；有锚点时只给折叠后的可见行。 */
export const islandCapsuleFace = (anchors: readonly IslandAnchor[], nowMs: number): IslandCapsuleFace =>
{
    if (!anchors.length) return { kind: "clock", text: islandClockLabel(nowMs), expandable: true };
    const row = islandAnchorRow(anchors);
    return { kind: "anchors", visible: row.visible, overflow: row.overflow, expandable: true };
};

/** hover 下方内容：最多两行与最多三个控件，只显示选中锚点自己的内容。 */
export interface IslandHoverContent
{
    lines: readonly string[];
    controls: readonly string[]; // 模型截取的可操作项：最多 3 个可立即执行、可撤销或低风险的操作。
    expandable: boolean; // 没有内容可看时为 false，对应「只剩锚点」。
}

/** hover 内容：主信息在前、补充信息在后，超出的行截断；控件由渲染层传入当前锚点可立即执行的项，模型只截取前 3 个。 */
export const islandHoverContent = (anchor: IslandAnchor | undefined, controls: readonly string[] = []): IslandHoverContent =>
{
    if (!anchor) return { lines: [], controls: [], expandable: false };
    const lines = anchor.unit
        ? [anchor.unit.primary, ...(anchor.unit.secondary === undefined ? [] : [anchor.unit.secondary])].slice(0, ISLAND_HOVER_LINE_LIMIT)
        : anchor.count === undefined
          ? []
          : [`${anchor.count} 条消息`];
    return { lines, controls: controls.slice(0, ISLAND_HOVER_CONTROL_LIMIT), expandable: lines.length > 0 };
};

/** hover 身份：消息通知列表由 islandMessages 投影，插件可自绘，其余使用行与控件。 */
export type IslandHoverProvider = Exclude<IslandFunctionKind, "event"> | "default";

/** 渲染层使用功能身份分发当前详情；普通事件与空锚点走默认。 */
export const islandHoverProvider = (anchor: IslandAnchor | undefined): IslandHoverProvider =>
{
    const kind = anchor && islandAnchorKind(anchor);
    return !kind || kind === "event" ? "default" : kind;
};

/** 提供详情基本行与动作限制；合并通知的多个条目不受单锚点两行限制。 */
export const islandHoverBody = (anchor: IslandAnchor | undefined, controls: readonly string[] = []): IslandHoverContent & { provider: IslandHoverProvider } =>
    ({ provider: islandHoverProvider(anchor), ...islandHoverContent(anchor, controls) });

/** hover 宽度取所有锚点实测内容宽度的最大值，至少保留胶囊宽度；忽略无效测量。 */
export const islandHoverInlineSize = (anchorWidths: readonly number[], capsuleWidth = 0): number =>
    anchorWidths.reduce((width, sample) => Number.isFinite(sample) ? Math.max(width, sample) : width, Number.isFinite(capsuleWidth) ? Math.max(0, capsuleWidth) : 0);

/** hover 打开期间保留测过的最大宽度；退出即归零，下一次从新内容测起。 */
export const islandHoverSessionWidth = (hover: boolean, previousWidth: number, measuredWidth: number): number =>
    hover ? islandHoverInlineSize([previousWidth, measuredWidth]) : 0;

/** hover 选中项：身份失效时回落到第一枚，列表与下方内容始终对应。 */
export const islandHoverSelect = (anchors: readonly IslandAnchor[], selected: string | undefined): string | undefined =>
    anchors.some((anchor) => anchor.id === selected) ? selected : anchors[0]?.id;

/** 从胶囊进入 hover，先用上一帧实际显示正文的锚点；没有对应项时沿用原有选中与回落规则。 */
export const islandHoverInitialSelection = (anchors: readonly IslandAnchor[], capsuleFront: string | undefined, selected: string | undefined): string | undefined =>
    islandHoverSelect(anchors, anchors.some((anchor) => anchor.id === capsuleFront) ? capsuleFront : selected);

/** 左右方向键在锚点列表内切换；到端点停住，不循环，避免越界后跳到另一端。 */
export const islandHoverStep = (anchors: readonly IslandAnchor[], selected: string | undefined, delta: number): string | undefined =>
{
    const current = islandHoverSelect(anchors, selected);
    if (current === undefined) return undefined;
    const index = anchors.findIndex((anchor) => anchor.id === current);
    const target = Math.min(anchors.length - 1, Math.max(0, index + Math.sign(delta)));
    return anchors[target]?.id;
};

/** Enter 进入选中锚点的内容：只有带可显示内容的锚点能进入，纯圆锚点不接管。 */
export const islandHoverEnter = (anchor: IslandAnchor | undefined): string | undefined => (anchor && islandHoverContent(anchor).expandable ? anchor.id : undefined);

/** 拖放目标：收集箱或音乐；没有匹配插件时没有目标。 */
export type IslandDropTarget = "collect-box" | "music";
/** 拖放接收态：锚点换松手图标，主信息给出松手结果或「无法接收」。 */
export interface IslandDropFace
{
    accept: boolean; // false 时松手回退且不执行。
    icon: "release";
    primary: string;
}

/** 接收态文案固定，不改写近义词；无匹配插件时明确不可接收。 */
export const islandDropFace = (target: IslandDropTarget | undefined): IslandDropFace =>
{
    if (target === "collect-box") return { accept: true, icon: "release", primary: "松开加入收集箱" };
    if (target === "music") return { accept: true, icon: "release", primary: "交给音乐播放" };
    return { accept: false, icon: "release", primary: "无法接收" };
};

/** 胶囊与 hover 共用可见锚点；仅过滤显示，不变更已到达的事实、展开卡片或默认值。 */
export const islandDisplayAnchors = (anchors: readonly IslandAnchor[], preferences: Pick<ApplicationPreferences, "islandAnchorVisible"> | undefined): IslandAnchor[] =>
    anchors.filter((anchor) => {
        const kind = anchor.layer === "message" ? "message" : anchor.kind;
        const key = ISLAND_ANCHOR_KEYS.find((item) => item === kind);
        return key === undefined || islandAnchorVisible(preferences, key);
    });

/** 媒体会话事实：音乐卡是否启用，以及当前会话的播放事实（缺失表示没有会话）。 */
export interface IslandMediaFacts
{
    enabled: boolean; // 音乐卡在岛卡顺序中启用。
    playback: "playing" | "paused" | "stopped" | "unknown" | undefined;
}

/** 音乐锚点只在有媒体会话且播放或暂停时出现；停止、无会话或未启用该卡都不产生，因此不再常驻。 */
export const islandMusicAnchorVisible = (facts: IslandMediaFacts): boolean => facts.enabled && (facts.playback === "playing" || facts.playback === "paused");

/** 天气通知：摘要变化时由浮岛投递一次本地通知；首次或未变化不投递。 */
export const islandWeatherNotice = (previous: string | undefined, next: string | undefined): boolean => previous !== undefined && next !== undefined && previous !== next;

/** 活动型音乐锚点由渲染层放入事件层，优先级低于合并通知。 */
export const ISLAND_NOTICE_LAYER: IslandContentLayer = "event";

/** 收集快照条目的最小形状：身份与创建时刻；标题格式仍由调用方既有函数负责。 */
export interface IslandCollectEntry
{
    name: string;
    createdUnixMs: number;
}

/** 收集快照比较结果：新条目出现时给出完成态与标题，否则不产生通知。 */
export type IslandCollectArrival = { activity: "completed"; title: string } | { activity: "idle" };

/**
 * 比较两次收集快照：只有「数量增加且出现新身份（名称与创建时刻都不在旧快照）」才算新收录。
 * 重命名（名称变、创建时刻未变）、置顶（只改顺序）、删除（数量减少）与无变化都不产生通知；
 * 首次读取没有旧快照，同样不通知。
 */
export const islandCollectArrival = <T extends IslandCollectEntry>(previous: readonly T[] | undefined, next: readonly T[], title: (entry: T) => string): IslandCollectArrival =>
{
    if (!previous || next.length <= previous.length) return { activity: "idle" };
    const names = new Set(previous.map((entry) => entry.name));
    const created = new Set(previous.map((entry) => entry.createdUnixMs));
    const added = next
        .filter((entry) => !names.has(entry.name) && !created.has(entry.createdUnixMs))
        .sort((left, right) => right.createdUnixMs - left.createdUnixMs)[0];
    return added ? { activity: "completed", title: title(added) } : { activity: "idle" };
};
