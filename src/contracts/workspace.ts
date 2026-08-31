/** 围栏允许的最小宽度，单位为 DIP。 */
export const FENCE_MIN_WIDTH = 164;

/** 围栏允许的最小展开高度，单位为 DIP。 */
export const FENCE_MIN_HEIGHT = 154;

/** 工作区中的二维位置。 */
export interface WorkspacePoint {
    left: number;
    top: number;
}

/** 围栏在单显示器工作区中的边界。 */
export interface WorkspaceRect extends WorkspacePoint {
    width: number;
    height: number;
}

/** Rust Workspace 发布的未归类网格配置。 */
export interface WorkspaceGrid {
    left: number;
    top: number;
    columnWidth: number;
    rowHeight: number;
    rows: number;
    columns: number;
}

/** 围栏成员的排列方式。 */
export type FenceLayoutMode = "tile" | "list";

/** 工作区持久化的完整围栏状态。 */
export interface FenceSnapshot {
    id: string;
    title: string;
    bounds: WorkspaceRect;
    members: string[];
    layoutMode: FenceLayoutMode;
    scrollTop: number;
    collapsed: boolean;
    lastExpandedHeight: number;
    zIndex: number;
}

/** 未归类桌面项目进入围栏前保留的位置。 */
export interface UnclassifiedItemSnapshot {
    stableReference: string;
    position: WorkspacePoint;
}

/** Rust 发布给 React 的不可变工作区快照。 */
export interface WorkspaceSnapshot {
    schemaVersion: number;
    revision: number;
    grid: WorkspaceGrid;
    fences: FenceSnapshot[];
    unclassifiedItems: UnclassifiedItemSnapshot[];
}

/** React 可以提交给工作区领域的完整命令集合。 */
export type WorkspaceCommand =
    | { type: "create-fence"; title: string; bounds: WorkspaceRect }
    | { type: "update-fence-bounds"; fenceId: string; bounds: WorkspaceRect }
    | { type: "set-fence-collapsed"; fenceId: string; collapsed: boolean }
    | { type: "set-fence-layout"; fenceId: string; layoutMode: FenceLayoutMode }
    | { type: "set-fence-scroll"; fenceId: string; scrollTop: number }
    | { type: "move-item-to-fence"; itemId: string; fenceId: string; beforeItemId: string | null }
    | { type: "remove-item-from-fence"; itemId: string }
    | { type: "dissolve-fence"; fenceId: string };

/** 标识工作区 command/event 载荷错误。 */
export class WorkspaceContractError extends Error {
    readonly code = "invalid-workspace-snapshot"; // 可稳定检索的契约错误码。

    /** 创建工作区契约错误。 */
    constructor() {
        super("工作区数据无效");
        this.name = "WorkspaceContractError";
    }
}

/** 判断未知值是否为有限数字。 */
const isFiniteNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

/** 校验工作区二维位置。 */
const parsePoint = (value: unknown): WorkspacePoint => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();

    // 等待字段校验的位置载荷。
    const candidate = value as Record<string, unknown>;
    if (!isFiniteNumber(candidate.left) || !isFiniteNumber(candidate.top)) throw new WorkspaceContractError();
    return { left: candidate.left, top: candidate.top };
};

/** 校验围栏边界。 */
const parseRect = (value: unknown): WorkspaceRect => {
    // 已完成基础坐标校验的边界位置。
    const point = parsePoint(value);
    // 等待尺寸字段校验的边界载荷。
    const candidate = value as Record<string, unknown>;
    if (!isFiniteNumber(candidate.width) || candidate.width <= 0 || !isFiniteNumber(candidate.height) || candidate.height <= 0)
        throw new WorkspaceContractError();
    return { ...point, width: candidate.width, height: candidate.height };
};

