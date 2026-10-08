import type { CollectItem } from "../../../bridge/collect";

export type CollectBoxLayout = "mixed" | "separated";
export type CollectBoxFilter = Record<CollectItem["kind"], boolean>;
export interface CollectBoxState {
    layout: CollectBoxLayout;
    filters: CollectBoxFilter;
}

export const defaultCollectBoxState: CollectBoxState = {
    layout: "mixed",
    filters: { text: true, image: true, file: true },
};

/** 未配置状态使用当前默认值，不迁移旧格式。 */
export const readCollectBoxState = (value: unknown): CollectBoxState => {
    const state = value && typeof value === "object" ? (value as Partial<CollectBoxState>) : {};
    return {
        layout: state.layout === "separated" ? "separated" : "mixed",
        filters: {
            text: typeof state.filters?.text === "boolean" ? state.filters.text : true,
            image: typeof state.filters?.image === "boolean" ? state.filters.image : true,
            file: typeof state.filters?.file === "boolean" ? state.filters.file : true,
        },
    };
};

/** 外部缺失保留当前会话线索；主动删除的文件必须先从缓存剔除。 */
export const mergeCollectItems = (current: CollectItem[], next: CollectItem[], deletedName?: string): CollectItem[] => [
    ...next.filter((item) => item.name !== deletedName),
    ...current
        .filter((item) => item.name !== deletedName && !next.some((candidate) => candidate.name === item.name))
        .map((item) => ({ ...item, available: false })),
];

/** 按置顶、布局和当前排序生成稳定呈现顺序。 */
export const orderCollectItems = (items: CollectItem[], layout: CollectBoxLayout, sort: "time" | "name") => {
    const ordered = [...items].sort((a, b) =>
        b.pinned === a.pinned ? (sort === "time" ? b.createdUnixMs - a.createdUnixMs : a.name.localeCompare(b.name)) : b.pinned ? 1 : -1,
    );
    if (layout === "mixed") return ordered;
    return (["text", "image", "file"] as const).flatMap((kind) => ordered.filter((item) => item.kind === kind));
};

/** 筛选不可见条目，并限制选择集合只包含当前可见项。 */
export const filterCollectItems = (items: CollectItem[], filters: CollectBoxFilter) => items.filter((item) => filters[item.kind]);
