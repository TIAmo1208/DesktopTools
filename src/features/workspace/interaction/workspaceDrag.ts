import type { FenceLayoutMode, WorkspaceGrid, WorkspacePoint, WorkspaceSnapshot } from "../../../contracts/workspace";
import type { DesktopHostMonitor } from "../../../contracts/desktopHost";
import type { InteractionRect } from "./workspaceInteraction";

/** 网格插入预览的方向。 */
export type GridInsertionDirection = "left" | "right" | "up" | "down";

/** 网格插入预览的计算结果。 */
export interface GridInsertionPreview {
    active: boolean;
    direction?: GridInsertionDirection;
    positions?: Map<string, WorkspacePoint>;
}

/** 围栏插入预览的计算结果。 */
export interface FenceInsertionPreview {
    active: boolean;
    direction?: GridInsertionDirection;
    placement?: "before" | "after";
}

/** 将任意持久位置稳定分配到互不重复的网格槽位。 */
const assignGridSlots = (positions: Map<string, WorkspacePoint>, excludedIds: Set<string>, grid: WorkspaceGrid): Map<string, number> => {
    const capacity = grid.rows * grid.columns;
    const used = new Set<number>();
    const result = new Map<string, number>();
    for (const [itemId, position] of positions) {
        if (excludedIds.has(itemId)) continue;
        let bestIndex: number | undefined;
        let bestDistance = Number.POSITIVE_INFINITY;
        for (let index = 0; index < capacity; index += 1) {
            if (used.has(index)) continue;
            const candidate = { left: grid.left + Math.floor(index / grid.rows) * grid.columnWidth, top: grid.top + (index % grid.rows) * grid.rowHeight };
            const distance = (candidate.left - position.left) ** 2 + (candidate.top - position.top) ** 2;
            if (distance < bestDistance) {
                bestIndex = index;
                bestDistance = distance;
            }
        }
        if (bestIndex !== undefined) {
            used.add(bestIndex);
            result.set(itemId, bestIndex);
        }
    }
    return result;
};

/** 将逐屏客户区 DIP 指针转换为虚拟桌面物理坐标。 */
export const clientPointToPhysical = (point: { x: number; y: number }, monitor: DesktopHostMonitor): { x: number; y: number } => ({
    x: monitor.workX + point.x * monitor.scaleFactor,
    y: monitor.workY + point.y * monitor.scaleFactor,
});

/** 返回包含虚拟桌面物理坐标的目标显示器。 */
export const monitorAtPhysicalPoint = (point: { x: number; y: number }, monitors: DesktopHostMonitor[]): DesktopHostMonitor | undefined =>
    monitors.find((monitor) => point.x >= monitor.x && point.x < monitor.x + monitor.width && point.y >= monitor.y && point.y < monitor.y + monitor.height);

/** 将虚拟桌面物理坐标转换为目标显示器工作区 DIP。 */
export const physicalPointToMonitorDip = (point: { x: number; y: number }, monitor: DesktopHostMonitor): WorkspacePoint => ({
    left: (point.x - monitor.workX) / monitor.scaleFactor,
    top: (point.y - monitor.workY) / monitor.scaleFactor,
});

/** 按显示器几何、容器几何和容器内部顺序排列跨屏拖动项目。 */
export const itemIdsInWorkspaceVisualOrder = (itemIds: Iterable<string>, workspace: WorkspaceSnapshot, monitors: DesktopHostMonitor[]): string[] => {
    const monitorOrder = new Map(
        [...monitors]
            .sort((first, second) => first.y - second.y || first.x - second.x || first.id.localeCompare(second.id))
            .map((monitor, index) => [monitor.id, index]),
    ); // 当前虚拟桌面屏幕几何对应的稳定显示器顺序。
    return [...itemIds]
        .map((itemId, originalIndex) => {
            const fence = workspace.fences.find((candidate) => candidate.members.includes(itemId)); // 当前项目所属围栏。
            const item = workspace.unclassifiedItems.find((candidate) => candidate.stableReference === itemId); // 当前项目保留的未归类位置。
            const monitorId = fence ? (fence.projectionMonitorId ?? fence.monitorId) : (item?.projectionMonitorId ?? item?.monitorId ?? ""); // 当前项目实际可见的显示器身份。
            const bounds = fence?.projectionBounds ?? fence?.bounds; // 当前围栏持久或临时投射边界。
            const position = item?.projectionPosition ?? item?.position ?? { left: 0, top: 0 }; // 当前未归类项目持久或投射位置。
            return {
                itemId,
                originalIndex,
                monitor: monitorOrder.get(monitorId) ?? Number.MAX_SAFE_INTEGER,
                top: bounds?.top ?? position.top,
                left: bounds?.left ?? position.left,
                member: fence?.members.indexOf(itemId) ?? 0,
            };
        })
        .sort(
            (first, second) =>
                first.monitor - second.monitor ||
                first.top - second.top ||
                first.left - second.left ||
                first.member - second.member ||
                first.originalIndex - second.originalIndex,
        )
        .map(({ itemId }) => itemId);
};

