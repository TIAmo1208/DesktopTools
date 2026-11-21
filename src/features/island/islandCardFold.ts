/** 卡内展开/收缩事务：意图、序号与目标高度分开，连续点击只替换最新目标。 */
export type IslandFoldPhase = "idle" | "pending" | "animating";

export interface IslandFoldState
{
    phase: IslandFoldPhase; // 当前事务阶段。
    target: boolean; // true = 收起，false = 展开。
    seq: number; // 每次点击递增；旧动画/旧回执只能结算自己的序号。
}

/** 点击立即登记新目标；旧目标与旧动画由序号淘汰，不排队叠加。 */
export const beginIslandFold = (state: IslandFoldState, collapsed: boolean): IslandFoldState => ({
    phase: "pending",
    target: !collapsed,
    seq: state.seq + 1,
});

/** 只有最新序号能进入动画阶段。 */
export const animateIslandFold = (state: IslandFoldState, seq: number): IslandFoldState =>
    state.seq === seq && state.phase !== "idle" ? { ...state, phase: "animating" } : state;

/** 只有最新序号能回到稳定态；旧动画结束不得清理新事务。 */
export const settleIslandFold = (state: IslandFoldState, seq: number): IslandFoldState =>
    state.seq === seq && state.phase !== "idle" ? { ...state, phase: "idle" } : state;

/** 目标高度：优先实测自然高，测不到时回落最近一次有效高度。 */
export const islandFoldTargetHeight = (naturalHeight: number, rememberedHeight?: number): number | undefined => {
    if (Number.isFinite(naturalHeight) && naturalHeight > 0) return naturalHeight;
    if (rememberedHeight !== undefined && Number.isFinite(rememberedHeight) && rememberedHeight > 0) return rememberedHeight;
    return undefined;
};

/** 拖动排序的落点：先取指针覆盖的卡片，否则取纵向中心最近的一张；不含被拖卡自身。 */
export interface IslandCardDropRect
{
    id: string;
    top: number;
    height: number;
}

export const islandCardDropTarget = (
    cards: readonly IslandCardDropRect[],
    pointOrY: number | { x: number; y: number },
    fromId: string,
    bounds?: { left: number; top: number; width: number; height: number },
): string | undefined => {
    const point = typeof pointOrY === "number" ? { x: 0, y: pointOrY } : pointOrY;
    // 拖出列表/窗口的落点不回退到最近卡，调用方据此回滚到原位。
    if (bounds && (point.x < bounds.left || point.x > bounds.left + bounds.width || point.y < bounds.top || point.y > bounds.top + bounds.height)) return undefined;
    let nearest: { id: string; distance: number } | undefined;
    for (const card of cards) {
        if (card.id === fromId) continue;
        if (point.y >= card.top && point.y <= card.top + card.height) return card.id;
        const distance = Math.abs(point.y - (card.top + card.height / 2));
        if (!nearest || distance < nearest.distance) nearest = { id: card.id, distance };
    }
    return nearest?.id;
};

/** 折叠保存在途计数：每次点击登记一次；无论是否因旧 revision 早退都在 finally 结算。 */
export interface IslandFoldSaveTracker
{
    revision: number;
    inflight: number;
}

/** 登记一次保存并返回本批次 revision；inflight 与登记成对增加。 */
export const beginIslandFoldSave = (tracker: IslandFoldSaveTracker): number => {
    tracker.revision += 1;
    tracker.inflight += 1;
    return tracker.revision;
};

/** 当前 revision 是否仍是最新；早退时仍必须调用 end 减 inflight。 */
export const isIslandFoldSaveCurrent = (tracker: IslandFoldSaveTracker, revision: number): boolean => revision === tracker.revision;

/** 结算一次保存：减 inflight 且不会小于 0，与 begin 严格成对。 */
export const endIslandFoldSave = (tracker: IslandFoldSaveTracker): void => {
    tracker.inflight = Math.max(0, tracker.inflight - 1);
};
