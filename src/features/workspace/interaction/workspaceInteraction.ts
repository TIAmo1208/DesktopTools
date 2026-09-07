import type { WorkspaceFocusTarget, WorkspaceSelectionMode } from "../../../contracts/workspace";

/** 选择规则需要的键盘修饰键。 */
interface SelectionModifiers {
    ctrlKey: boolean;
    shiftKey: boolean;
}

/** 工作区中的矩形边界。 */
export interface InteractionRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** 可参与工作区空间导航的项目。 */
export interface InteractionItem {
    id: string;
    rect: InteractionRect;
}

/** 可参与高层 Tab 顺序的工作区表面。 */
export interface InteractionSurface {
    target: Exclude<WorkspaceFocusTarget, { type: "item" }>;
    rect: InteractionRect;
    zIndex: number;
}

/** 工作区支持的空间导航方向。 */
export type NavigationDirection = "up" | "right" | "down" | "left";

/** 将桌面项目指针输入映射为选择规则。 */
export const selectionModeForPointer = ({ ctrlKey, shiftKey }: SelectionModifiers): WorkspaceSelectionMode => {
    if (shiftKey) return "range";
    return ctrlKey ? "toggle" : "replace";
};

/** 将方向键输入映射为选择规则。 */
export const selectionModeForDirectionalKey = ({ ctrlKey, shiftKey }: SelectionModifiers): WorkspaceSelectionMode => {
    if (shiftKey) return "range";
    return ctrlKey ? "preserve" : "replace";
};

/** 按输入视觉顺序返回与框选矩形相交的项目。 */
export const itemIdsIntersectingRect = (selection: InteractionRect, items: InteractionItem[]): string[] => {
    const selectionRight = selection.left + selection.width; // 框选区域右边界。
    const selectionBottom = selection.top + selection.height; // 框选区域下边界。
    return items
        .map((item, index) => ({ ...item, index }))
        .filter(({ rect }) => {
            const right = rect.left + rect.width; // 项目右边界。
            const bottom = rect.top + rect.height; // 项目下边界。
            return rect.left <= selectionRight && right >= selection.left && rect.top <= selectionBottom && bottom >= selection.top;
        })
        .sort((first, second) => first.rect.top - second.rect.top || first.rect.left - second.rect.left || first.index - second.index)
        .map(({ id }) => id);
};

/** 返回当前焦点容器中指定空间方向的最近项目。 */
export const nearestDirectionalItemId = (items: InteractionItem[], currentId: string, direction: NavigationDirection): string | undefined => {
    const current = items.find(({ id }) => id === currentId); // 当前获得项目焦点的空间节点。
    if (!current) return undefined;
    const currentX = current.rect.left + current.rect.width / 2; // 当前项目中心横坐标。
    const currentY = current.rect.top + current.rect.height / 2; // 当前项目中心纵坐标。

    return items
        .filter(({ id }) => id !== currentId)
        .map((item, index) => {
            const deltaX = item.rect.left + item.rect.width / 2 - currentX; // 候选项目中心水平位移。
            const deltaY = item.rect.top + item.rect.height / 2 - currentY; // 候选项目中心垂直位移。
            const primary = direction === "left" ? -deltaX : direction === "right" ? deltaX : direction === "up" ? -deltaY : deltaY; // 目标方向主轴距离。
            const secondary = direction === "left" || direction === "right" ? Math.abs(deltaY) : Math.abs(deltaX); // 垂直于目标方向的偏移。
            // 候选项目与当前项目在主轴垂直方向上的投影相交状态。
            const projectionIntersects =
                direction === "left" || direction === "right"
                    ? item.rect.top <= current.rect.top + current.rect.height && item.rect.top + item.rect.height >= current.rect.top
                    : item.rect.left <= current.rect.left + current.rect.width && item.rect.left + item.rect.width >= current.rect.left;
            const distanceSquared = deltaX * deltaX + deltaY * deltaY; // 当前项目中心到候选项目中心的平方距离。
            return { id: item.id, primary, secondary, projectionIntersects, distanceSquared, index };
        })
        .filter(({ primary }) => primary > 0)
        .sort(
            (first, second) =>
                Number(second.projectionIntersects) - Number(first.projectionIntersects) ||
                first.distanceSquared - second.distanceSquared ||
                first.primary - second.primary ||
                first.secondary - second.secondary ||
                first.index - second.index,
        )[0]?.id;
};

/** 在当前容器边缘继续搜索相邻容器中的空间方向目标。 */
export const nextDirectionalItemId = (
    currentContainerItems: InteractionItem[],
    allItems: InteractionItem[],
    currentId: string,
    direction: NavigationDirection,
): string | undefined => nearestDirectionalItemId(currentContainerItems, currentId, direction) ?? nearestDirectionalItemId(allItems, currentId, direction);

/** 按当前显示器视觉顺序返回背景、围栏标题和组件焦点目标。 */
export const workspaceFocusTargetsInVisualOrder = (surfaces: InteractionSurface[]): WorkspaceFocusTarget[] => [
    { type: "background" },
    ...surfaces
        .map((surface, index) => ({ ...surface, index }))
        .sort((first, second) => {
            // 两个高层焦点表面的可见边界重叠状态。
            const overlaps =
                first.rect.left < second.rect.left + second.rect.width &&
                first.rect.left + first.rect.width > second.rect.left &&
                first.rect.top < second.rect.top + second.rect.height &&
                first.rect.top + first.rect.height > second.rect.top;
            if (overlaps && first.zIndex !== second.zIndex) return second.zIndex - first.zIndex;
            return first.rect.top - second.rect.top || first.rect.left - second.rect.left || second.zIndex - first.zIndex || first.index - second.index;
        })
        .map(({ target }) => target),
];

/** 返回 Tab 顺序中的下一高层工作区焦点。 */
export const nextWorkspaceFocus = (targets: WorkspaceFocusTarget[], current: WorkspaceFocusTarget, reverse: boolean): WorkspaceFocusTarget | undefined => {
    if (targets.length === 0) return undefined;
    // 当前焦点在高层 Tab 顺序中的位置。
    const currentIndex = targets.findIndex(
        (target) =>
            target.type === current.type &&
            (target.type === "background" ||
                (target.type === "item" && current.type === "item" && target.itemId === current.itemId) ||
                (target.type === "fence" && current.type === "fence" && target.fenceId === current.fenceId) ||
                (target.type === "component" && current.type === "component" && target.componentId === current.componentId)),
    );
    if (currentIndex < 0) return targets[reverse ? targets.length - 1 : 0];
    return targets[(currentIndex + (reverse ? -1 : 1) + targets.length) % targets.length];
};
