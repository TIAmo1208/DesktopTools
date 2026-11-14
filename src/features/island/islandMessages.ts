import type { IslandMessage, IslandStateRequest } from "../../contracts/island.ts";

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

/**
 * 三态投影，最新一条在前：常态与 hover 只留一条，展开保留最近若干条。
 * 消息集合本身已由后端去重与过期，这里只决定呈现，不重复判定生命周期。
 */
export const islandMessageView = (incoming: IslandMessage[], held: IslandMessage[], mode: IslandStateRequest["state"], limit = ISLAND_MESSAGE_LIMIT): IslandMessage[] => {
    const newestFirst = (messages: IslandMessage[]) => [...messages].sort((left, right) => right.id - left.id);
    if (mode === "collapsed") return newestFirst(incoming).slice(0, 1);
    const merged = newestFirst(islandMergeMessages(held, incoming)).slice(0, limit);
    return mode === "hover" ? merged.slice(0, 1) : merged;
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
