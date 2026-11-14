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
    const cards = measured.map((rect) => ({
        rect: { ...rect, left: rect.left + growLeft, top: rect.top + growTop },
        radius: Math.min(radius, rect.width / 2, rect.height / 2),
    }));
    if (state === "expanded") {
        for (let index = 1; index < measured.length; index++) {
            const previous = measured[index - 1]!;
            const current = measured[index]!;
            const gap = current.top - previous.top - previous.height;
            if (gap > 0) cards.push({ rect: { left: growLeft, top: previous.top + previous.height + growTop, width, height: gap }, radius: 0 });
        }
    }
    const windowBox = {
        left,
        top,
        width: width + (horizontalEdge ? 0 : notch * 2),
        height: height + (horizontalEdge ? notch * 2 : 0),
    };
    return {
        state,
        windowBox,
        viewport: islandViewport(work, edge, offset),
        cards,
    };
};

/** 固定页面视口带宽：覆盖最大窗口宽度（展开卡 360 + 刘海 24）并留余量。 */
export const ISLAND_VIEWPORT_WIDTH = 420;

/**
 * 固定页面原点：视口按“最大 420 宽窗口”使用与窗口相同的边/偏移落位并同样夹紧，
 * 因此同一偏移下所有状态窗口（含较宽的展开与并集）都落在视口内；状态切换不移动视口。
 */
export const islandViewport = (work: IslandRect, edge: string, offset: number): IslandRect => {
    const horizontal = edge === "left" || edge === "right"; // 左右贴边时视口贴左右边，窗口沿竖直方向居中。
    if (horizontal)
        return {
            left: edge === "right" ? work.left + work.width - ISLAND_VIEWPORT_WIDTH : work.left,
            top: work.top,
            width: ISLAND_VIEWPORT_WIDTH,
            height: work.height,
        };
    const span = work.width; // 上下贴边时沿边方向是水平方向。
    const maxOffset = Math.max(0, (span - ISLAND_VIEWPORT_WIDTH) / 2); // 视口按最大宽度夹紧，不能按具体窗口中心。
    const start = Math.max(0, span - ISLAND_VIEWPORT_WIDTH) / 2 + Math.min(Math.max(offset, -maxOffset), maxOffset);
    return {
        left: work.left + start,
        top: work.top,
        width: ISLAND_VIEWPORT_WIDTH,
        height: work.height,
    };
};

/** 刘海非展开态贴边侧的外凸负圆角半径，单位 DIP。 */
export const ISLAND_NOTCH_FILLET = 12;

/** 贴边侧角的带符号圆角：负数表示外凸负圆角，正数表示普通圆角，胶囊形态恒为普通圆角。 */
export const islandAttachedCorner = (state: IslandStateRequest["state"], form: "capsule" | "notch", radius: number): number =>
    form === "notch" && state !== "expanded" ? -ISLAND_NOTCH_FILLET : radius;

/** 补面沿卡片贴边侧向内重叠的宽度，覆盖分数设备像素上的合成接缝。 */
/** 一块外凸圆弧补面：`slot` 为沿边方向的起点侧或终点侧，`rect` 是窗口局部坐标，`radius` 是遮罩圆弧半径。 */
export interface IslandFilletPatch {
    slot: "start" | "end";
    radius: number;
    rect: IslandRect;
}

/**
 * 按卡片贴边角定位两块圆弧补面，使补面跟随卡片而不是窗口角；
 * 沿边方向到窗口边缘的余量不足时收紧半径，补面不会画出原生窗口。
 */
export const islandFilletPatches = (card: IslandRect, windowBox: IslandRect, edge: string, radius: number): IslandFilletPatch[] => {
    if (!(radius > 0)) return [];
    const horizontal = edge === "left" || edge === "right"; // 左右贴边的沿边方向是竖直的。
    const startRoom = horizontal ? card.top : card.left; // 起点侧到窗口边缘的可用余量。
    const endRoom = horizontal ? windowBox.height - card.top - card.height : windowBox.width - card.left - card.width;
    const start = Math.min(radius, Math.max(0, startRoom));
    const end = Math.min(radius, Math.max(0, endRoom));
    const patches: IslandFilletPatch[] = [];
    // 这里只返回理想几何；接缝补采样由islandSeamMask在同一轮廓内裁剪，不改变圆弧轮廓。
    const push = (slot: IslandFilletPatch["slot"], size: number, left: number, top: number) => {
        if (size > 0.01) patches.push({ slot, radius: size, rect: { left, top, width: size, height: size } });
    };
    switch (edge) {
        case "bottom":
            push("start", start, card.left - start, card.top + card.height - start);
            push("end", end, card.left + card.width, card.top + card.height - end);
            break;
        case "left":
            push("start", start, card.left, card.top - start);
            push("end", end, card.left, card.top + card.height);
            break;
        case "right":
            push("start", start, card.left + card.width - start, card.top - start);
            push("end", end, card.left + card.width - end, card.top + card.height);
            break;
        default:
            push("start", start, card.left - start, card.top);
            push("end", end, card.left + card.width, card.top);
    }
    return patches;
};

