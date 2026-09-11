import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import {
    parseWorkspaceBackupSummary,
    parseWorkspaceHistoryResult,
    parseWorkspaceRestoreResult,
    parseWorkspaceSnapshot,
    type WorkspaceBackupSummary,
    type WorkspaceCommand,
    type WorkspaceContextMenuTarget,
    type WorkspaceHistoryResult,
    type WorkspaceInteractionCommand,
    type WorkspaceRestoreResult,
    type WorkspaceSnapshot,
} from "../contracts/workspace";
import { invokeRuntime, listenRuntime } from "./runtime";

// Rust 发布的工作区快照事件名称。
export const WORKSPACE_SNAPSHOT_CHANGED_EVENT = "workspace-snapshot-changed";

/** 获取当前不可变工作区快照。 */
export const getWorkspaceSnapshot = async (): Promise<WorkspaceSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("get_workspace_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceSnapshot(response);
};

/** 提交一个布局事务并返回新的权威工作区快照。 */
export const applyWorkspaceCommand = async (monitorId: string, command: WorkspaceCommand): Promise<WorkspaceSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("apply_workspace_command", { monitorId, command }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceSnapshot(response);
};

/** 按完成时间撤销统一时间线中的最近布局或 Shell 操作。 */
export const undoWorkspace = async (): Promise<WorkspaceHistoryResult> => {
    const response = await invokeRuntime<unknown>("undo_workspace").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的统一撤销结果。
    return parseWorkspaceHistoryResult(response);
};

/** 按最近撤销时间重做统一时间线中的布局或 Shell 操作。 */
export const redoWorkspace = async (): Promise<WorkspaceHistoryResult> => {
    const response = await invokeRuntime<unknown>("redo_workspace").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    }); // 等待契约校验的统一重做结果。
    return parseWorkspaceHistoryResult(response);
};

/** 将当前完整工作区原子写入手动备份文件。 */
export const writeWorkspaceBackup = async (path: string): Promise<WorkspaceBackupSummary> => {
    const response = await invokeRuntime<unknown>("write_workspace_backup", { path }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceBackupSummary(response);
};

/** 只读迁移并校验手动备份或自动快照。 */
export const inspectWorkspaceBackup = async (path: string): Promise<WorkspaceBackupSummary> => {
    const response = await invokeRuntime<unknown>("inspect_workspace_backup", { path }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceBackupSummary(response);
};

/** 返回当前仍保留且通过完整性校验的全部自动快照。 */
export const listWorkspaceAutoSnapshots = async (): Promise<WorkspaceBackupSummary[]> => {
    const response = await invokeRuntime<unknown>("list_workspace_auto_snapshots").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    if (!Array.isArray(response)) throw new Error("工作区自动快照数据无效");
    return response.map(parseWorkspaceBackupSummary);
};

/** 事务式还原统一工作区备份。 */
export const restoreWorkspaceBackup = async (path: string): Promise<WorkspaceRestoreResult> => {
    const response = await invokeRuntime<unknown>("restore_workspace_backup", { path }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceRestoreResult(response);
};

/** 提交一个运行期交互事务并返回新的权威工作区快照。 */
export const applyWorkspaceInteraction = async (monitorId: string, command: WorkspaceInteractionCommand): Promise<WorkspaceSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("apply_workspace_interaction", { monitorId, command }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceSnapshot(response);
};

/** 在当前语义目标显示原生菜单并返回产品动作后的工作区快照。 */
export const showWorkspaceContextMenu = async (
    monitorId: string,
    target: WorkspaceContextMenuTarget,
    clientX: number,
    clientY: number,
): Promise<WorkspaceSnapshot> => {
    // 等待契约校验的原生菜单 command 返回值。
    const response = await invokeRuntime<unknown>("show_workspace_context_menu", { monitorId, target, clientX, clientY }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseWorkspaceSnapshot(response);
};

/** 订阅 Rust 发布的工作区快照并校验 event 载荷。 */
export const listenWorkspaceSnapshots = (onSnapshot: (snapshot: WorkspaceSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(WORKSPACE_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseWorkspaceSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
