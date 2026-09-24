import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { WorkspaceSnapshot, WorkspaceRect } from "../../../contracts/workspace";
import type { CrossMonitorPreviewEvent } from "../interaction/crossMonitorDrag";
import { createDesktopItemPreview } from "../../desktop-items/desktopItemPreview";
import { createTileDragPreview } from "../../tiles/tileDragPreview";
import { moveFence, snapFenceEdges } from "../../fences/fenceGeometry";

interface PreviewNode {
    node: HTMLElement;
    visual: CrossMonitorPreviewEvent["visuals"][number];
}

/** 管理一个跨屏拖动会话的预览 DOM，连续消息只更新几何。 */
export interface CrossMonitorDragPreview {
    update: (drag: CrossMonitorPreviewEvent, items: DesktopItemSnapshot | undefined, workspace: WorkspaceSnapshot | undefined, snapTargets?: WorkspaceRect[]) => void;
    clear: () => void;
}

const visualKey = (drag: CrossMonitorPreviewEvent) =>
    `${drag.kind}:${drag.visuals.map((visual) => `${visual.itemId ?? ""}:${visual.title ?? ""}:${visual.bounds.left},${visual.bounds.top},${visual.bounds.width},${visual.bounds.height}`).join("|")}`;

/** 创建会话级跨屏预览控制器。 */
export const createCrossMonitorDragPreview = (): CrossMonitorDragPreview => {
    let nodes: PreviewNode[] = [];
    let key = "";
    let currentKind: CrossMonitorPreviewEvent["kind"] | undefined;

    const clear = () => {
        nodes.forEach(({ node }) => node.remove());
        nodes = [];
        key = "";
        currentKind = undefined;
    };

    const createNode = (drag: CrossMonitorPreviewEvent, visual: CrossMonitorPreviewEvent["visuals"][number], items: DesktopItemSnapshot | undefined, workspace: WorkspaceSnapshot | undefined) => {
        const item = items?.items.find((entry) => entry.stableReference === visual.itemId);
        const fence = workspace?.fences.find((entry) => entry.members.includes(visual.itemId ?? ""));
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
                  );
        if (drag.kind === "fence" && !visual.content) {
            node.className = "desktop-tile fence fence--cross-preview";
            const label = document.createElement("span");
            label.textContent = visual.title ?? "";
            node.append(label);
        }
        Object.assign(node.style, { position: "fixed", pointerEvents: "none", zIndex: "20000", opacity: drag.kind === "fence" ? "0.75" : "" });
        (document.querySelector(".fence-workspace") ?? document.body).append(node);
        return node;
    };

    const update = (drag: CrossMonitorPreviewEvent, items: DesktopItemSnapshot | undefined, workspace: WorkspaceSnapshot | undefined, snapTargets: WorkspaceRect[] = []) => {
        if (drag.source === "ole") return;
        const nextKey = visualKey(drag);
        if (nextKey !== key || currentKind !== drag.kind || nodes.length !== drag.visuals.length) {
            clear();
            nodes = drag.visuals.flatMap((visual) => {
                if (![visual.bounds?.left, visual.bounds?.top, visual.bounds?.width, visual.bounds?.height].every(Number.isFinite)) return [];
                return [{ node: createNode(drag, visual, items, workspace), visual }];
            });
            key = nextKey;
            currentKind = drag.kind;
        }
        nodes.forEach(({ node, visual }) => {
            let rect = { ...visual.bounds, left: drag.pointer.left + visual.bounds.left - drag.origin.left, top: drag.pointer.top + visual.bounds.top - drag.origin.top };
            if (drag.kind === "fence") {
                const viewport = { width: window.innerWidth, height: window.innerHeight };
                rect = moveFence(rect, { x: 0, y: 0 }, viewport);
                if (drag.snap) rect = snapFenceEdges(rect, snapTargets, viewport, 8, undefined, drag.minimum);
            }
            Object.assign(node.style, { left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px`, height: `${rect.height}px` });
        });
    };

    return { update, clear };
};
