import { TILE_MIN_HEIGHT, TILE_MIN_WIDTH } from "../../contracts/tile.ts";
import type { WorkspaceRect } from "../../contracts/workspace.ts";

/** 围栏八方向缩放边框。 */
export type TileResizeDirection = "n" | "ne" | "e" | "se" | "s" | "sw" | "w" | "nw";

/** 指针相对交互起点的位移。 */
interface PointerDelta {
    x: number;
    y: number;
}

/** 当前透明宿主工作区尺寸。 */
interface WorkspaceSize {
    width: number;
    height: number;
}

/** 内容可以提高外框下限，但不能突破公共数据底线或提供无效尺寸。 */
export const resolveTileMinimum = (minimum: WorkspaceSize = { width: TILE_MIN_WIDTH, height: TILE_MIN_HEIGHT }): WorkspaceSize => {
    if (![minimum.width, minimum.height].every((size) => Number.isFinite(size) && size > 0)) throw new Error("格子最小尺寸无效");
    return { width: Math.max(TILE_MIN_WIDTH, minimum.width), height: Math.max(TILE_MIN_HEIGHT, minimum.height) };
};

/** 当前围栏边缘参与吸附的方向。 */
type TileSnapEdges = Partial<Record<"left" | "right" | "top" | "bottom", boolean>>;

/** 将围栏边缘吸附到工作区或其他围栏边缘。 */
export const snapTileEdges = (
    rect: WorkspaceRect,
    targets: WorkspaceRect[],
    workspace: WorkspaceSize,
    threshold = 8,
    edges?: TileSnapEdges,
    minimum: WorkspaceSize = { width: TILE_MIN_WIDTH, height: TILE_MIN_HEIGHT },
): WorkspaceRect => {
    minimum = resolveTileMinimum(minimum); // 直接算法调用与公共交互使用相同下限。
    const result = { ...rect };
    const xTargets = [0, workspace.width, ...targets.flatMap((target) => [target.left, target.left + target.width])];
    const yTargets = [0, workspace.height, ...targets.flatMap((target) => [target.top, target.top + target.height])];
    const nearest = (value: number, candidates: number[]) =>
        candidates.reduce<{ value: number; distance: number } | undefined>((best, candidate) => {
            const distance = Math.abs(candidate - value);
            return distance <= threshold && (!best || distance < best.distance) ? { value: candidate, distance } : best;
        }, undefined);

    if (edges) {
        const right = rect.left + rect.width;
        const bottom = rect.top + rect.height;
        const leftSnap = edges.left ? nearest(rect.left, xTargets) : undefined;
        const rightSnap = edges.right ? nearest(right, xTargets) : undefined;
        const topSnap = edges.top ? nearest(rect.top, yTargets) : undefined;
        const bottomSnap = edges.bottom ? nearest(bottom, yTargets) : undefined;
        if (leftSnap) {
            result.left = Math.min(leftSnap.value, right - minimum.width);
            result.width = right - result.left;
        }
        if (rightSnap) result.width = Math.max(minimum.width, rightSnap.value - result.left);
        if (topSnap) {
            result.top = Math.min(topSnap.value, bottom - minimum.height);
            result.height = bottom - result.top;
        }
        if (bottomSnap) result.height = Math.max(minimum.height, bottomSnap.value - result.top);
        return result;
    }

    const leftSnap = nearest(rect.left, xTargets);
    const rightSnap = nearest(rect.left + rect.width, xTargets);
    const topSnap = nearest(rect.top, yTargets);
    const bottomSnap = nearest(rect.top + rect.height, yTargets);
    const xDelta =
        leftSnap && rightSnap
            ? leftSnap.distance <= rightSnap.distance
                ? leftSnap.value - rect.left
                : rightSnap.value - (rect.left + rect.width)
            : leftSnap
              ? leftSnap.value - rect.left
              : rightSnap
                ? rightSnap.value - (rect.left + rect.width)
                : 0;
    const yDelta =
        topSnap && bottomSnap
            ? topSnap.distance <= bottomSnap.distance
                ? topSnap.value - rect.top
                : bottomSnap.value - (rect.top + rect.height)
            : topSnap
              ? topSnap.value - rect.top
              : bottomSnap
                ? bottomSnap.value - (rect.top + rect.height)
                : 0;
    result.left = Math.max(0, Math.min(workspace.width - result.width, result.left + xDelta));
    result.top = Math.max(0, Math.min(workspace.height - result.height, result.top + yDelta));
    return result;
};

/** 将移动后的围栏完整约束在当前工作区。 */
export const moveTile = (start: WorkspaceRect, delta: PointerDelta, workspace: WorkspaceSize): WorkspaceRect => ({
    ...start,
    left: Math.max(0, Math.min(workspace.width - start.width, start.left + delta.x)),
    top: Math.max(0, Math.min(workspace.height - start.height, start.top + delta.y)),
});

/** 按八方向规则缩放围栏并保持最小尺寸与工作区边界。 */
export const resizeTile = (
    direction: TileResizeDirection,
    start: WorkspaceRect,
    delta: PointerDelta,
    workspace: WorkspaceSize,
    minimum: WorkspaceSize = { width: TILE_MIN_WIDTH, height: TILE_MIN_HEIGHT },
): WorkspaceRect => {
    minimum = resolveTileMinimum(minimum); // 不允许覆盖参数绕过数据底线。
    // 等待当前缩放方向修改的围栏边界。
    const result = { ...start };
    // 缩放开始时的固定右边界。
    const right = start.left + start.width;
    // 缩放开始时的固定下边界。
    const bottom = start.top + start.height;

    if (direction.includes("e")) result.width = Math.max(minimum.width, Math.min(workspace.width - start.left, start.width + delta.x));
    if (direction.includes("s")) result.height = Math.max(minimum.height, Math.min(workspace.height - start.top, start.height + delta.y));
    if (direction.includes("w")) {
        result.left = Math.max(0, Math.min(right - minimum.width, start.left + delta.x));
        result.width = right - result.left;
    }
    if (direction.includes("n")) {
        result.top = Math.max(0, Math.min(bottom - minimum.height, start.top + delta.y));
        result.height = bottom - result.top;
    }
    return result;
};
