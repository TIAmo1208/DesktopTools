import { ISLAND_MESSAGE_CARD_ID } from "../../contracts/settings.ts";

/**
 * 消息通知卡在展开卡列表里的插入下标：只数排在它前面的实例卡。
 * 顺序里没有消息卡时返回 -1，表示不渲染；等于实例数时落在末尾。
 */
export const islandMessageSlotIndex = (order: readonly string[], instanceIds: readonly string[]): number => {
    const position = order.indexOf(ISLAND_MESSAGE_CARD_ID);
    if (position < 0) return -1;
    const before = new Set(order.slice(0, position));
    return instanceIds.filter((id) => before.has(id)).length;
};

/** 卡内收起默认：收集箱用偏好默认，消息卡按规格默认收起到最近一条，其余默认展开。 */
export const islandCardCollapseDefault = (
    componentId: string,
    collectDefault: boolean,
    defaultCollapsed: Readonly<Record<string, boolean>> = {},
): boolean => {
    // 统一优先级（t102/t103）：有逐卡「默认收起」键就以它为准；缺省时收集箱回退旧 collectDefault、消息卡回退收起、普通卡展开。
    if (Object.prototype.hasOwnProperty.call(defaultCollapsed, componentId)) return defaultCollapsed[componentId] === true;
    return componentId === "official.collect-box" ? collectDefault : componentId === ISLAND_MESSAGE_CARD_ID;
};

/**
 * 从已保存偏好读出某张卡的收起态：记忆开关打开时以用户显式写过的值为准，
 * 关闭时丢掉已存值回到该卡默认；从未操作过时消息卡默认收起。
 */
export const islandCardRememberedCollapsed = (
    stored: Readonly<Record<string, boolean>>,
    memory: Readonly<Record<string, boolean>>,
    componentId: string,
    collectDefault: boolean,
    defaultCollapsed: Readonly<Record<string, boolean>> = {},
): boolean => (memory[componentId] === false ? undefined : stored[componentId]) ?? islandCardCollapseDefault(componentId, collectDefault, defaultCollapsed);
