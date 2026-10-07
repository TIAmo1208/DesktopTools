import type { CollectItem } from "../../../bridge/collect";

/** 外部缺失保留当前会话线索；主动删除的文件必须先从缓存剔除。 */
export const mergeCollectItems = (current: CollectItem[], next: CollectItem[], deletedName?: string): CollectItem[] => [
    ...next.filter((item) => item.name !== deletedName),
    ...current
        .filter((item) => item.name !== deletedName && !next.some((candidate) => candidate.name === item.name))
        .map((item) => ({ ...item, available: false })),
];
