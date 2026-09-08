import { parseComponentInstanceSnapshot, type ComponentInstanceSnapshot } from "./component.ts";
import { parseShellOperationResult, type ShellOperationResult } from "./desktopItem.ts";

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

/** 一台显示器对应的独立未归类网格配置。 */
export interface WorkspaceMonitorGrid {
    monitorId: string;
    grid: WorkspaceGrid;
}

/** 围栏成员的排列方式。 */
export type FenceLayoutMode = "tile" | "list";

/** 围栏自动排序方式。 */
export type FenceSortMode = "none" | "name-ascending" | "name-descending";

/** 工作区持久化的完整围栏状态。 */
export interface FenceSnapshot {
    id: string;
    title: string;
    monitorId: string;
    projectionMonitorId: string | null;
    projectionBounds: WorkspaceRect | null;
    bounds: WorkspaceRect;
    members: string[];
    layoutMode: FenceLayoutMode;
    sortMode?: FenceSortMode; // 旧快照省略时使用无排序。
    scrollTop: number;
    collapsed: boolean;
    autoExpand?: boolean; // 折叠时悬停临时展开，旧布局默认关闭。
    lastExpandedHeight: number;
    zIndex: number;
}

/** 未归类桌面项目进入围栏前保留的位置。 */
export interface UnclassifiedItemSnapshot {
    stableReference: string;
    monitorId: string;
    projectionMonitorId: string | null;
    projectionPosition: WorkspacePoint | null;
    position: WorkspacePoint;
    zIndex: number;
}

/** 一次项目拖动提交的未归类网格位置。 */
export interface WorkspaceItemPlacement {
    itemId: string;
    position: WorkspacePoint;
}

/** 一次项目拖动提交的围栏滚动位置。 */
export interface WorkspaceFenceScroll {
    fenceId: string;
    scrollTop: number;
}

/** 当前接收键盘命令的桌面焦点目标。 */
export type WorkspaceFocusTarget =
    { type: "background" } | { type: "item"; itemId: string } | { type: "fence"; fenceId: string } | { type: "component"; componentId: string };

/** 当前虚拟桌面的运行期选择、锚点和键盘焦点。 */
export interface WorkspaceInteractionSnapshot {
    revision: number;
    selectedItemIds: string[];
    selectionAnchorId: string | null;
    keyboardFocus: WorkspaceFocusTarget;
}

/** Rust 发布给 React 的不可变工作区快照。 */
export interface WorkspaceSnapshot {
    schemaVersion: number;
    revision: number;
    grid: WorkspaceGrid;
    monitorGrids: WorkspaceMonitorGrid[];
    fences: FenceSnapshot[];
    unclassifiedItems: UnclassifiedItemSnapshot[];
    components: ComponentInstanceSnapshot[];
    interaction: WorkspaceInteractionSnapshot;
}

/** 一个已校验工作区备份或自动快照的摘要。 */
export interface WorkspaceBackupSummary {
    schemaVersion: number;
    createdAtUnix: number;
    path: string;
    layoutCount: number;
    archiveCount: number;
    checksum: string;
}

/** 完整工作区备份事务式还原结果。 */
export interface WorkspaceRestoreResult {
    snapshot: WorkspaceSnapshot;
    restoredLayoutCount: number;
    message: string;
}

/** 统一撤销时间线实际处理的历史单元类型。 */
export type WorkspaceHistoryKind = "layout" | "shell" | "none";

/** 统一撤销时间线的一次命令结果。 */
export interface WorkspaceHistoryResult {
    snapshot: WorkspaceSnapshot;
    kind: WorkspaceHistoryKind;
    shellOperation: ShellOperationResult | null;
    message: string;
}

/** 项目焦点变化时应用的选择规则。 */
export type WorkspaceSelectionMode = "replace" | "toggle" | "range" | "preserve";

/** 原生上下文菜单对应的工作区语义目标。 */
export type WorkspaceContextMenuTarget = { type: "items"; itemIds: string[] } | { type: "fence"; fenceId: string } | { type: "background" };

/** React 可以提交给 Workspace Domain 的运行期交互命令。 */
export type WorkspaceInteractionCommand =
    | { type: "focus-item"; itemId: string; selectionMode: WorkspaceSelectionMode }
    | { type: "select-items"; itemIds: string[] }
    | { type: "focus-surface"; focus: Exclude<WorkspaceFocusTarget, { type: "item" }> }
    | { type: "select-focus-container" };

