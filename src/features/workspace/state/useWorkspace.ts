import { useCallback, useEffect, useState } from "react";

import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../../bridge/workspace";
import type { WorkspaceCommand, WorkspaceSnapshot } from "../../../contracts/workspace";

/** 将未知错误转换为工作区用户可见说明。 */
const workspaceErrorMessage = (error: unknown): string => (error instanceof Error ? error.message : "工作区请求失败");

/** 订阅权威工作区快照并暴露布局事务命令。 */
export const useWorkspace = (enabled: boolean) => {
    const [snapshot, setSnapshot] = useState<WorkspaceSnapshot>(); // 当前权威工作区快照。
    const [failure, setFailure] = useState<string>(); // 当前工作区命令或事件错误。

    useEffect(() => {
        if (!enabled) return;
        let isMounted = true; // 当前工作区页面挂载标记。

        // 接收 Rust 发布的权威工作区快照。
        const handleSnapshot = (nextSnapshot: WorkspaceSnapshot) => {
            if (!isMounted) return;
            setSnapshot(nextSnapshot);
            setFailure(undefined);
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
    }, [enabled]);

    /** 提交工作区命令并立即采用返回的权威快照。 */
    const applyCommand = useCallback(async (command: WorkspaceCommand) => {
        try {
            const nextSnapshot = await applyWorkspaceCommand(command); // 已持久化的权威工作区快照。
            setSnapshot(nextSnapshot);
            setFailure(undefined);
            return nextSnapshot;
        } catch (error: unknown) {
            setFailure(workspaceErrorMessage(error));
            throw error;
        }
    }, []);

    return { snapshot, failure, applyCommand };
};
