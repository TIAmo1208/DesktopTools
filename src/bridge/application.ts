import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError, parseApplicationSnapshot, type ApplicationSnapshot, type ApplicationStatus } from "../contracts/application";
import { invokeRuntime, listenRuntime } from "./runtime";

// Rust 发布的应用快照事件名称。
export const APPLICATION_SNAPSHOT_CHANGED_EVENT = "application-snapshot-changed";

/** 获取 Rust 持有的当前应用快照。 */
export const getApplicationSnapshot = async (): Promise<ApplicationSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("get_application_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });

    return parseApplicationSnapshot(response);
};

/** 提交应用状态并校验 command 返回值。 */
export const setApplicationStatus = async (status: ApplicationStatus): Promise<ApplicationSnapshot> => {
    // 等待契约校验的 command 返回值。
    const response = await invokeRuntime<unknown>("set_application_status", { status }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });

    return parseApplicationSnapshot(response);
};

/** 订阅 Rust 发布的应用快照并校验 event 载荷。 */
export const listenApplicationSnapshots = (onSnapshot: (snapshot: ApplicationSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(APPLICATION_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseApplicationSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
