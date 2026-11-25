import { stripIslandControlCharacters } from "../../contracts/island.ts";
import { ISLAND_STATUS_SPECS, islandAnchorGlyph, type IslandStatus } from "./islandContent.ts";

/**
 * 浮岛插件卡的纯投影：一行摘要、消息身份与投递门禁。
 * 摘要与消息正文共用契约里的控制字符规则；长度上限集中在 contracts/island。
 */

/** 胶囊与 hover 一行摘要的字符上限；与消息正文分开，避免长正文塞满锚点。 */
export const ISLAND_PLUGIN_SUMMARY_LIMIT = 40;

/** 把插件给的摘要收敛成单行文本；空、空白与控制字符都回落为 undefined。 */
export const islandPluginSummaryText = (value: string | null | undefined): string | undefined =>
{
    if (typeof value !== "string") return undefined;
    const text = stripIslandControlCharacters(value);
    if (!text) return undefined;
    return Array.from(text).slice(0, ISLAND_PLUGIN_SUMMARY_LIMIT).join("");
};

/** 胶囊与 hover 的主信息：插件调用过 setIslandSummary 时用它，否则回落清单名称；null 表示显式隐藏。 */
export const islandPluginPrimary = (summary: IslandPluginSummaryState | string | null | undefined, fallback: string): string => {
    const text = typeof summary === "object" && summary !== null ? summary.text : summary;
    const state = islandPluginSummaryState(text);
    return state === undefined ? fallback : state.text ?? "";
};

/** 一条插件摘要：null 正文表示显式隐藏，status 由插件同帧给出。 */
export interface IslandPluginSummaryState
{
    text: string | null; // 显式隐藏时也为 null，与「从未调用」区分。
    status?: IslandStatus; // 业务状态；宿主据此选锚点符号，不猜中文短词。
    navigation?: { uri: string }; // 可选声明式导航；有值时锚点可点击。
}

/** 摘要状态：undefined 为从未调用（回落清单名），null 正文为显式隐藏。 */
export const islandPluginSummaryState = (
    value: string | null | undefined,
    status?: string,
    navigation?: { uri: string },
): IslandPluginSummaryState | undefined =>
{
    if (value === undefined) return undefined;
    const text = value === null ? null : islandPluginSummaryText(value) ?? null; // 空串与空白按显式隐藏处理。
    const uri = navigation?.uri?.trim();
    const knownStatus = status && status in ISLAND_STATUS_SPECS ? (status as IslandStatus) : undefined; // 只接受既有状态表里的状态。
    return {
        text,
        ...(knownStatus === undefined ? {} : { status: knownStatus }),
        ...(uri ? { navigation: { uri } } : {}),
    };
};

/** 业务状态到锚点符号的固定映射；只用于兼容旧插件只传短词的调用，插件应直接传 status。 */
const ISLAND_PLUGIN_SUMMARY_STATUS: readonly (readonly [RegExp, IslandStatus])[] = [
    [/^未授权$/, "unauthorized"],
    [/^未连接$/, "disconnected"],
    [/^读取失败$/, "failed"],
    [/^数据陈旧$/, "paused"],
    [/^\d+ 项待决$/, "awaiting"],
    [/^99\+ 项待决$/, "awaiting"],
    [/^账号未登录$/, "unauthorized"],
    [/^余额未授权$/, "unauthorized"],
    [/^余额读取失败$/, "failed"],
    [/^已连接$/, "pending"],
];

/** 从摘要短词推断业务状态；插件没传 status 时用于保持图标语义。 */
export const islandPluginStatusFromSummary = (text: string | null | undefined): IslandStatus | undefined => {
    if (!text) return undefined;
    for (const [pattern, status] of ISLAND_PLUGIN_SUMMARY_STATUS) if (pattern.test(text)) return status;
    return undefined;
};

/** 锚点业务状态：插件显式给的优先，其次兼容短词映射，最后回落注册状态。 */
export const islandPluginAnchorStatus = (
    status: IslandStatus | undefined,
    text: string | null | undefined,
    fallback: IslandStatus,
): IslandStatus => status ?? islandPluginStatusFromSummary(text) ?? fallback;

