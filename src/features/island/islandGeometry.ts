import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";

/** 依实测卡片形成一次提交；透明间隙也命中，不填充可见表面。 */
export const islandGeometry = (
    state: IslandStateRequest["state"],
    measured: IslandRect[],
    radius: number,
    work: IslandRect,
    edge: "" | "top" | "bottom" | "left" | "right",
    offset: number,
): IslandStateRequest => {
    if (!measured.length) throw new Error("岛缺少实测卡片");
    const width = Math.ceil(Math.max(...measured.map((rect) => rect.left + rect.width)));
    const height = Math.ceil(Math.max(...measured.map((rect) => rect.top + rect.height)));
    if (width > work.width || height > work.height) throw new Error("岛内容超过主屏工作区");
    const vertical = edge === "left" || edge === "right"; // 沿边偏移不随展开态尺寸重新居中。
    const along = Math.min(Math.max(0, edge === "" ? (work.width - width) / 2 : offset), Math.max(0, vertical ? work.height - height : work.width - width));
    const left = edge === "right" ? work.left + work.width - width : work.left + (vertical ? 0 : along);
    const top = edge === "bottom" ? work.top + work.height - height : work.top + (vertical ? along : 0);
    const cards = measured.map((rect) => ({ rect, radius: Math.min(radius, rect.width / 2, rect.height / 2) }));
    if (state === "expanded") {
        for (let index = 1; index < measured.length; index++) {
            const previous = measured[index - 1]!;
            const current = measured[index]!;
            const gap = current.top - previous.top - previous.height;
            if (gap > 0) cards.push({ rect: { left: 0, top: previous.top + previous.height, width, height: gap }, radius: 0 });
        }
    }
    return { state, windowBox: { left, top, width, height }, cards };
};

/** 展开列表超出工作区时，只提交滚动视口内的局部区域。 */
export const visibleIslandRect = (rect: IslandRect, height: number): IslandRect[] => {
    const top = Math.max(0, rect.top);
    const bottom = Math.min(height, rect.top + rect.height);
    return bottom > top ? [{ ...rect, top, height: bottom - top }] : [];
};

/** 排序只移动已有身份，不增删卡片。 */
export const moveIslandCard = (order: readonly string[], from: string, to: string): string[] => {
    const next = [...order];
    const source = next.indexOf(from);
    const target = next.indexOf(to);
    if (source < 0 || target < 0 || source === target) return next;
    next.splice(source, 1);
    next.splice(target, 0, from);
    return next;
};
