import type { ComponentManifest } from "../../contracts/component.ts";

const ISLAND_CARD_LIMIT = 32;
const ISLAND_CARD_PREFIXES = ["official.", "plugin."];

/** 岛卡顺序只保留合法身份，按首次出现去重并限制数量。 */
export const normalizeIslandCardOrder = (order: readonly string[]) =>
    [...new Set(order.filter((id) => ISLAND_CARD_PREFIXES.some((prefix) => id.startsWith(prefix) && id.length > prefix.length)))].slice(0, ISLAND_CARD_LIMIT);

/** 开关直接增删同一份岛卡顺序。 */
export const updateIslandCardOrder = (order: readonly string[], cardId: string, enabled: boolean) => {
    const normalized = normalizeIslandCardOrder(order);
    return enabled ? normalizeIslandCardOrder([...normalized, cardId]) : normalized.filter((id) => id !== cardId);
};

/** 在可用卡列表内移动一格，边界动作保持原顺序。 */
export const moveIslandCard = (order: readonly string[], cardId: string, direction: -1 | 1) => {
    const normalized = normalizeIslandCardOrder(order);
    const index = normalized.indexOf(cardId);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= normalized.length) return normalized;
    [normalized[index], normalized[target]] = [normalized[target], normalized[index]];
    return normalized;
};

/** 行动作可用性由当前启用顺序唯一决定。 */
export const islandCardAvailability = (order: readonly string[], cardId: string) => {
    const normalized = normalizeIslandCardOrder(order);
    const index = normalized.indexOf(cardId);
    return { enabled: index >= 0, canMoveUp: index > 0, canMoveDown: index >= 0 && index < normalized.length - 1 };
};

/** 清单里的岛卡按已启用顺序优先，其余按名称稳定排列。 */
export const sortIslandCardManifests = (manifests: readonly ComponentManifest[], order: readonly string[]) => {
    const positions = new Map(normalizeIslandCardOrder(order).map((id, index) => [id, index]));
    return manifests
        .filter((manifest) => manifest.island === true)
        .sort((left, right) => {
            const leftIndex = positions.get(left.id);
            const rightIndex = positions.get(right.id);
            if (leftIndex !== undefined || rightIndex !== undefined) return (leftIndex ?? Number.MAX_SAFE_INTEGER) - (rightIndex ?? Number.MAX_SAFE_INTEGER);
            return left.name.localeCompare(right.name, "zh");
        });
};
