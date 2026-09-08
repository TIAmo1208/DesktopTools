import type { WorkspaceSnapshot } from "../../../contracts/workspace.ts";

/** 将权威工作区投影到单个显示器，不改变全局选择和持久归属。 */
export const projectWorkspaceSurface = (snapshot: WorkspaceSnapshot | undefined, monitorId: string | undefined): WorkspaceSnapshot | undefined => {
    if (!snapshot || !monitorId) return snapshot;
    const fences = snapshot.fences
        .filter((fence) => (fence.projectionMonitorId ?? fence.monitorId) === monitorId)
        .map((fence) => ({ ...fence, bounds: fence.projectionMonitorId === monitorId && fence.projectionBounds ? fence.projectionBounds : fence.bounds })); // 当前显示器可见的持久或临时投射围栏。
    const unclassifiedItems = snapshot.unclassifiedItems
        .filter((item) => (item.projectionMonitorId ?? item.monitorId) === monitorId)
        .map((item) => ({
            ...item,
            position: item.projectionMonitorId === monitorId && item.projectionPosition ? item.projectionPosition : item.position,
        })); // 当前显示器可见的持久或临时投射项目位置。
    const components = snapshot.components
        .filter((component) => (component.projectionMonitorId ?? component.monitorId) === monitorId)
        .map((component) => ({
            ...component,
            bounds: component.projectionMonitorId === monitorId && component.projectionBounds ? component.projectionBounds : component.bounds,
        })); // 当前显示器可见的持久或临时投射组件实例。
    const visibleItemIds = new Set([...unclassifiedItems.map((item) => item.stableReference), ...fences.flatMap((fence) => fence.members)]); // 当前页面可以呈现的完整项目集合。
    const focus = snapshot.interaction.keyboardFocus; // 当前全局工作区的键盘焦点。
    const interaction = {
        ...snapshot.interaction,
        selectedItemIds: snapshot.interaction.selectedItemIds.filter((itemId) => visibleItemIds.has(itemId)),
        selectionAnchorId:
            snapshot.interaction.selectionAnchorId && visibleItemIds.has(snapshot.interaction.selectionAnchorId)
                ? snapshot.interaction.selectionAnchorId
                : null,
        keyboardFocus:
            focus.type === "item" && !visibleItemIds.has(focus.itemId)
                ? ({ type: "background" } as const)
                : focus.type === "fence" && !fences.some((fence) => fence.id === focus.fenceId)
                  ? ({ type: "background" } as const)
                  : focus,
    }; // 当前逐屏页面隔离后的选择、锚点和焦点投影。
    return {
        ...snapshot,
        grid: snapshot.monitorGrids.find((entry) => entry.monitorId === monitorId)?.grid ?? snapshot.grid,
        fences,
        unclassifiedItems,
        components,
        interaction,
    };
};
