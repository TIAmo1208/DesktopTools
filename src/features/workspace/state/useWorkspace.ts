import { useCallback, useEffect, useRef, useState } from "react";

import {
    applyWorkspaceCommand,
    applyWorkspaceInteraction,
    getWorkspaceSnapshot,
    listenWorkspaceSnapshots,
    redoWorkspace,
    showWorkspaceContextMenu,
    undoWorkspace,
} from "../../../bridge/workspace";
import type { WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceInteractionCommand, WorkspaceSnapshot } from "../../../contracts/workspace";
import { isWorkspaceSnapshotNewer } from "./workspaceSnapshotOrdering";

/** 将未知错误转换为工作区用户可见说明。 */
const workspaceErrorMessage = (error: unknown): string => (error instanceof Error ? error.message : "工作区请求失败");

/** 订阅权威工作区快照并暴露布局事务命令。 */
export const useWorkspace = (enabled: boolean) => {
    const [snapshot, setSnapshot] = useState<WorkspaceSnapshot>(); // 当前权威工作区快照。
    const [failure, setFailure] = useState<string>(); // 当前工作区命令或事件错误。
    const [status, setStatus] = useState<string>(); // 当前统一撤销或重做的用户可见结果。
    const snapshotRef = useRef<WorkspaceSnapshot | undefined>(undefined); // 防止轮询旧快照覆盖最新命令结果。

    /** 仅提交不早于当前状态的权威快照。 */
    const commitSnapshot = useCallback((nextSnapshot: WorkspaceSnapshot) => {
        if (!isWorkspaceSnapshotNewer(snapshotRef.current, nextSnapshot)) return;
        snapshotRef.current = nextSnapshot;
        setSnapshot(nextSnapshot);
        setFailure(undefined);
    }, []);

    useEffect(() => {
        if (!enabled) return;
        let isMounted = true; // 当前工作区页面挂载标记。

        // 接收 Rust 发布的权威工作区快照。
        const handleSnapshot = (nextSnapshot: WorkspaceSnapshot) => {
            if (!isMounted) return;
            commitSnapshot(nextSnapshot);
        };
        // 呈现工作区 command、event 或契约错误。
        const handleFailure = (error: unknown) => {
            if (isMounted) setFailure(workspaceErrorMessage(error));
        };
        // 工作区快照事件订阅。
        const subscription = listenWorkspaceSnapshots(handleSnapshot, handleFailure);
        void subscription.catch(handleFailure);
        void getWorkspaceSnapshot().then(handleSnapshot).catch(handleFailure);
        return () => {
            isMounted = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [commitSnapshot, enabled]);

    /** 提交工作区命令并立即采用返回的权威快照。 */
    const applyCommand = useCallback(
        async (monitorId: string, command: WorkspaceCommand) => {
            try {
                const historyResult =
                    command.type === "undo-layout" ? await undoWorkspace() : command.type === "redo-layout" ? await redoWorkspace() : undefined; // 当前统一时间线的可选结果。
                const nextSnapshot = historyResult?.snapshot ?? (await applyWorkspaceCommand(monitorId, command)); // 已持久化的权威工作区快照。
                setStatus(historyResult?.message);
                commitSnapshot(nextSnapshot);
                return nextSnapshot;
            } catch (error: unknown) {
                setFailure(workspaceErrorMessage(error));
                throw error;
            }
        },
        [commitSnapshot],
    );

    /** 提交运行期交互命令并立即采用返回的权威快照。 */
    const applyInteraction = useCallback(
        async (monitorId: string, command: WorkspaceInteractionCommand) => {
            try {
                const nextSnapshot = await applyWorkspaceInteraction(monitorId, command); // 已提交到内存的权威工作区快照。
                commitSnapshot(nextSnapshot);
                return nextSnapshot;
            } catch (error: unknown) {
                setFailure(workspaceErrorMessage(error));
                throw error;
            }
        },
        [commitSnapshot],
    );

    /** 显示原生工作区菜单并采用产品动作返回的权威快照。 */
    const showContextMenu = useCallback(
        async (monitorId: string, target: WorkspaceContextMenuTarget, clientX: number, clientY: number) => {
            try {
                const nextSnapshot = await showWorkspaceContextMenu(monitorId, target, clientX, clientY); // 原生菜单关闭后的权威工作区快照。
                commitSnapshot(nextSnapshot);
                return nextSnapshot;
            } catch (error: unknown) {
                setFailure(workspaceErrorMessage(error));
                throw error;
            }
        },
        [commitSnapshot],
    );

    return { snapshot, failure, status, applyCommand, applyInteraction, showContextMenu };
};