/** 校验 Rust Workspace 发布的未归类网格配置。 */
const parseGrid = (value: unknown): WorkspaceGrid => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();

    // 等待字段校验的网格配置载荷。
    const candidate = value as Record<string, unknown>;
    if (!isFiniteNumber(candidate.left) || !isFiniteNumber(candidate.top)) throw new WorkspaceContractError();
    if (!isFiniteNumber(candidate.columnWidth) || candidate.columnWidth <= 0) throw new WorkspaceContractError();
    if (!isFiniteNumber(candidate.rowHeight) || candidate.rowHeight <= 0) throw new WorkspaceContractError();
    if (!Number.isSafeInteger(candidate.rows) || (candidate.rows as number) <= 0) throw new WorkspaceContractError();
    if (!Number.isSafeInteger(candidate.columns) || (candidate.columns as number) <= 0) throw new WorkspaceContractError();
    return {
        left: candidate.left,
        top: candidate.top,
        columnWidth: candidate.columnWidth,
        rowHeight: candidate.rowHeight,
        rows: candidate.rows as number,
        columns: candidate.columns as number,
    };
};

/** 校验 command 返回值与 event 载荷并生成工作区快照。 */
export const parseWorkspaceSnapshot = (value: unknown): WorkspaceSnapshot => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();

    // 等待字段校验的工作区快照载荷。
    const candidate = value as Record<string, unknown>;
    if (candidate.schemaVersion !== 1 || !Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0) throw new WorkspaceContractError();
    if (!Array.isArray(candidate.fences) || !Array.isArray(candidate.unclassifiedItems)) throw new WorkspaceContractError();

    // 已完成字段校验的围栏集合。
    const fences = candidate.fences.map((fence) => {
        if (typeof fence !== "object" || fence === null) throw new WorkspaceContractError();

        // 等待字段校验的围栏载荷。
        const fenceCandidate = fence as Record<string, unknown>;
        if (typeof fenceCandidate.id !== "string" || fenceCandidate.id.length === 0) throw new WorkspaceContractError();
        if (typeof fenceCandidate.title !== "string" || fenceCandidate.title.length === 0) throw new WorkspaceContractError();
        if (!Array.isArray(fenceCandidate.members) || fenceCandidate.members.some((member) => typeof member !== "string" || member.length === 0))
            throw new WorkspaceContractError();
        if (fenceCandidate.layoutMode !== "tile" && fenceCandidate.layoutMode !== "list") throw new WorkspaceContractError();
        if (!isFiniteNumber(fenceCandidate.scrollTop) || fenceCandidate.scrollTop < 0) throw new WorkspaceContractError();
        if (typeof fenceCandidate.collapsed !== "boolean") throw new WorkspaceContractError();
        if (!isFiniteNumber(fenceCandidate.lastExpandedHeight) || fenceCandidate.lastExpandedHeight <= 0) throw new WorkspaceContractError();
        if (!Number.isSafeInteger(fenceCandidate.zIndex) || (fenceCandidate.zIndex as number) < 0) throw new WorkspaceContractError();

        // 已完成契约校验的围栏布局模式。
        const layoutMode: FenceLayoutMode = fenceCandidate.layoutMode;

        return {
            id: fenceCandidate.id,
            title: fenceCandidate.title,
            bounds: parseRect(fenceCandidate.bounds),
            members: [...fenceCandidate.members] as string[],
            layoutMode,
            scrollTop: fenceCandidate.scrollTop,
            collapsed: fenceCandidate.collapsed,
            lastExpandedHeight: fenceCandidate.lastExpandedHeight,
            zIndex: fenceCandidate.zIndex as number,
        };
    });

    // 已完成字段校验的未归类项目位置集合。
    const unclassifiedItems = candidate.unclassifiedItems.map((item) => {
        if (typeof item !== "object" || item === null) throw new WorkspaceContractError();

        // 等待字段校验的未归类项目载荷。
        const itemCandidate = item as Record<string, unknown>;
        if (typeof itemCandidate.stableReference !== "string" || itemCandidate.stableReference.length === 0) throw new WorkspaceContractError();
        return { stableReference: itemCandidate.stableReference, position: parsePoint(itemCandidate.position) };
    });

    return { schemaVersion: 1, revision: candidate.revision as number, grid: parseGrid(candidate.grid), fences, unclassifiedItems };
};
