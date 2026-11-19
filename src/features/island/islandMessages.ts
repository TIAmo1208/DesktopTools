import type { IslandMessage, IslandStateRequest } from "../../contracts/island.ts";
import type { WeatherOutlookFacts } from "./islandWeatherOutlook.ts";
import { ISLAND_MESSAGE_ANCHOR_MS, ISLAND_MESSAGE_CONTENT_MS, islandAnchorId, type IslandAnchor, type IslandStatus } from "./islandContent.ts";

/** 消息两段时长：正文展示与锚点保留，由偏好在渲染层换算成毫秒后显式传入。 */
export interface IslandMessageTiming {
    contentMs: number; // 正文可展示时长。
    anchorMs: number; // 正文收起后图标继续保留的时长。
}

/** 默认两段时长；渲染层没有偏好时与设置项默认值一致。 */
export const ISLAND_MESSAGE_TIMING: IslandMessageTiming = { contentMs: ISLAND_MESSAGE_CONTENT_MS, anchorMs: ISLAND_MESSAGE_ANCHOR_MS };

/** 合并通知锚点固定身份；单条通知保留自己的来源。 */
export const ISLAND_NOTIFICATION_SOURCE = "notifications";
export const ISLAND_NOTIFICATION_ANCHOR_ID = islandAnchorId("message", ISLAND_NOTIFICATION_SOURCE);

/** 通知锚点的只读条目；只有原生条目持有可回传的 Toast 身份。 */
export interface IslandNotification {
    key: string;
    nativeId?: number;
    source: string;
    content: string;
    receivedAtMs: number;
    status: IslandStatus;
    type?: IslandMessage["type"]; // 原生类型不等同于状态；警告与普通信息都可能是 pending。
    loading?: boolean;
    progress?: number; // 内部事实的归一化进度；原生 Toast 投影只在调用方给出进度时才有。
    iconUrl?: string; // 应用 Logo 数据 URL；系统通知条目才有。
    weatherCode?: string;
    weatherOutlookFacts?: WeatherOutlookFacts; // 只在本地天气条目保存到达时的同城预报。
}

/** 浮岛由实际天气变化与新增收录产生的本地事实。 */
export type IslandLocalNotification = { content: string } & ({ source: "weather"; weatherCode?: string; weatherOutlookFacts?: WeatherOutlookFacts } | { source: "collect" });

/** 天气与收录共用消息锚点的到达和期限，不创建原生 Toast 身份。 */
export const islandLocalNotificationArrival = (id: number, notice: IslandLocalNotification, receivedAtMs: number): IslandNotification => ({
    key: `local:${id}`, ...notice, receivedAtMs, status: "pending",
});

/** 原生通知的语义投影；原生队列仍独立负责 Toast，可选通道参数只在存在时带出。 */
export const islandMessageArrival = (message: IslandMessage): IslandNotification => ({
    key: `native:${message.id}`,
    nativeId: message.id,
    source: message.source,
    content: message.content,
    receivedAtMs: message.receivedAtMs,
    type: message.type,
    loading: message.loading,
    ...(message.progress === undefined ? {} : { progress: message.progress }),
    ...(message.iconUrl === undefined ? {} : { iconUrl: message.iconUrl }),
    status: message.type === "error" ? "failed" : message.loading ? "indeterminate" : message.type === "success" ? "completed" : "pending",
});

/** 胶囊只按顶部条目的真实类型着色，加载优先于类型。 */
export const islandNotificationTone = (notice: IslandNotification | undefined): IslandMessage["type"] | "loading" =>
    notice?.loading ? "loading" : notice?.type ?? "info";

/** 详情只消费投影携带的事实，不从正文或来源字符串猜测进度、天气。 */
export const islandNotificationVisual = (notice: IslandNotification) => {
    if (notice.loading) {
        const progress = notice.progress;
        return typeof progress === "number" && Number.isFinite(progress) && progress >= 0 && progress <= 1
            ? { kind: "progress" as const, progress } : { kind: "loading" as const };
    }
    if (notice.weatherCode !== undefined) return { kind: "weather" as const, code: notice.weatherCode };
    return notice.type ? { kind: "type" as const, type: notice.type } : undefined;
};

