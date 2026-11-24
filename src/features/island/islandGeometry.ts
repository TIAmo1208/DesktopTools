import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";

/** 依实测卡片形成一次提交；透明间隙也命中，不填充可见表面。 */
export const islandGeometry = (
    state: IslandStateRequest["state"],
    measuredInput: IslandRect[],
    radius: number,
    work: IslandRect,
    edge: "" | "top" | "bottom" | "left" | "right",
    offset: number,
    form: "capsule" | "notch" = "capsule",
    options: { anchorStart?: number } = {},
): IslandStateRequest => {
    let measured = measuredInput; // 展开态在下面统一改写列宽。
    if (!measured.length) throw new Error("岛缺少实测卡片");
    // 展开态列宽由页面固定：首帧、动画帧、重测与重同步都从这一处取值，
    // 否则内容自然宽会随写入宽度重排，出现 280→288→314→342 的分次长大与末帧回退（t69–t73）。
    if (state === "expanded") measured = measured.map((rect) => ({ ...rect, width: ISLAND_EXPANDED_COLUMN_WIDTH }));
    const gutter = state === "expanded" ? ISLAND_SCROLLBAR_GUTTER : 0; // 展开态在卡片外留一条滚动条槽。
    const gutterStart = edge === "right" ? gutter : 0; // 贴右边时槽在卡片左侧，保持卡片贴边。
    const gutterEnd = gutter - gutterStart;
    const contentWidth = Math.ceil(Math.max(...measured.map((rect) => rect.left + rect.width))); // 卡片右边界即内容宽，含贴右边时的起始槽。
    const columnWidth = contentWidth - gutterStart; // 纯卡片内容宽：贴右边时扣掉起始槽，沿边居中只看它，卡片坐标与槽显隐无关。
    const width = contentWidth + gutterEnd; // 窗口在内容之外补上滚动条槽。
    const height = state === "expanded" ? work.height : Math.ceil(Math.max(...measured.map((rect) => rect.top + rect.height)));
    if (width > work.width || height > work.height) throw new Error("岛内容超过主屏工作区");
    // 展开内容在页面中滚动；只向原生提交工作区视口内的卡片区域。
    const visible = state === "expanded" ? measured.flatMap((rect) => visibleIslandRect(rect, height)) : measured;
    if (!visible.length) throw new Error("岛缺少实测卡片");
    const horizontalEdge = edge === "left" || edge === "right"; // 左右贴边时沿竖直方向居中，上下贴边时沿水平方向居中。
    const margin = form === "notch" ? 0 : 6; // 胶囊离工作区边 6px，刘海贴边。
    const notch = form === "notch" && state !== "expanded" ? 12 : 0; // 外凸负圆角只在常态与 hover 生效；展开态不额外扩大窗口。
    const alongSpan = horizontalEdge ? work.height : work.width; // 沿边方向的工作区尺寸。
    const alongSize = (horizontalEdge ? height : columnWidth) + notch * 2; // 沿边居中按卡片内容宽计算，滚动条槽不参与居中。
    const alongGutter = horizontalEdge ? 0 : gutter; // 槽只在上/下贴边时占用沿边方向。
    const 半幅 = Math.max(0, (alongSpan - alongSize) / 2); // 内容在沿边方向的居中半幅；偏移以居中为 0。
    const maxOffset = Math.max(0, 半幅 - alongGutter); // 槽占着沿边方向的末端，那一侧可移动范围相应减少。
    const alongOffset = Math.min(Math.max(offset, -半幅), maxOffset); // 空边按顶部同样应用持久偏移，不再强制居中。
    // 内容按自身宽度居中，窗口向槽一侧长出去；极端偏移夹住内容起点，保证窗口仍在工作区内。
    const alongStart = Math.min(Math.max(半幅 + alongOffset, 0), Math.max(0, alongSpan - alongSize - alongGutter)); // 极端偏移只夹紧内容起点，窗口始终留在工作区内。
    const centeredLeft = edge === "right" ? work.left + work.width - width - margin : work.left + (horizontalEdge ? margin : alongStart);
    // 自然宽切换（胶囊/悬停消息切换、relay 逐帧收放）：把窗口左缘锚定在上一帧，宽度向右扩展，
    // 避免“先按新宽居中、再随原生 margin 回执跳回”的横向抖动。展开态不锚定（列已固定）。
    const anchoredLeft = options.anchorStart !== undefined && state !== "expanded"
        ? Math.min(Math.max(options.anchorStart, work.left), work.left + work.width - width)
        : undefined;
    const left = anchoredLeft ?? centeredLeft;
    const top = state === "expanded" ? work.top : edge === "bottom" ? work.top + work.height - height - margin : work.top + (horizontalEdge ? alongStart : margin);
    // 刘海的窗口在沿边方向各向外扩 12px，本体向内让出同样的距离；被让出的角部是新增透明像素。
    const growLeft = horizontalEdge ? 0 : notch; // 上下贴边时在水平方向让出圆弧区。
    const growTop = horizontalEdge ? notch : 0; // 左右贴边时在竖直方向让出圆弧区。
    const cards = visible.map((rect) => ({
        rect: { ...rect, left: rect.left + growLeft, top: rect.top + growTop },
        radius: Math.min(radius, rect.width / 2, rect.height / 2),
    }));
    if (state === "expanded") {
        for (let index = 1; index < visible.length; index++) {
            const previous = visible[index - 1]!;
            const current = visible[index]!;
            const gap = current.top - previous.top - previous.height;
            if (gap > 0) cards.push({ rect: { left: growLeft, top: previous.top + previous.height + growTop, width: contentWidth, height: gap }, radius: 0 });
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
        viewport: islandViewport(work, edge, offset, state),
        cards,
    };
};

/** 内容列相对页面视口的偏移；展开首帧必须先用它定位再提交窗口，避免 (0,0) 中间态。 */
export const islandContentOffset = ({ windowBox, viewport }: Pick<IslandStateRequest, "windowBox" | "viewport">): { x: number; y: number } => ({
    x: windowBox.left - viewport.left,
    y: windowBox.top - viewport.top,
});

/** 展开态只在窗口盒或固定视口变化时提交，不把列表内部卡片变化当成新窗口。 */
/** 展开卡片列在满工作区根内的左边距；逐帧绘制与终帧恢复共用同一算式，避免最后一帧跳列。 */
export const islandExpandedColumn = ({ windowBox, viewport }: Pick<IslandStateRequest, "windowBox" | "viewport">): number => windowBox.left - viewport.left;

export const islandWindowKey = ({ state, windowBox, viewport }: IslandStateRequest): string => JSON.stringify([state, windowBox, viewport]);

/** 展开态卡片外侧的滚动条槽：4 DIP 间距加 12 DIP 轨道。 */
export const ISLAND_SCROLLBAR_GUTTER = 16;

/** 展开内容列的固定列宽：切换消息 1/全部只改高度，列偏移与窗口盒不随文字宽变化。 */
export const ISLAND_EXPANDED_COLUMN_WIDTH = 400;

/** 固定页面视口带宽：覆盖最大窗口宽度（展开卡 400 + 滚动条槽 16 + 刘海 24）并留余量。 */
export const ISLAND_VIEWPORT_WIDTH = 440;

/** 紧凑态（胶囊/hover）窗口宽度上限：沿用原 420 视口带的可见宽度，带宽扩到 440 只用于容纳展开列。 */
export const ISLAND_COMPACT_MAX_WIDTH = 420;

/**
 * 固定页面原点：视口按“最大 440 宽窗口”使用与窗口相同的边/偏移落位并同样夹紧，
 * 因此同一偏移下所有状态窗口（含较宽的展开与并集）都落在视口内；状态切换不移动视口。
 */
export const islandViewport = (work: IslandRect, edge: string, offset: number, state: IslandStateRequest["state"] = "collapsed"): IslandRect => {
    // 展开态页面铺满工作区，整片透明区都能收到指针事件；紧凑两态仍用 440 视口带。
    if (state === "expanded") return { ...work };
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

/** 交换完整保存顺序里的两张可见卡；未显示的消息槽与其它未参与项保持原位置。 */
export const moveIslandVisibleCard = (order: readonly string[], from: string, to: string): string[] => {
    const next = [...order];
    const source = next.indexOf(from);
    const target = next.indexOf(to);
    if (source < 0 || target < 0 || source === target) return next;
    [next[source], next[target]] = [next[target], next[source]];
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
