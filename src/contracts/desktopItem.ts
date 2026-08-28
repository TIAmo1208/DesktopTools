/** 桌面项目当前可解析状态。 */
export type DesktopItemAvailability = "available" | "unavailable";

/** 前后端共享的桌面项目。 */
export interface DesktopItem {
    stableReference: string;
    displayName: string;
    systemImage: string | null;
    availability: DesktopItemAvailability;
}

/** 前后端共享的不可变桌面项目快照。 */
export interface DesktopItemSnapshot {
    revision: number;
    items: DesktopItem[];
}

/** 标识桌面项目契约边界的载荷错误。 */
export class DesktopItemContractError extends Error {
    readonly code = "invalid-desktop-item-snapshot"; // 可稳定检索的契约错误码。

    /** 创建桌面项目契约错误。 */
    constructor() {
        super("桌面项目数据无效");
        this.name = "DesktopItemContractError";
    }
}

/** 校验 command 返回值与 event 载荷并生成桌面项目快照。 */
export const parseDesktopItemSnapshot = (value: unknown): DesktopItemSnapshot => {
    if (typeof value !== "object" || value === null) throw new DesktopItemContractError();

    // 等待字段校验的桌面项目快照载荷。
    const candidate = value as Record<string, unknown>;
    if (!Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0 || !Array.isArray(candidate.items))
        throw new DesktopItemContractError();

    // 已完成契约校验的桌面项目集合。
    const items = candidate.items.map((item) => {
        if (typeof item !== "object" || item === null) throw new DesktopItemContractError();

        // 等待字段校验的桌面项目载荷。
        const itemCandidate = item as Record<string, unknown>;
        if (typeof itemCandidate.stableReference !== "string" || itemCandidate.stableReference.length === 0) throw new DesktopItemContractError();
        if (typeof itemCandidate.displayName !== "string" || itemCandidate.displayName.length === 0) throw new DesktopItemContractError();
        if (itemCandidate.systemImage !== null && typeof itemCandidate.systemImage !== "string") throw new DesktopItemContractError();
        if (itemCandidate.availability !== "available" && itemCandidate.availability !== "unavailable") throw new DesktopItemContractError();
        // 已完成契约校验的桌面项目可用状态。
        const availability: DesktopItemAvailability = itemCandidate.availability;

        return {
            stableReference: itemCandidate.stableReference,
            displayName: itemCandidate.displayName,
            systemImage: itemCandidate.systemImage,
            availability,
        };
    });

    return { revision: candidate.revision as number, items };
};
