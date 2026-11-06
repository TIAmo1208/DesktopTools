import type { IslandStateRequest } from "../../contracts/island.ts";

// 控件、作者内容和显式禁拖区域不参与窗口拖动或卡片排序。
export const ISLAND_NO_DRAG = "button, input, select, textarea, a, [contenteditable], [data-plugin-container], [data-island-plugin], [data-island-no-drag]";

// ponytail: 双向暂用 180ms 固定延迟，后续由设置页参数项接管，不扩大偏好契约。
export const ISLAND_HOVER_DELAY_MS = 180;
export const ISLAND_DRAG_THRESHOLD = 4; // 屏幕 DIP 位移，不按按住时长猜测拖动。

/** hover 只在真实位移越过阈值后进入原生移动循环。 */
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
export const islandHoverTarget = (mode: IslandStateRequest["state"], inside: boolean, busy: boolean): "hover" | "collapsed" | undefined =>
    busy || mode === "expanded" ? undefined : inside && mode === "collapsed" ? "hover" : !inside && mode === "hover" ? "collapsed" : undefined;

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
