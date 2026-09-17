import type { WorkspaceSnapshot } from "../../../contracts/workspace.ts";
import type { HitRegion } from "../../../contracts/desktopHost.ts";

/** 收集当前实际可见的完整命中图，供窗口输入与 OLE 预览共同使用。 */
export const collectHitRegions = (): HitRegion[] =>
    projectHitRegions(
        [...document.querySelectorAll<HTMLElement>("[data-hit-id]")]
            .filter(
                (element) =>
                    !element.closest('[data-cross-drag-hidden="true"]') &&
                    element.getClientRects().length > 0 &&
                    getComputedStyle(element).visibility !== "hidden",
            )
            .map((element) => {
                const bounds = element.getBoundingClientRect(); // 当前语义对象 CSS 像素边界。
                const surface = element.closest<HTMLElement>(".desktop-tile, .desktop-tile-menu") ?? element; // 控件所属的真实公共表面。
                const clip = element.closest<HTMLElement>("[data-fence-content-id]")?.getBoundingClientRect(); // 不可见成员不能参与原生命中。
                return {
                    id: element.dataset.hitId ?? "",
                    kind: element.dataset.hitKind ?? "",
                    left: bounds.left,
                    top: bounds.top,
                    width: bounds.width,
                    height: bounds.height,
                    priority: Number(element.dataset.hitPriority ?? "0"),
                    targetPath: element.dataset.folderPortalPath,
                    stackingOrder: Number.parseInt(getComputedStyle(surface).zIndex, 10) || 0,
                    clip,
                };
            }),
        window.innerWidth,
        window.innerHeight,
    );

/** 将语义矩形限制在可见视口，并按实际表面层级生成原生命中顺序。 */
export const projectHitRegions = (
    regions: (HitRegion & { stackingOrder: number; clip?: Pick<HitRegion, "left" | "top" | "width" | "height"> })[],
    width: number,
    height: number,
): HitRegion[] =>
    [...regions]
        .sort((first, second) => first.stackingOrder - second.stackingOrder || first.priority - second.priority)
        .flatMap((region, index) => {
            const left = Math.max(0, region.left, region.clip?.left ?? 0); // 可见左边界。
            const top = Math.max(0, region.top, region.clip?.top ?? 0); // 可见上边界。
            const right = Math.min(width, region.left + region.width, region.clip ? region.clip.left + region.clip.width : width); // 可见右边界。
            const bottom = Math.min(height, region.top + region.height, region.clip ? region.clip.top + region.clip.height : height); // 可见下边界。
            return right > left && bottom > top
                ? [
                      {
                          id: region.id,
                          kind: region.kind,
                          left,
                          top,
                          width: right - left,
                          height: bottom - top,
                          priority: index + 1,
                          ...(region.targetPath ? { targetPath: region.targetPath } : {}),
                      },
                  ]
                : [];
        });

/** 将权威工作区投影到单个显示器，不改变全局选择和持久归属。 */
export const projectWorkspaceSurface = (snapshot: WorkspaceSnapshot | undefined, monitorId: string | undefined): WorkspaceSnapshot | undefined => {
    if (!snapshot || !monitorId) return undefined;
    const fences = snapshot.fences
        .filter((fence) => (fence.projectionMonitorId ?? fence.monitorId) === monitorId)
        .map((fence) => ({ ...fence, bounds: fence.projectionMonitorId === monitorId && fence.projectionBounds ? fence.projectionBounds : fence.bounds })); // 当前显示器可见的持久或临时投射围栏。
    const memberIds = new Set(snapshot.fences.flatMap((fence) => fence.members)); // 归属来自完整工作区，不能把其他屏幕围栏成员重新呈现为桌面项目。
    const unclassifiedItems = snapshot.unclassifiedItems
        .filter((item) => !memberIds.has(item.stableReference) && (item.projectionMonitorId ?? item.monitorId) === monitorId)
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