/** 计算单项或组拖动进入已占网格时的稳定传播预览。 */
export const resolveGridInsertion = (
    draggedIds: string[],
    targetId: string,
    pointer: { x: number; y: number },
    targetBounds: InteractionRect,
    positions: Map<string, WorkspacePoint>,
    grid: WorkspaceGrid,
): GridInsertionPreview | null => {
    const dx = pointer.x - (targetBounds.left + targetBounds.width / 2); // 指针相对目标中心的横向距离。
    const dy = pointer.y - (targetBounds.top + targetBounds.height / 2); // 指针相对目标中心的纵向距离。
    // 当前主导轴对应的插入方向。
    const direction: GridInsertionDirection = Math.abs(dx) >= Math.abs(dy) ? (dx < 0 ? "left" : "right") : dy < 0 ? "up" : "down";
    const horizontal = direction === "left" || direction === "right"; // 当前插入方向使用横轴传播。
    const activationDistance = horizontal ? Math.abs(dx) : Math.abs(dy); // 当前主导轴的中心距离。
    const activationThreshold = (horizontal ? targetBounds.width : targetBounds.height) * 0.25; // 目标中心死区占主轴尺寸的四分之一。
    if (activationDistance <= activationThreshold) return { active: false };

    const verticalPropagation = direction === "up" || direction === "down"; // 上下插入沿列优先顺序传播。
    const capacity = grid.rows * grid.columns; // 当前未归类网格容量。
    // 将当前传播轴的线性索引转换为网格位置。
    const indexToPosition = (index: number): WorkspacePoint => ({
        left: grid.left + (verticalPropagation ? Math.floor(index / grid.rows) : index % grid.columns) * grid.columnWidth,
        top: grid.top + (verticalPropagation ? index % grid.rows : Math.floor(index / grid.columns)) * grid.rowHeight,
    });
    if (!positions.has(targetId) || draggedIds.length === 0 || draggedIds.some((itemId) => !positions.has(itemId))) return null;

    const assignedTargetSlots = assignGridSlots(positions, new Set(draggedIds), grid);
    const targetIndex = assignedTargetSlots.get(targetId); // 当前目标项目的稳定线性索引。
    if (targetIndex === undefined) return null;
    const insertionIndex = direction === "left" || direction === "up" ? targetIndex : targetIndex + 1; // 当前方向对应的组插入索引。
    if (insertionIndex >= capacity) return null;
    const draggedSet = new Set(draggedIds); // 当前组拖动项目的快速查询表。
    const slots = Array<string | undefined>(capacity); // 移除拖动集合后的网格占用槽位。
    for (const [itemId, index] of assignGridSlots(positions, draggedSet, grid)) slots[index] = itemId;
    const sourceSlots = assignGridSlots(positions, new Set(), grid);
    const gapStart = direction === "left" || direction === "up" ? insertionIndex - draggedIds.length : insertionIndex;
    if (
        gapStart >= 0 &&
        gapStart + draggedIds.length <= capacity &&
        draggedIds.every((_, index) => !slots[gapStart + index]) &&
        draggedIds.every(
            (itemId) =>
                sourceSlots.get(itemId) === undefined || sourceSlots.get(itemId)! < gapStart || sourceSlots.get(itemId)! >= gapStart + draggedIds.length,
        )
    ) {
        const previewPositions = new Map(positions);
        draggedIds.forEach((itemId, index) => {
            slots[gapStart + index] = itemId;
            previewPositions.set(itemId, indexToPosition(gapStart + index));
        });
        return { active: true, direction, positions: previewPositions };
    }
    const pending = [...draggedIds]; // 等待按稳定顺序插入和向后传播的项目。
    for (let index = insertionIndex; index < capacity && pending.length > 0; index += 1) {
        const displaced = slots[index]; // 当前槽位需要继续向后传播的项目。
        slots[index] = pending.shift();
        if (displaced) pending.push(displaced);
    }
    if (pending.length > 0) return null;

    const previewPositions = new Map(positions); // 当前组插入后的完整网格位置。
    slots.forEach((itemId, index) => {
        if (itemId) previewPositions.set(itemId, indexToPosition(index));
    });
    return { active: true, direction, positions: previewPositions };
};