/** 根尺寸SVG内的单路径理想并集及接缝裁剪带；实际新增覆盖为path∩bands，不是整块带状矩形。 */
export const islandSeamMask = (card: IslandRect, corners: number[], windowBox: IslandRect, edge: string, form: "capsule" | "notch", attached: number, dpr: number): { path: string; bands: IslandRect[] } | undefined => {
    if (form !== "notch" || attached >= -0.01) return undefined;
    if (!(Number.isFinite(dpr) && dpr > 0)) throw new RangeError("mask设备像素比例必须为正有限数");
    const patches = islandFilletPatches(card, windowBox, edge, -attached);
    if (!patches.length) return undefined;
    const parts = [islandCardMaskPath(card.width, card.height, corners, card.left, card.top)];
    const bands: IslandRect[] = [];
    const effectiveEdge = edge === "bottom" || edge === "left" || edge === "right" ? edge : "top";
    const centers = { top: [[0, 1], [1, 1]], bottom: [[0, 0], [1, 0]], left: [[1, 0], [1, 1]], right: [[0, 0], [0, 1]] };
    // 接缝两侧各1 CSS px实心采样区；至少留半CSS像素的SVG本征栅格余量，同时覆盖半设备像素。
    const half = 1 + Math.max(0.5, 0.5 / dpr);
    for (const patch of patches) {
        const { left: x, top: y, width, height } = patch.rect;
        const right = x + width, bottom = y + height, radius = patch.radius;
        const [cx, cy] = centers[effectiveEdge][patch.slot === "start" ? 0 : 1]!;
        const arc = (px: number, py: number) => `A ${radius} ${radius} 0 0 0 ${px} ${py}`;
        if (cx === 0 && cy === 1) parts.push(`M ${x} ${y} L ${right} ${y} L ${right} ${bottom} ${arc(x, y)} Z`);
        else if (cx === 1 && cy === 1) parts.push(`M ${x} ${y} L ${right} ${y} ${arc(x, bottom)} Z`);
        else if (cx === 0 && cy === 0) parts.push(`M ${right} ${y} L ${right} ${bottom} L ${x} ${bottom} ${arc(right, y)} Z`);
        else parts.push(`M ${x} ${y} ${arc(right, bottom)} L ${x} ${bottom} Z`);
        if (effectiveEdge === "left" || effectiveEdge === "right") {
            const join = patch.slot === "start" ? card.top : card.top + card.height;
            bands.push({ left: x, top: join - half, width, height: half * 2 });
        } else {
            const join = patch.slot === "start" ? card.left : card.left + card.width;
            bands.push({ left: join - half, top: y, width: half * 2, height });
        }
    }
    return { path: parts.join(" "), bands };
};

/** 刘海在非展开态把贴边侧两角切成直角，由贴着卡片角的圆弧补面衔接屏幕边；展开态统一使用岛圆角。 */
export const islandCornerRadius = (radius: number, form: "capsule" | "notch", edge: string, attached = 0): string => {
    if (form !== "notch") return `${radius}px`;
    const shown = Math.max(0, Math.min(attached, radius)); // 贴边角不能比同卡其余角更圆。
    if (shown >= radius && radius >= 0) return `${radius}px`;
    const value = `${radius}px`;
    const corner = `${shown}px`; // 带符号半径为正时补面消失，由贴边角自身过渡。
    switch (edge) {
        case "bottom":
            return `${value} ${value} ${corner} ${corner}`;
        case "left":
            return `${corner} ${value} ${value} ${corner}`;
        case "right":
            return `${value} ${corner} ${corner} ${value}`;
        default:
            return `${corner} ${corner} ${value} ${value}`;
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

/**
 * 卡片自身的单层遮罩路径（卡片局部坐标，左上为原点）：四角半径按贴边角/其余角分别给值；
 * 补面帧贴边角为 0，外凸圆弧由同一 surface 的另外两层遮罩叠加，不再画第二块半透明表面。
 */
export const islandCardMaskPath = (width: number, height: number, corners: number[], left = 0, top = 0): string => {
    const [tl = 0, tr = 0, br = 0, bl = 0] = corners;
    const line = (x: number, y: number) => `L ${x + left} ${y + top}`;
    const arc = (radius: number, x: number, y: number) => (radius > 0.01 ? `A ${radius} ${radius} 0 0 1 ${x + left} ${y + top}` : line(x, y));
    return `M ${tl + left} ${top} ${line(width - tr, 0)} ${arc(tr, width, tr)} ${line(width, height - br)} ${arc(br, width - br, height)} ${line(bl, height)} ${arc(bl, 0, height - bl)} ${line(0, tl)} ${arc(tl, tl, 0)} Z`;
};