/** React 可以提交给工作区领域的完整命令集合。 */
export type WorkspaceCommand =
    | { type: "create-fence"; title: string; bounds: WorkspaceRect }
    | { type: "update-fence-bounds"; fenceId: string; bounds: WorkspaceRect }
    | { type: "set-fence-collapsed"; fenceId: string; collapsed: boolean }
    | { type: "set-fence-layout"; fenceId: string; layoutMode: FenceLayoutMode }
    | { type: "set-fence-sort"; fenceId: string; sortMode: FenceSortMode }
    | { type: "rename-fence"; fenceId: string; title: string }
    | { type: "set-fence-auto-expand"; fenceId: string; enabled: boolean }
    | { type: "set-fence-scroll"; fenceId: string; scrollTop: number }
    | { type: "move-item-to-fence"; itemId: string; fenceId: string; beforeItemId: string | null }
    | { type: "remove-item-from-fence"; itemId: string }
    | { type: "dissolve-fence"; fenceId: string }
    | {
          type: "commit-item-drag";
          anchorItemId: string;
          itemIds: string[];
          placements: WorkspaceItemPlacement[];
          targetFenceId: string | null;
          beforeItemId: string | null;
          fenceScrolls: WorkspaceFenceScroll[];
      }
    | { type: "update-component-bounds"; instanceId: string; bounds: WorkspaceRect }
    | { type: "update-component-state"; instanceId: string; stateVersion: number; state: unknown }
    | { type: "undo-layout" }
    | { type: "redo-layout" };

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

/** 校验当前键盘焦点目标。 */
const parseFocusTarget = (value: unknown): WorkspaceFocusTarget => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();

    // 等待字段校验的焦点载荷。
    const candidate = value as Record<string, unknown>;
    if (candidate.type === "background") return { type: "background" };
    if (candidate.type === "item" && typeof candidate.itemId === "string" && candidate.itemId.length > 0) return { type: "item", itemId: candidate.itemId };
    if (candidate.type === "fence" && typeof candidate.fenceId === "string" && candidate.fenceId.length > 0)
        return { type: "fence", fenceId: candidate.fenceId };
    if (candidate.type === "component" && typeof candidate.componentId === "string" && candidate.componentId.length > 0)
        return { type: "component", componentId: candidate.componentId };
    throw new WorkspaceContractError();
};

/** 校验当前运行期交互快照。 */
const parseInteraction = (value: unknown): WorkspaceInteractionSnapshot => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();

    // 等待字段校验的交互载荷。
    const candidate = value as Record<string, unknown>;
    if (!Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0) throw new WorkspaceContractError();
    if (!Array.isArray(candidate.selectedItemIds) || candidate.selectedItemIds.some((itemId) => typeof itemId !== "string" || itemId.length === 0))
        throw new WorkspaceContractError();
    if (candidate.selectionAnchorId !== null && (typeof candidate.selectionAnchorId !== "string" || candidate.selectionAnchorId.length === 0))
        throw new WorkspaceContractError();
    return {
        revision: candidate.revision as number,
        selectedItemIds: [...candidate.selectedItemIds] as string[],
        selectionAnchorId: candidate.selectionAnchorId as string | null,
        keyboardFocus: parseFocusTarget(candidate.keyboardFocus),
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
        if (
            fenceCandidate.sortMode !== undefined &&
            fenceCandidate.sortMode !== "none" &&
            fenceCandidate.sortMode !== "name-ascending" &&
            fenceCandidate.sortMode !== "name-descending"
        )
            throw new WorkspaceContractError();
        if (!isFiniteNumber(fenceCandidate.scrollTop) || fenceCandidate.scrollTop < 0) throw new WorkspaceContractError();
        if (typeof fenceCandidate.collapsed !== "boolean") throw new WorkspaceContractError();
        if (fenceCandidate.autoExpand !== undefined && typeof fenceCandidate.autoExpand !== "boolean") throw new WorkspaceContractError();
        if (!isFiniteNumber(fenceCandidate.lastExpandedHeight) || fenceCandidate.lastExpandedHeight <= 0) throw new WorkspaceContractError();
        if (!Number.isSafeInteger(fenceCandidate.zIndex) || (fenceCandidate.zIndex as number) < 0) throw new WorkspaceContractError();

        // 已完成契约校验的围栏布局模式。
        const layoutMode: FenceLayoutMode = fenceCandidate.layoutMode;

        return {
            id: fenceCandidate.id,
            title: fenceCandidate.title,
            monitorId: typeof fenceCandidate.monitorId === "string" ? fenceCandidate.monitorId : "",
            projectionMonitorId: typeof fenceCandidate.projectionMonitorId === "string" ? fenceCandidate.projectionMonitorId : null,
            projectionBounds: fenceCandidate.projectionBounds ? parseRect(fenceCandidate.projectionBounds) : null,
            bounds: parseRect(fenceCandidate.bounds),
            members: [...fenceCandidate.members] as string[],
            layoutMode,
            sortMode: (fenceCandidate.sortMode ?? "none") as FenceSortMode,
            autoExpand: fenceCandidate.autoExpand === true,
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
        if (itemCandidate.zIndex !== undefined && (!Number.isSafeInteger(itemCandidate.zIndex) || (itemCandidate.zIndex as number) < 0))
            throw new WorkspaceContractError();
        return {
            stableReference: itemCandidate.stableReference,
            monitorId: typeof itemCandidate.monitorId === "string" ? itemCandidate.monitorId : "",
            projectionMonitorId: typeof itemCandidate.projectionMonitorId === "string" ? itemCandidate.projectionMonitorId : null,
            projectionPosition: itemCandidate.projectionPosition ? parsePoint(itemCandidate.projectionPosition) : null,
            position: parsePoint(itemCandidate.position),
            zIndex: (itemCandidate.zIndex as number | undefined) ?? 0,
        };
    });

    const monitorGrids = Array.isArray(candidate.monitorGrids)
        ? candidate.monitorGrids.map((entry) => {
              if (typeof entry !== "object" || entry === null) throw new WorkspaceContractError();
              const gridCandidate = entry as Record<string, unknown>; // 等待字段校验的逐屏网格载荷。
              if (typeof gridCandidate.monitorId !== "string" || gridCandidate.monitorId.length === 0) throw new WorkspaceContractError();
              return { monitorId: gridCandidate.monitorId, grid: parseGrid(gridCandidate.grid) };
          })
        : []; // 已完成契约校验的逐屏网格集合。
    const components = Array.isArray(candidate.components) ? candidate.components.map(parseComponentInstanceSnapshot) : []; // 已完成契约校验的组件实例集合。
    return {
        schemaVersion: 1,
        revision: candidate.revision as number,
        grid: parseGrid(candidate.grid),
        monitorGrids,
        fences,
        unclassifiedItems,
        components,
        interaction: parseInteraction(candidate.interaction),
    };
};