/** 自由定位保留组内偏移，并将整组限制在工作区左上边界内。 */
export const resolveFreeDrop = (draggedIds: string[], desired: WorkspacePoint, positions: Map<string, WorkspacePoint>): Map<string, WorkspacePoint> | null => {
    if (!draggedIds.length || !Number.isFinite(desired.left) || !Number.isFinite(desired.top)) return null;
    const anchor = positions.get(draggedIds[0]); // 来源组的视觉锚点。
    if (!anchor || draggedIds.some((id) => !positions.has(id))) return null;
    const offsets = draggedIds.map((id) => ({ id, left: positions.get(id)!.left - anchor.left, top: positions.get(id)!.top - anchor.top })); // 保留组内相对位置。
    const left = Math.max(desired.left, -Math.min(0, ...offsets.map((offset) => offset.left))); // 整组留在工作区左边界内。
    const top = Math.max(desired.top, -Math.min(0, ...offsets.map((offset) => offset.top))); // 整组留在工作区上边界内。
    return new Map(offsets.map((offset) => [offset.id, { left: left + offset.left, top: top + offset.top }]));
};

/** 将组拖动释放点吸附到最近可用网格位置。 */
export const resolveGridDrop = (
    draggedIds: string[],
    desiredPosition: WorkspacePoint,
    positions: Map<string, WorkspacePoint>,
    grid: WorkspaceGrid,
    renderedPositions?: Map<string, WorkspacePoint>, // 来源当前可见排列，围栏项目不能使用旧桌面占位。
): Map<string, WorkspacePoint> | null => {
    if (draggedIds.length === 0) return null;
    const capacity = grid.rows * grid.columns; // 当前未归类网格容量。
    const draggedSet = new Set(draggedIds); // 当前组拖动项目的快速查询表。
    const occupied = new Set(assignGridSlots(positions, draggedSet, grid).values()); // 排除拖动集合后的已占网格索引。
    // 按释放点距离排列的候选锚点索引。
    const candidates = Array.from({ length: capacity }, (_, index) => index).sort((first, second) => {
        const firstPosition = { left: grid.left + Math.floor(first / grid.rows) * grid.columnWidth, top: grid.top + (first % grid.rows) * grid.rowHeight }; // 首个候选位置。
        const secondPosition = { left: grid.left + Math.floor(second / grid.rows) * grid.columnWidth, top: grid.top + (second % grid.rows) * grid.rowHeight }; // 第二个候选位置。
        const firstDistance = (firstPosition.left - desiredPosition.left) ** 2 + (firstPosition.top - desiredPosition.top) ** 2; // 首个候选平方距离。
        const secondDistance = (secondPosition.left - desiredPosition.left) ** 2 + (secondPosition.top - desiredPosition.top) ** 2; // 第二个候选平方距离。
        return firstDistance - secondDistance || first - second;
    });
    const anchorPosition = positions.get(draggedIds[0]); // 当前拖动锚点的来源网格位置。
    if (anchorPosition) {
        const assignedSlots = assignGridSlots(positions, new Set(), grid);
        const anchorIndex = assignedSlots.get(draggedIds[0]);
        const anchorColumn = anchorIndex === undefined ? Math.round((anchorPosition.left - grid.left) / grid.columnWidth) : Math.floor(anchorIndex / grid.rows); // 来源锚点列索引。
        const anchorRow = anchorIndex === undefined ? Math.round((anchorPosition.top - grid.top) / grid.rowHeight) : anchorIndex % grid.rows; // 来源锚点行索引。
        const offsets = draggedIds.map((itemId) => {
            const renderedAnchor = renderedPositions?.get(draggedIds[0]); // 当前拖动锚点的真实位置。
            const rendered = renderedPositions?.get(itemId); // 当前项目的真实位置。
            if (renderedAnchor && rendered)
                return {
                    itemId,
                    column: Math.round((rendered.left - renderedAnchor.left) / grid.columnWidth),
                    row: Math.round((rendered.top - renderedAnchor.top) / grid.rowHeight),
                };
            const position = positions.get(itemId); // 当前组成员的来源网格位置。
            const assignedIndex = assignedSlots.get(itemId);
            return position
                ? {
                      itemId,
                      column:
                          (assignedIndex === undefined ? Math.round((position.left - grid.left) / grid.columnWidth) : Math.floor(assignedIndex / grid.rows)) -
                          anchorColumn,
                      row: (assignedIndex === undefined ? Math.round((position.top - grid.top) / grid.rowHeight) : assignedIndex % grid.rows) - anchorRow,
                  }
                : undefined;
        }); // 当前组成员相对拖动锚点的 DIP 网格几何。
        if (offsets.every((offset) => offset !== undefined)) {
            for (const candidate of candidates) {
                const candidateColumn = Math.floor(candidate / grid.rows); // 当前候选锚点列索引。
                const candidateRow = candidate % grid.rows; // 当前候选锚点行索引。
                const slots = offsets.map((offset) => ({
                    itemId: offset.itemId,
                    column: candidateColumn + offset.column,
                    row: candidateRow + offset.row,
                })); // 保持组内 DIP 几何后的目标槽位。
                const slotIndexes = slots.map(({ column, row }) => column * grid.rows + row); // 当前组目标槽位的列优先索引。
                if (
                    slots.some(({ column, row }) => column < 0 || column >= grid.columns || row < 0 || row >= grid.rows) ||
                    new Set(slotIndexes).size !== slotIndexes.length ||
                    slotIndexes.some((index) => occupied.has(index))
                )
                    continue;
                const result = new Map(positions); // 保持完整组内几何的最终网格位置。
                slots.forEach(({ itemId, column, row }) => {
                    result.set(itemId, {
                        left: grid.left + column * grid.columnWidth,
                        top: grid.top + row * grid.rowHeight,
                    });
                });
                return result;
            }
        }
    }
    for (const candidate of candidates) {
        const available: number[] = []; // 当前候选锚点之后的稳定空槽集合。
        for (let offset = 0; offset < capacity && available.length < draggedIds.length; offset += 1) {
            const index = (candidate + offset) % capacity; // 当前循环扫描的网格索引。
            if (!occupied.has(index)) available.push(index);
        }
        if (available.length < draggedIds.length) continue;
        const result = new Map(positions); // 当前组释放后的完整网格位置。
        draggedIds.forEach((itemId, index) => {
            const slot = available[index]; // 当前拖动项目获得的稳定网格索引。
            result.set(itemId, {
                left: grid.left + Math.floor(slot / grid.rows) * grid.columnWidth,
                top: grid.top + (slot % grid.rows) * grid.rowHeight,
            });
        });
        return result;
    }
    return null;
};

