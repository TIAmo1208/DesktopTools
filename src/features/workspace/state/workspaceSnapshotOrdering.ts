import type { WorkspaceSnapshot } from "../../../contracts/workspace";

/** 判断收到的工作区快照是否至少包含当前状态。 */
export const isWorkspaceSnapshotNewer = (current: WorkspaceSnapshot | undefined, next: WorkspaceSnapshot): boolean => {
    if (!current) return true;
    return next.revision > current.revision || (next.revision === current.revision && next.interaction.revision > current.interaction.revision);
};
