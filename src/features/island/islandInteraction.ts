import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";
import { islandFilletPatches } from "./islandGeometry.ts";

// 控件、作者内容和显式禁拖区域不参与窗口拖动或卡片排序。
export const ISLAND_NO_DRAG = "button, input, select, textarea, a, [contenteditable], [data-plugin-container], [data-island-plugin], [data-island-no-drag]";

// ponytail: 双向暂用 180ms 固定延迟，后续由设置页参数项接管，不扩大偏好契约。
export const ISLAND_HOVER_DELAY_MS = 180;
export const ISLAND_DRAG_DELAY_MS = 250; // 按住满 250ms 才进入原生拖动，避免单击与拖动手势抢跑。
export const ISLAND_DRAG_THRESHOLD = 4; // 屏幕 DIP 位移，不按按住时长猜测拖动。

/** 位移越过阈值后取消长按拖动，避免拖动和普通操作抢跑。 */
export const islandDragMoved = (start: { x: number; y: number }, point: { x: number; y: number }) =>
    Math.hypot(point.x - start.x, point.y - start.y) > ISLAND_DRAG_THRESHOLD;

/** 取消挂起的 hover 切换；释放重判后旧定时器不得重新拉起 hover。 */
export const cancelIslandHover = (timer: { current: ReturnType<typeof setTimeout> | undefined }) => {
    clearTimeout(timer.current);
    timer.current = undefined;
};

/** 原生移动循环结束后按后端命中结果重新判态，不沿用开始态。 */
export const islandAfterDrag = (inside: boolean): "hover" | "collapsed" => (inside ? "hover" : "collapsed");

/** 延时读取当前命中与拖动态；反向经过边界会取消原定切换。 */
export const islandHoverTarget = (mode: IslandStateRequest["state"], inside: boolean, busy: boolean, latched = false): "hover" | "collapsed" | undefined =>
    busy || mode === "expanded" || mode === "collapsed" && latched ? undefined : inside && mode === "collapsed" ? "hover" : !inside && mode === "hover" ? "collapsed" : undefined;

export const ISLAND_MARGIN_DELAY_MS = 120; // hover透明区连续停留的退出延迟。
export const ISLAND_REENTRY_MARGIN = 4; // 门闩出入各留4 DIP滞回，进入仍180ms。
export const ISLAND_SCROLLBAR_FADE_MS = 150; // 条只做透明度过渡，不参与卡片形变。
export type IslandPoint = { x: number; y: number };
export type IslandRoundedCard = { rect: IslandRect; radii: number[] };
export type IslandContour = { cards: IslandRoundedCard[]; arcs: { rect: IslandRect; radius: number; center: IslandPoint }[] };
export type IslandHitRegion = "outside" | "content" | "transparent" | "extra-band" | "scrollbar";
export type IslandMarginState = { latch: "open" | "blocked" | "outside"; since?: number };

