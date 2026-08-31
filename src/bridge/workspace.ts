import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import { parseWorkspaceSnapshot, type WorkspaceCommand, type WorkspaceSnapshot } from "../contracts/workspace";
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
export const applyWorkspaceCommand = async (command: WorkspaceCommand): Promise<WorkspaceSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("apply_workspace_command", { command }).catch((error: unknown) => {
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