/** 按围栏布局轴判断当前指针对应的手动插入方向。 */
export const resolveFenceInsertion = (pointer: { x: number; y: number }, targetBounds: InteractionRect, layoutMode: FenceLayoutMode): FenceInsertionPreview => {
    const vertical = layoutMode === "list"; // 列表使用纵轴，平铺使用横轴。
    const distance = vertical ? pointer.y - (targetBounds.top + targetBounds.height / 2) : pointer.x - (targetBounds.left + targetBounds.width / 2); // 指针到目标中心在布局轴上的距离。
    const threshold = (vertical ? targetBounds.height : targetBounds.width) * 0.25; // 目标中心死区占布局轴尺寸的四分之一。
    if (Math.abs(distance) <= threshold) return { active: false };
    return {
        active: true,
        direction: vertical ? (distance < 0 ? "up" : "down") : distance < 0 ? "left" : "right",
        placement: distance < 0 ? "before" : "after",
    };
};

/** 计算围栏边缘自动滚动的每帧速度。 */
export const resolveFenceAutoScroll = (pointerY: number, viewportBounds: Pick<DOMRect, "top" | "bottom">): number => {
    if (pointerY < viewportBounds.top || pointerY > viewportBounds.bottom) return 0;
    const center = (viewportBounds.top + viewportBounds.bottom) / 2; // 围栏内容区垂直中心。
    const halfHeight = (viewportBounds.bottom - viewportBounds.top) / 2; // 围栏内容区半高。
    const deadZoneHalfHeight = halfHeight * 0.7; // 中央死区占内容区半高的 70%。
    const activeDistance = halfHeight - deadZoneHalfHeight; // 单侧有效滚动距离。
    const distance = pointerY - center; // 指针离开内容中心的有符号距离。
    const outsideDistance = Math.max(0, Math.abs(distance) - deadZoneHalfHeight); // 指针越过死区后的距离。
    if (outsideDistance === 0 || activeDistance <= 0) return 0;
    return Math.sign(distance) * 12 * Math.min(1, outsideDistance / activeDistance);
};

/** 将围栏插入方向转换为移除拖动集合后的后继成员。 */
export const beforeItemIdForFenceDrop = (
    members: string[],
    draggedIds: string[],
    targetId: string | undefined,
    placement: "before" | "after" | undefined,
): string | null => {
    const draggedSet = new Set(draggedIds); // 当前组拖动项目的快速查询表。
    const remaining = members.filter((itemId) => !draggedSet.has(itemId)); // 移除拖动集合后的目标围栏顺序。
    if (!targetId || !placement) return null;
    const targetIndex = remaining.indexOf(targetId); // 当前命中项目在剩余成员中的索引。
    if (targetIndex < 0) return null;
    return placement === "before" ? targetId : (remaining[targetIndex + 1] ?? null);
};