/** 左上含、右下不含；所有参数均为固定viewport内的CSS DIP。 */
export const islandPointInRect = (point: IslandPoint, rect: IslandRect) => point.x >= rect.left && point.y >= rect.top && point.x < rect.left + rect.width && point.y < rect.top + rect.height;
/** 圆角矩形有符号距离，负数在内容内；各角半径取实际DOM计算值。 */
export const islandCardDistance = (point: IslandPoint, card: IslandRoundedCard) => {
    const { left, top, width, height } = card.rect;
    const x = point.x - left - width / 2, y = point.y - top - height / 2;
    const index = y < 0 ? (x < 0 ? 0 : 1) : (x < 0 ? 3 : 2);
    const radius = Math.max(0, Math.min(card.radii[index] ?? card.radii[0] ?? 0, width / 2, height / 2));
    const qx = Math.abs(x) - width / 2 + radius, qy = Math.abs(y) - height / 2 + radius;
    return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - radius;
};
/** 轮廓保留圆角与真实弧面；方形patch内的透明扇区不算内容。 */
export const islandHitContour = (cards: IslandRoundedCard[], window: IslandRect, edge: string, form: "capsule" | "notch", attached: number): IslandContour => {
    const arcs: IslandContour["arcs"] = [];
    if (form === "notch" && attached < -0.01) for (const card of cards) {
        const local = { ...card.rect, left: card.rect.left - window.left, top: card.rect.top - window.top };
        for (const patch of islandFilletPatches(local, window, edge, -attached)) {
            const rect = { ...patch.rect, left: patch.rect.left + window.left, top: patch.rect.top + window.top };
            const horizontal = edge !== "left" && edge !== "right";
            const right = edge === "left" || horizontal && patch.slot === "end";
            const bottom = horizontal ? edge !== "bottom" : patch.slot === "end";
            arcs.push({ rect, radius: patch.radius, center: { x: rect.left + (right ? rect.width : 0), y: rect.top + (bottom ? rect.height : 0) } });
        }
    }
    return { cards, arcs };
};
/** 点到方形减四分之一圆的距离，用于真实弧面命中及4 DIP外侧滞回。 */
export const islandContourDistance = (point: IslandPoint, contour: IslandContour) => {
    const distances = contour.cards.map((card) => islandCardDistance(point, card));
    for (const { rect, radius, center } of contour.arcs) {
        const radial = Math.hypot(point.x - center.x, point.y - center.y);
        const inclusive = (p: IslandPoint) => p.x >= rect.left - 1e-8 && p.x <= rect.left + rect.width + 1e-8 && p.y >= rect.top - 1e-8 && p.y <= rect.top + rect.height + 1e-8;
        if (inclusive(point)) {
            distances.push(radial < radius ? radius - radial : -Math.min(radial - radius, point.x - rect.left, rect.left + rect.width - point.x, point.y - rect.top, rect.top + rect.height - point.y));
            continue;
        }
        const candidates = [
            { x: Math.max(rect.left, Math.min(point.x, rect.left + rect.width)), y: Math.max(rect.top, Math.min(point.y, rect.top + rect.height)) },
            ...[rect.left, rect.left + rect.width].flatMap((x) => [rect.top, rect.top + rect.height].map((y) => ({ x, y }))),
        ];
        if (radial > 0) candidates.push({ x: center.x + (point.x - center.x) * radius / radial, y: center.y + (point.y - center.y) * radius / radial });
        distances.push(Math.min(...candidates.filter((p) => inclusive(p) && Math.hypot(p.x - center.x, p.y - center.y) >= radius - 1e-8).map((p) => Math.hypot(point.x - p.x, point.y - p.y))));
    }
    return Math.min(...distances);
};
/** 只取沿贴边轴的额外带；正交历史余量、卡间隙和圆角透明像素不触发展开退回。 */
export const islandExtraBands = (window: IslandRect, target: IslandRect, edge: string): IslandRect[] => {
    const vertical = edge === "left" || edge === "right";
    const start = vertical ? window.top : window.left, length = vertical ? window.height : window.width;
    const from = vertical ? target.top : target.left, to = from + (vertical ? target.height : target.width);
    return [[start, Math.max(0, Math.min(start + length, from) - start)], [Math.max(start, to), Math.max(0, start + length - Math.max(start, to))]].filter(([, size]) => size! > 0).map(([position, size]) => vertical ? { ...window, top: position!, height: size! } : { ...window, left: position!, width: size! });
};
/** 同一次计算返回退回带/track/thumb；渲染和命中共用，不另读root.right。 */
export const islandScrollGeometry = (frame: IslandStateRequest, target: IslandStateRequest | undefined, edge: string, metrics: { mode: IslandStateRequest["state"]; settled: boolean; scrollHeight: number; clientHeight: number; scrollTop: number }) => {
    const client = (rect: IslandRect) => ({ ...rect, left: rect.left - frame.viewport.left, top: rect.top - frame.viewport.top });
    const window = client(frame.windowBox), content = client(target?.windowBox ?? frame.windowBox);
    const bands = islandExtraBands(window, content, edge);
    const maxScroll = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
    const visible = metrics.mode === "expanded" && metrics.settled && Number.isFinite(maxScroll) && maxScroll > 0 && window.height > 8;
    const track: IslandRect | undefined = visible ? { left: window.left + Math.max(0, window.width - 16), top: window.top + 4, width: 12, height: window.height - 8 } : undefined;
    const thumbHeight = track ? Math.min(track.height, Math.max(24, track.height * metrics.clientHeight / metrics.scrollHeight)) : 0;
    const travel = track ? track.height - thumbHeight : 0;
    const thumb: IslandRect | undefined = track ? { left: track.left + 3, top: track.top + travel * Math.max(0, Math.min(1, metrics.scrollTop / maxScroll)), width: 6, height: thumbHeight } : undefined;
    return { window, content, bands, visible, track, thumb, travel, maxScroll };
};
/** 保留原生矩形命中，只把批准的透明区作为页面侧退回特例。 */
export const islandHitRegion = (mode: IslandStateRequest["state"], point: IslandPoint, layout: ReturnType<typeof islandScrollGeometry>, contour: IslandContour): IslandHitRegion =>
    !islandPointInRect(point, layout.window) ? "outside" : layout.track && islandPointInRect(point, layout.track) ? "scrollbar" : mode === "expanded" && layout.bands.some((band) => islandPointInRect(point, band)) ? "extra-band" : islandContourDistance(point, contour) <= 1e-8 ? "content" : "transparent";
