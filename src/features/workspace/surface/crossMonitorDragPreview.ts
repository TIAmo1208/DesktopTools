import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { WorkspaceSnapshot } from "../../../contracts/workspace";
import type { CrossMonitorPreviewEvent } from "../interaction/crossMonitorDrag";
import { createDesktopItemPreview } from "../../desktop-items/desktopItemPreview";
import { createTileDragPreview } from "../../tiles/tileDragPreview";
import { moveFence, snapFenceEdges } from "../../fences/fenceGeometry";
import type { WorkspaceRect } from "../../../contracts/workspace";

/** 呈现跨屏拖动内容；目标几何与消息会话分别由调用方管理。 */
export const renderCrossMonitorDragPreview = (
    drag: CrossMonitorPreviewEvent,
    items: DesktopItemSnapshot | undefined,
    workspace: WorkspaceSnapshot | undefined,
    snapTargets: WorkspaceRect[] = [], // 目标适配器提供本帧真实几何，呈现与释放共用吸附算法。
) => {
    const nodes: HTMLElement[] = []; // 当前帧拖动内容节点。
    for (const visual of drag.visuals) {
        if (![visual.bounds?.left, visual.bounds?.top, visual.bounds?.width, visual.bounds?.height].every(Number.isFinite)) continue;
        const item = items?.items.find((item) => item.stableReference === visual.itemId); // 权威系统图像与名称。
        const fence = workspace?.fences.find((fence) => fence.members.includes(visual.itemId ?? "")); // 来源归属决定列表或平铺样式。
        const node =
            drag.kind === "fence"
                ? visual.content
                    ? createTileDragPreview(visual.content)
                    : document.createElement("div")
                : createDesktopItemPreview(
                      {
                          key: visual.itemId ?? "",
                          label: item?.displayName ?? visual.title ?? "",
                          imageSource: item?.systemImage ?? null,
                          isAvailable: item?.availability === "available",
                      },
                      fence ? `desktop-item--fence${fence.layoutMode === "list" ? " desktop-item--list" : ""}` : "desktop-item--unclassified",
                      workspace?.interaction.selectedItemIds.includes(visual.itemId ?? "") ?? false,
                  ); // 与本地预览共用项目内容，不硬编码图像尺寸。
        if (drag.kind === "fence" && !visual.content) {
            node.className = "desktop-tile fence fence--cross-preview";
            const label = document.createElement("span"); // 围栏预览保留原有标题呈现。
            label.textContent = visual.title ?? "";
            node.append(label);
        }
        Object.assign(node.style, {
            position: "fixed",
            pointerEvents: "none",
            zIndex: "20000",
            opacity: drag.kind === "fence" ? "0.75" : "",
            left: `${drag.pointer.left + visual.bounds.left - drag.origin.left}px`,
            top: `${drag.pointer.top + visual.bounds.top - drag.origin.top}px`,
            width: `${visual.bounds.width}px`,
            height: `${visual.bounds.height}px`,
        });
        if (drag.kind === "fence") {
            const viewport = { width: window.innerWidth, height: window.innerHeight }; // 目标屏工作区。
            const rect = moveFence(
                {
                    ...visual.bounds,
                    left: drag.pointer.left + visual.bounds.left - drag.origin.left,
                    top: drag.pointer.top + visual.bounds.top - drag.origin.top,
                },
                { x: 0, y: 0 },
                viewport,
            ); // 先约束整个格子到目标屏。
            const snapped = drag.snap ? snapFenceEdges(rect, snapTargets, viewport, 8, undefined, drag.minimum) : rect; // 与释放时相同的吸附与下限。
            Object.assign(node.style, { left: `${snapped.left}px`, top: `${snapped.top}px`, width: `${snapped.width}px`, height: `${snapped.height}px` });
        }
        (document.querySelector(".fence-workspace") ?? document.body).append(node);
        nodes.push(node);
    }

    return () => nodes.forEach((node) => node.remove());
};
