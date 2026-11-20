/** 收集项排序只需要身份、类型、时间与置顶；桌面格子与浮岛卡共用同一顺序。 */
export interface CollectViewItem {
    name: string;
    kind: "text" | "image" | "file";
    createdUnixMs: number;
    pinned: boolean;
}

export type CollectViewLayout = "mixed" | "separated";

/** 按置顶、布局和当前排序生成稳定呈现顺序。 */
export const orderCollectViewItems = <T extends CollectViewItem>(items: readonly T[], layout: CollectViewLayout, sort: "time" | "name"): T[] => {
    const ordered = [...items].sort((a, b) => (b.pinned === a.pinned ? (sort === "time" ? b.createdUnixMs - a.createdUnixMs : a.name.localeCompare(b.name)) : b.pinned ? 1 : -1));
    if (layout === "mixed") return ordered;
    return (["text", "image", "file"] as const).flatMap((kind) => ordered.filter((item) => item.kind === kind));
};
