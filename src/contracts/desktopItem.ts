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

/** 用户触发的 Shell 操作类型。 */
export type ShellOperationKind = "rename" | "recycle" | "permanent-delete" | "copy" | "cut" | "paste" | "drag";

/** 一个 Shell 项目的实际操作结果。 */
export interface ShellOperationItemResult {
    sourceReference: string;
    resultReference: string | null;
    succeeded: boolean;
    resultCode: number;
}

/** 用户触发的一次 Shell 操作结果与撤销属性。 */
export interface ShellOperationResult {
    operationId: number;
    completedAtUnixMicros: number;
    kind: ShellOperationKind;
    aborted: boolean;
    reversible: boolean;
    permanent: boolean;
    effect: string | null;
    items: ShellOperationItemResult[];
    message: string;
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

/** 校验 command 返回的 Shell 操作实际结果。 */
export const parseShellOperationResult = (value: unknown): ShellOperationResult => {
    if (typeof value !== "object" || value === null) throw new DesktopItemContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的 Shell 操作结果。
    const kinds: ShellOperationKind[] = ["rename", "recycle", "permanent-delete", "copy", "cut", "paste", "drag"]; // 公开协议允许的操作类型。
    if (!Number.isSafeInteger(candidate.operationId) || (candidate.operationId as number) < 0) throw new DesktopItemContractError();
    if (!Number.isSafeInteger(candidate.completedAtUnixMicros) || (candidate.completedAtUnixMicros as number) < 0) throw new DesktopItemContractError();
    if (!kinds.includes(candidate.kind as ShellOperationKind)) throw new DesktopItemContractError();
    if (typeof candidate.aborted !== "boolean" || typeof candidate.reversible !== "boolean" || typeof candidate.permanent !== "boolean")
        throw new DesktopItemContractError();
    if (candidate.effect !== null && typeof candidate.effect !== "string") throw new DesktopItemContractError();
    if (!Array.isArray(candidate.items) || typeof candidate.message !== "string") throw new DesktopItemContractError();
    const items = candidate.items.map((item) => {
        if (typeof item !== "object" || item === null) throw new DesktopItemContractError();
        const itemCandidate = item as Record<string, unknown>; // 等待字段校验的单项 Shell 结果。
        if (typeof itemCandidate.sourceReference !== "string" || itemCandidate.sourceReference.length === 0) throw new DesktopItemContractError();
        if (itemCandidate.resultReference !== null && typeof itemCandidate.resultReference !== "string") throw new DesktopItemContractError();
        if (typeof itemCandidate.succeeded !== "boolean" || !Number.isSafeInteger(itemCandidate.resultCode)) throw new DesktopItemContractError();
        return {
            sourceReference: itemCandidate.sourceReference,
            resultReference: itemCandidate.resultReference as string | null,
            succeeded: itemCandidate.succeeded,
            resultCode: itemCandidate.resultCode as number,
        };
    }); // 已完成契约校验的项目结果集合。
    return {
        operationId: candidate.operationId as number,
        completedAtUnixMicros: candidate.completedAtUnixMicros as number,
        kind: candidate.kind as ShellOperationKind,
        aborted: candidate.aborted,
        reversible: candidate.reversible,
        permanent: candidate.permanent,
        effect: candidate.effect as string | null,
        items,
        message: candidate.message,
    };
};
