import { FENCE_MIN_HEIGHT, FENCE_MIN_WIDTH, type WorkspaceRect } from "../../contracts/workspace.ts";

/** 围栏八方向缩放边框。 */
export type FenceResizeDirection = "n" | "ne" | "e" | "se" | "s" | "sw" | "w" | "nw";

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

/** 将移动后的围栏完整约束在当前工作区。 */
export const moveFence = (start: WorkspaceRect, delta: PointerDelta, workspace: WorkspaceSize): WorkspaceRect => ({
    ...start,
    left: Math.max(0, Math.min(workspace.width - start.width, start.left + delta.x)),
    top: Math.max(0, Math.min(workspace.height - start.height, start.top + delta.y)),
});

/** 按原型八方向规则缩放围栏并保持最小尺寸与工作区边界。 */
export const resizeFence = (direction: FenceResizeDirection, start: WorkspaceRect, delta: PointerDelta, workspace: WorkspaceSize): WorkspaceRect => {
    // 等待当前缩放方向修改的围栏边界。
    const result = { ...start };
    // 缩放开始时的固定右边界。
    const right = start.left + start.width;
    // 缩放开始时的固定下边界。
    const bottom = start.top + start.height;

    if (direction.includes("e")) result.width = Math.max(FENCE_MIN_WIDTH, Math.min(workspace.width - start.left, start.width + delta.x));
    if (direction.includes("s")) result.height = Math.max(FENCE_MIN_HEIGHT, Math.min(workspace.height - start.top, start.height + delta.y));
    if (direction.includes("w")) {
        result.left = Math.max(0, Math.min(right - FENCE_MIN_WIDTH, start.left + delta.x));
        result.width = right - result.left;
    }
    if (direction.includes("n")) {
        result.top = Math.max(0, Math.min(bottom - FENCE_MIN_HEIGHT, start.top + delta.y));
        result.height = bottom - result.top;
    }
    return result;
};
