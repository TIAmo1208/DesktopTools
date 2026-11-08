import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";

/** 依实测卡片形成一次提交；透明间隙也命中，不填充可见表面。 */
export const islandGeometry = (
    state: IslandStateRequest["state"],
    measured: IslandRect[],
    radius: number,
    work: IslandRect,
    edge: "" | "top" | "bottom" | "left" | "right",
    offset: number,
    form: "capsule" | "notch" = "capsule",
): IslandStateRequest => {
    if (!measured.length) throw new Error("岛缺少实测卡片");
    const width = Math.ceil(Math.max(...measured.map((rect) => rect.left + rect.width)));
    const height = Math.ceil(Math.max(...measured.map((rect) => rect.top + rect.height)));
    if (width > work.width || height > work.height) throw new Error("岛内容超过主屏工作区");
    const horizontalEdge = edge === "left" || edge === "right"; // 左右贴边时沿竖直方向居中，上下贴边时沿水平方向居中。
    const margin = form === "notch" ? 0 : 6; // 胶囊离工作区边 6px，刘海贴边。
    const notch = form === "notch" && state !== "expanded" ? 12 : 0; // 外凸负圆角只在常态与 hover 生效；展开态不额外扩大窗口。
    const alongSpan = horizontalEdge ? work.height : work.width; // 沿边方向的工作区尺寸。
    const alongSize = (horizontalEdge ? height : width) + notch * 2; // 外凸圆弧占用的沿边尺寸。
    const maxOffset = Math.max(0, (alongSpan - alongSize) / 2); // 以居中为 0 的可偏移半幅。
    const alongOffset = Math.min(Math.max(offset, -maxOffset), maxOffset); // 空边按顶部同样应用持久偏移，不再强制居中。
    const alongStart = Math.max(0, alongSpan - alongSize) / 2 + alongOffset; // 居中后再应用持久偏移；尺寸变化时两侧对称扩展。
    const left = edge === "right" ? work.left + work.width - width - margin : work.left + (horizontalEdge ? margin : alongStart);
    const top = edge === "bottom" ? work.top + work.height - height - margin : work.top + (horizontalEdge ? alongStart : margin);
    // 刘海的窗口在沿边方向各向外扩 12px，本体向内让出同样的距离；被让出的角部是新增透明像素。
    const growLeft = horizontalEdge ? 0 : notch; // 上下贴边时在水平方向让出圆弧区。
    const growTop = horizontalEdge ? notch : 0; // 左右贴边时在竖直方向让出圆弧区。
    const cards = measured.map((rect) => ({ rect: { ...rect, left: rect.left + growLeft, top: rect.top + growTop }, radius: Math.min(radius, rect.width / 2, rect.height / 2) }));
    if (state === "expanded") {
        for (let index = 1; index < measured.length; index++) {
            const previous = measured[index - 1]!;
            const current = measured[index]!;
            const gap = current.top - previous.top - previous.height;
            if (gap > 0) cards.push({ rect: { left: growLeft, top: previous.top + previous.height + growTop, width, height: gap }, radius: 0 });
        }
    }
    return {
        state,
        windowBox: {
            left,
            top,
            width: width + (horizontalEdge ? 0 : notch * 2),
            height: height + (horizontalEdge ? notch * 2 : 0),
        },
        cards,
    };
};

/** 刘海在非展开态把贴边侧两角切成直角，由窗口角部的外凸圆弧补面衔接屏幕边；展开态统一使用岛圆角。 */
export const islandCornerRadius = (radius: number, form: "capsule" | "notch", edge: string, expanded = false): string => {
    const value = `${radius}px`;
    if (form !== "notch" || expanded) return value;
    switch (edge) {
        case "bottom":
            return `${value} ${value} 0 0`;
        case "left":
            return `0 ${value} ${value} 0`;
        case "right":
            return `${value} 0 0 ${value}`;
        default:
            return `0 0 ${value} ${value}`;
    }
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