/** 合并各来源条目，同身份用最新投递内容，按到达时间由新到旧。 */
export const islandMergeNotifications = (current: readonly IslandNotification[], incoming: readonly IslandNotification[]): IslandNotification[] => {
    const byKey = new Map<string, IslandNotification>();
    for (const item of [...incoming].reverse()) if (!byKey.has(item.key)) byKey.set(item.key, item); // 后到覆盖旧内容。
    for (const item of current) if (!byKey.has(item.key)) byKey.set(item.key, item);
    return [...byKey.values()].sort((left, right) => right.receivedAtMs - left.receivedAtMs);
};

/** 有确定且尚未走满的进度事实时，锚点不自动过期；进度本身不推断成功。 */
const islandNotificationActiveLoading = (notice: IslandNotification): boolean =>
    notice.loading === true && typeof notice.progress === "number" && Number.isFinite(notice.progress) && notice.progress >= 0 && notice.progress < 1;

/** 正文截止时刻：所有条目从各自到达时刻起算，失败与待确认同样会收起正文。 */
export const islandNotificationContentDeadline = (notice: IslandNotification, timing: IslandMessageTiming = ISLAND_MESSAGE_TIMING): number =>
    notice.receivedAtMs + timing.contentMs;

/** 正文期是否仍在：决定胶囊显示正文还是只留锚点图标。 */
export const islandNotificationContentVisible = (notice: IslandNotification, now: number, timing: IslandMessageTiming = ISLAND_MESSAGE_TIMING): boolean =>
    now < islandNotificationContentDeadline(notice, timing);

/**
 * 条目寿命：正文时长与锚点时长各自从到达时刻独立起算，互相不串接。
 * 正文长于锚点时锚点跟正文一起结束，正文短于锚点时正文收起后锚点继续留到自己的期限。
 */
export const islandNotificationDeadline = (notice: IslandNotification, timing: IslandMessageTiming = ISLAND_MESSAGE_TIMING): number | undefined =>
    notice.status === "failed" || notice.status === "awaiting" || islandNotificationActiveLoading(notice)
        ? undefined
        : notice.receivedAtMs + Math.max(timing.contentMs, timing.anchorMs);

/** 阅读期不抽走已显示的条目；恢复可见时用常态条件丢弃隐藏期间已过期的通知。 */
export const islandKeepNotifications = (notices: readonly IslandNotification[], now: number, mode: IslandStateRequest["state"], timing: IslandMessageTiming = ISLAND_MESSAGE_TIMING): readonly IslandNotification[] => {
    if (mode !== "collapsed") return notices;
    const alive = notices.filter((notice) => now < (islandNotificationDeadline(notice, timing) ?? Infinity));
    return alive.length === notices.length ? notices : alive;
};

/** 单条关闭、计数及胶囊摘要共用同一组通知。 */
export const islandDismissNotification = (notices: readonly IslandNotification[], key: string): readonly IslandNotification[] => notices.filter((notice) => notice.key !== key);

/** 保留一个溢出值以区分恰好 99 条与 99+。 */
export const islandNotificationCountLabel = (count: number): string => count > 99 ? "99+" : String(count);

/**
 * 投影已有排序的条目；胶囊取最新一条，hover 列表由调用方取同一视图前五条。
 * 正文期结束后只保留锚点身份（计数仍在），不再提供可竞争胶囊正文的内容单元。
 */
export const islandNotificationAnchor = (notices: readonly IslandNotification[], options: { nowMs?: number; timing?: IslandMessageTiming } = {}): IslandAnchor | undefined => {
    const latest = notices[0];
    if (!latest) return undefined;
    const timing = options.timing ?? ISLAND_MESSAGE_TIMING;
    const withContent = options.nowMs === undefined || islandNotificationContentVisible(latest, options.nowMs, timing);
    return {
        id: ISLAND_NOTIFICATION_ANCHOR_ID, source: ISLAND_NOTIFICATION_SOURCE, layer: "message", kind: "message",
        count: Math.min(100, notices.length), updatedAtMs: latest.receivedAtMs, noticeKey: latest.key,
        ...(withContent ? { unit: { primary: latest.content, status: latest.status } } : {}),
    };
};