/** 120ms连续停留与门闩纯状态机；leave只代理窗外无法继续观察的指针，detect=false关闭常态透明检测。 */
export const islandMarginStep = (state: IslandMarginState, input: { mode: IslandStateRequest["state"]; region: IslandHitRegion; now: number; busy: boolean; outlineDistance: number; coreDistance: number; leave?: boolean; detect?: boolean }) => {
    // 常态透明边距不承载本次切态语义，且检测会与 180ms 进入、拖动释放重判冲突：关闭检测既不写门闩也不产生切态副作用，逻辑保留以便恢复。
    if (input.detect === false) return { state: { latch: state.latch } as IslandMarginState, collapse: false, canEnter: false };
    if (input.leave) return { state: { latch: state.latch === "open" ? "open" : "outside" } as IslandMarginState, collapse: false, canEnter: false };
    if (input.busy) return { state: { latch: state.latch } as IslandMarginState, collapse: false, canEnter: false };
    if (input.mode === "hover" || input.mode === "expanded") {
        // hover 的透明边距与展开的额外带共用同一条 120ms 连续停留计时，不另设超时。
        const delayRegion: IslandHitRegion = input.mode === "hover" ? "transparent" : "extra-band";
        const since = input.region === delayRegion ? state.since ?? input.now : undefined;
        const collapse = since !== undefined && input.now - since >= ISLAND_MARGIN_DELAY_MS;
        const latch: IslandMarginState["latch"] = input.mode === "hover" && collapse ? "blocked" : state.latch;
        return { state: { latch, since: collapse ? undefined : since } as IslandMarginState, collapse, canEnter: false };
    }
    let latch = state.latch;
    if (latch === "blocked" && input.outlineDistance >= ISLAND_REENTRY_MARGIN) latch = "outside";
    if (latch === "outside" && input.coreDistance <= -ISLAND_REENTRY_MARGIN) latch = "open";
    return { state: { latch } as IslandMarginState, collapse: false, canEnter: latch === "open" && input.region === "content" };
};
/** DEV参数显式启用且最多8张；发布路径忽略参数，不污染实例/排序偏好。 */
export const islandDebugCardCount = (development: boolean, search: string) => {
    if (!development) return 0;
    const value = Number(new URLSearchParams(search).get("island-debug") ?? 0);
    return Number.isFinite(value) ? Math.max(0, Math.min(8, Math.trunc(value))) : 0;
};
/**
 * 收起收口的唯一顺序：token 仍有效时先隐藏滚动条，再提交状态；
 * transitionend 与兜底计时共用它，已取消（反向请求或卸载）时两者都不执行。
 */
export const islandCollapseSettle = (token: number, activeToken: number, apply: { hide: () => void; commit: () => void }): boolean => {
    if (token !== activeToken) return false;
    apply.hide();
    apply.commit();
    return true;
};

/** 同一次按下只走一条拖动路径，歌词与媒体图不属于控件。 */
export const islandDragIntent = (mode: IslandStateRequest["state"], button: number, blocked: boolean, insideCard: boolean): "window" | "sort" | undefined => {
    if (button !== 0 || blocked || !insideCard) return undefined;
    return mode === "expanded" ? "sort" : "window";
};

/** 排序落点是卡片身份，不需要句柄；外部拖放与控件内落点不接管。 */
export const islandSortTarget = (
    mode: IslandStateRequest["state"],
    order: readonly string[],
    from: string | undefined,
    to: string,
    blocked: boolean,
): string | undefined => (mode === "expanded" && !blocked && from !== undefined && from !== to && order.includes(from) && order.includes(to) ? to : undefined);