/** 按插件的 componentId 更新摘要表；传空摘要即撤下，未变化时返回原对象避免多余重绘。 */
export const islandPluginSummaries = (
    previous: Readonly<Record<string, IslandPluginSummaryState>>,
    componentId: string,
    summary: string | null | undefined,
    status?: string,
    navigation?: { uri: string },
): Readonly<Record<string, IslandPluginSummaryState>> =>
{
    const next = islandPluginSummaryState(summary, status, navigation);
    if (next === undefined)
    {
        if (!(componentId in previous)) return previous;
        const remaining = { ...previous };
        delete remaining[componentId];
        return remaining;
    }
    const current = previous[componentId];
    return current?.text === next.text && current.status === next.status && current.navigation?.uri === next.navigation?.uri ? previous : { ...previous, [componentId]: next };
};

/** 本地消息身份：来源插件 + 插件内 key，跨插件不会互相覆盖。 */
export const islandPluginMessageKey = (source: string, key: string): string => `plugin:${source}:${key}`;

/** 暂停（锁屏、浮岛隐藏、全屏隐藏）时不接收新提醒；恢复后只显示插件自己的现状。 */
export const islandPluginMessageDeliverable = (paused: boolean): boolean => !paused;

/** 插件卡的挂载位；同一实例任一时刻只允许一处，暂停时一处也不留。 */
export type IslandPluginMountSite = "hidden" | "hover" | "expanded";

/** 当前三态与运行状态下的挂载计划：实例身份 → 挂载位；暂停或关闭时为空表。 */
export const islandPluginMountSites = (options: {
    running: boolean; // 浮岛实际可运行（启用且未暂停）。
    mode: "collapsed" | "hover" | "expanded"; // 三态。
    instanceIds: readonly string[]; // 当前插件卡的实例标识。
}): ReadonlyMap<string, IslandPluginMountSite> => {
    const sites = new Map<string, IslandPluginMountSite>();
    if (!options.running) return sites; // 锁屏、隐藏、关岛：所有挂载位都不挂。
    const site: IslandPluginMountSite = options.mode === "expanded" ? "expanded" : "hidden"; // hover 也保留隐藏宿主，避免停掉轮询。
    for (const instanceId of options.instanceIds) sites.set(instanceId, site);
    return sites;
};

/**
 * 每个 componentId 一条稳定的摘要回调：上下文每次重绘拿到同一函数，
 * 插件内容不会因为摘要变化被清理重挂；回调内部转发到最新一次更新函数。
 */
export const islandPluginSummaryCallbacks = (
    update: (componentId: string, text: string | null, status?: string, navigation?: { uri: string }) => void,
): ((componentId: string) => (text: string | null, status?: string, navigation?: { uri: string }) => void) => {
    const cache = new Map<string, (text: string | null, status?: string, navigation?: { uri: string }) => void>();
    return (componentId: string) => {
        const cached = cache.get(componentId);
        if (cached) return cached;
        const callback = (text: string | null, status?: string, navigation?: { uri: string }) => update(componentId, text, status, navigation);
        cache.set(componentId, callback);
        return callback;
    };
};

/** 插件身份位的图形选择：只有正常连接（pending）用身份 Logo，其余沿用状态符号。 */
export type IslandPluginMark = "logo" | "pause" | "check" | "alert" | "question" | "link-off" | "lock" | "cross" | "ring" | "sweep";

/** 锚点与标题共用：状态符号优先于身份 Logo，避免锁/暂停/失败被 Logo 覆盖。 */
export const islandPluginMark = (status: string): IslandPluginMark | undefined =>
{
    if (!(status in ISLAND_STATUS_SPECS)) return undefined; // 未知扩展状态仍走插件自绘，不在这里猜。
    if (status === "pending") return "logo";
    const glyph = islandAnchorGlyph(status as IslandStatus);
    return glyph.kind === "glyph" ? glyph.symbol : glyph.kind;
};

/** 插件身份图标的来源：有合法地址且未加载失败才用原图，否则回落通用图标。 */
export type IslandPluginIconSource = { kind: "image"; url: string } | { kind: "fallback" };

/** 解析插件图标渲染源；加载失败记录以 url 为键，换图标后重新尝试。 */
export const islandPluginIconSource = (
    iconPath: string | undefined,
    failedUrl: string | undefined,
    assetUrl: (path: string) => string | undefined,
): IslandPluginIconSource => {
    const url = iconPath ? assetUrl(iconPath) : undefined;
    if (!url || failedUrl === url) return { kind: "fallback" };
    return { kind: "image", url };
};