/** 校验工作区备份摘要。 */
export const parseWorkspaceBackupSummary = (value: unknown): WorkspaceBackupSummary => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的备份摘要。
    if (candidate.schemaVersion !== 3) throw new WorkspaceContractError();
    if (!Number.isSafeInteger(candidate.createdAtUnix) || (candidate.createdAtUnix as number) < 0) throw new WorkspaceContractError();
    if (typeof candidate.path !== "string" || candidate.path.length === 0) throw new WorkspaceContractError();
    if (!Number.isSafeInteger(candidate.layoutCount) || (candidate.layoutCount as number) <= 0) throw new WorkspaceContractError();
    if (!Number.isSafeInteger(candidate.archiveCount) || (candidate.archiveCount as number) < 0) throw new WorkspaceContractError();
    if (typeof candidate.checksum !== "string" || !candidate.checksum.startsWith("fnv1a64:")) throw new WorkspaceContractError();
    return {
        schemaVersion: 3,
        createdAtUnix: candidate.createdAtUnix as number,
        path: candidate.path,
        layoutCount: candidate.layoutCount as number,
        archiveCount: candidate.archiveCount as number,
        checksum: candidate.checksum,
    };
};

/** 校验完整工作区还原结果。 */
export const parseWorkspaceRestoreResult = (value: unknown): WorkspaceRestoreResult => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的完整还原结果。
    if (!Number.isSafeInteger(candidate.restoredLayoutCount) || (candidate.restoredLayoutCount as number) <= 0) throw new WorkspaceContractError();
    if (typeof candidate.message !== "string") throw new WorkspaceContractError();
    return {
        snapshot: parseWorkspaceSnapshot(candidate.snapshot),
        restoredLayoutCount: candidate.restoredLayoutCount as number,
        message: candidate.message,
    };
};

/** 校验统一撤销或重做结果。 */
export const parseWorkspaceHistoryResult = (value: unknown): WorkspaceHistoryResult => {
    if (typeof value !== "object" || value === null) throw new WorkspaceContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的统一历史结果。
    if (candidate.kind !== "layout" && candidate.kind !== "shell" && candidate.kind !== "none") throw new WorkspaceContractError();
    if (typeof candidate.message !== "string") throw new WorkspaceContractError();
    return {
        snapshot: parseWorkspaceSnapshot(candidate.snapshot),
        kind: candidate.kind,
        shellOperation: candidate.shellOperation === null ? null : parseShellOperationResult(candidate.shellOperation),
        message: candidate.message,
    };
};