/** 胶囊锚点内的图形：按最新条目的真实类型选图标，加载走进度环或不定动画。 */
export type IslandCapsuleGlyph =
    | { kind: "bell" } // 普通信息、天气与收录等无原生类型的条目。
    | { kind: "logo"; url: string } // 系统通知条目用来源应用 Logo。
    | { kind: "type"; type: "success" | "warning" | "error" } // 成功、警告与错误的固定图形。
    | { kind: "progress"; progress: number } // 有确定进度的加载。
    | { kind: "loading" }; // 没有有效进度的加载。

/** 只读投影事实，不从正文措辞猜测类型或进度；未知类型回落到铃铛。 */
export const islandCapsuleGlyph = (notice: IslandNotification | undefined): IslandCapsuleGlyph => {
    if (!notice) return { kind: "bell" };
    if (notice.iconUrl) return { kind: "logo", url: notice.iconUrl };
    if (notice.loading) {
        const progress = notice.progress;
        return typeof progress === "number" && Number.isFinite(progress) && progress >= 0 && progress <= 1
            ? { kind: "progress", progress } : { kind: "loading" };
    }
    return notice.type === "success" || notice.type === "warning" || notice.type === "error" ? { kind: "type", type: notice.type } : { kind: "bell" };
};

/** 浮岛消息最多同时保留的条数，与原生队列上限一致。 */
export const ISLAND_MESSAGE_LIMIT = 5;

/** 合并两批消息并按身份去重；id 单调递增，排序即到达顺序。 */
export const islandMergeMessages = (held: IslandMessage[], incoming: IslandMessage[]): IslandMessage[] => {
    const byId = new Map<number, IslandMessage>();
    for (const message of [...held, ...incoming]) byId.set(message.id, message); // 后到的同身份覆盖旧内容。
    return [...byId.values()].sort((left, right) => left.id - right.id);
};

/** 阅读期暂存：hover 与展开期间消失的消息留在当前视图，回到常态立即清空；只保留最近若干条。 */
export const islandHoldMessages = (held: IslandMessage[], incoming: IslandMessage[], mode: IslandStateRequest["state"]): IslandMessage[] =>
    mode === "collapsed" ? [] : islandMergeMessages(held, incoming).slice(-ISLAND_MESSAGE_LIMIT);

/** 展开消息卡只消费原生活动与阅读暂存，仍按原生到达顺序取最近五条。 */
export const islandMessagePanelView = (incoming: IslandMessage[], held: IslandMessage[]): IslandMessage[] =>
    islandMergeMessages(held, incoming).slice(-ISLAND_MESSAGE_LIMIT).reverse();

/** 键盘关闭时按原列表顺序找仍在的下一条，再反向找上一条。 */
export const islandMessageFocusOrder = (ids: readonly number[], dismissed: number): number[] => {
    const index = ids.indexOf(dismissed);
    return index < 0 ? [] : [...ids.slice(index + 1), ...ids.slice(0, index).reverse()];
};

/** 当天只显示时分，跨天补月日；时间由到达时刻固定，刷新不改变。 */
export const islandMessageTime = (receivedAtMs: number, now: number): string => {
    const received = new Date(receivedAtMs);
    const current = new Date(now);
    const pad = (value: number) => String(value).padStart(2, "0");
    const clock = `${pad(received.getHours())}:${pad(received.getMinutes())}`;
    const sameDay = received.getFullYear() === current.getFullYear() && received.getMonth() === current.getMonth() && received.getDate() === current.getDate();
    return sameDay ? clock : `${received.getMonth() + 1}/${received.getDate()} ${clock}`;
};
