import type { PointerEvent, RefObject } from "react";

import type { DesktopHostMonitor } from "../../../contracts/desktopHost";
import type { ShellDragPreparation } from "../../../contracts/desktopItem";
import { prepareDesktopItemDrag, cancelDesktopItemDragPreparation } from "../../../bridge/desktopItems";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspacePoint, WorkspaceSnapshot } from "../../../contracts/workspace";
import { listenVirtualDesktopSwitching } from "../../../bridge/virtualDesktop";
import { selectionModeForPointer, shouldOpenItemOnPointerRelease } from "./workspaceInteraction";
import { usePreferences } from "../../settings/PreferencesProvider";
import { publishCrossMonitorDrag, resolveCrossMonitorDrop, type CrossMonitorDrag } from "./crossMonitorDrag";
import {
    beforeItemIdForFenceDrop,
    clientPointToPhysical,
    itemIdsInWorkspaceVisualOrder,
    monitorAtPhysicalPoint,
    physicalPointToMonitorDip,
    resolveFenceAutoScroll,
    resolveFenceInsertion,
    resolveGridDrop,
    resolveFreeDrop,
    resolveGridInsertion,
} from "./workspaceDrag";

/** 工作区项目拖动需要的权威状态和命令边界。 */
interface WorkspaceDragOptions {
    workspaceRef: RefObject<HTMLElement | null>;
    workspace?: WorkspaceSnapshot;
    completeWorkspace?: WorkspaceSnapshot;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    selectedItemIds: Set<string>;
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
    onStartShellDrag: (token: number, pointerOffset: WorkspacePoint) => Promise<void>;
    onOpenItems: (itemIds: string[]) => Promise<void>; // 未形成拖动的单击打开动作。
    onFailure: (failure: string | undefined) => void;
}

/** 管理单项或组拖动的原型预览、取消恢复和原子提交。 */
export const useWorkspaceDrag = ({
    workspaceRef,
    workspace,
    completeWorkspace,
    currentMonitor,
    monitors,
    selectedItemIds,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
    onStartShellDrag,
    onOpenItems,
    onFailure,
}: WorkspaceDragOptions) => {
    const { preferences } = usePreferences(); // 当前窗口共用的单击或双击打开偏好。
    /** 冻结布局后建立连续项目拖动。 */
    const handleItemPointerDown = async (event: PointerEvent<HTMLLIElement>, anchorItemId: string) => {
        if (event.button !== 0 || !workspace) return;
        event.preventDefault();
        event.stopPropagation();
        onFailure(undefined);
        workspaceRef.current?.focus({ preventScroll: true });

        const handle = event.currentTarget; // 当前连续拖动持有指针捕获的锚点节点。
        const openingModifiers = { ctrlKey: event.ctrlKey, shiftKey: event.shiftKey, altKey: event.altKey, metaKey: event.metaKey }; // 冻结按下时的选择修饰键，避免释放后读取失效事件。
        const workspaceElement = workspaceRef.current; // 当前完整工作区交互表面。
        if (!workspaceElement) return;
        const allItemElements = Array.from(workspaceElement.querySelectorAll<HTMLElement>("[data-item-id]")); // 当前宿主中全部可见项目节点。
        const globalSelectedItemIds = new Set(completeWorkspace?.interaction.selectedItemIds ?? selectedItemIds); // 当前虚拟桌面跨显示器完整选择集合。
        const preservesSelection = globalSelectedItemIds.has(anchorItemId) && !event.ctrlKey && !event.shiftKey; // 当前普通按下是否沿用已有组选择。
        const pointerSelectionMode = preservesSelection ? "replace" : selectionModeForPointer(event); // 未形成拖动时提交的点击选择规则。
        const requestedIds = preservesSelection ? globalSelectedItemIds : new Set([anchorItemId]); // 当前拖动使用的跨屏选择集合。
        const dragItemIds = itemIdsInWorkspaceVisualOrder(requestedIds, completeWorkspace ?? workspace, monitors); // 当前组拖动按屏幕、容器和项目几何排列的稳定集合。
        const anchorIndex = dragItemIds.indexOf(anchorItemId); // 实际拖动项目在稳定集合中的位置。
        if (anchorIndex >= 0) dragItemIds.splice(anchorIndex, 1);
        dragItemIds.unshift(anchorItemId);

        const dragElements = new Map(
            allItemElements.flatMap((element) => {
                const itemId = element.dataset.itemId; // 当前 DOM 节点对应的稳定项目引用。
                return itemId && dragItemIds.includes(itemId) ? [[itemId, element] as const] : [];
            }),
        ); // 当前拖动集合对应的可见 DOM 节点。
        const startPositions = new Map((completeWorkspace ?? workspace).unclassifiedItems.map((item) => [item.stableReference, item.position])); // 拖动开始时冻结的跨屏全部来源位置。
        const fenceContents = new Map(
            Array.from(workspaceElement.querySelectorAll<HTMLElement>("[data-fence-content-id]")).flatMap((element) => {
                const fenceId = element.dataset.fenceContentId; // 当前滚动内容对应的围栏标识。
                return fenceId ? [[fenceId, element] as const] : [];
            }),
        ); // 当前宿主中的围栏滚动内容节点。
        const startScrolls = new Map(Array.from(fenceContents, ([fenceId, element]) => [fenceId, element.scrollTop])); // 拖动开始时冻结的围栏滚动位置。
        const anchorPosition = startPositions.get(anchorItemId); // 当前拖动锚点的来源网格位置。
        const startPointer = { x: event.clientX, y: event.clientY }; // 当前连续拖动开始时的指针位置。
        let crossDrag: CrossMonitorDrag | undefined; // 本次跨宿主短会话。
        const dragSession = crypto.randomUUID(); // 排除其他源窗口旧消息。
        const anchorBounds = dragElements.get(anchorItemId)?.getBoundingClientRect(); // 拖动锚点初始客户区边界。
        const pointerOffset = anchorBounds ? { x: event.clientX - anchorBounds.left, y: event.clientY - anchorBounds.top } : { x: 0, y: 0 }; // 指针在拖动锚点内部保持的 DIP 偏移。

        try {
            await onBeginInputDrag();
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        const previews = dragItemIds.flatMap((itemId) => {
            const element = dragElements.get(itemId); // 当前拖动项目对应的原始 DOM 节点。
            if (!element) return [];
            const bounds = element.getBoundingClientRect(); // 当前拖动项目的初始客户区边界。
            const preview = element.cloneNode(true) as HTMLElement; // 当前跟随指针且不参与命中的项目预览。
            preview.removeAttribute("data-hit-id");
            preview.removeAttribute("data-fence-member-id");
            preview.classList.add("desktop-item--drag-preview");
            const appearance = window.getComputedStyle(element); // 脱离围栏后仍需保留的真实文字与主题。
            for (const property of ["--fence-text", "--fence-item-text-shadow"]) preview.style.setProperty(property, appearance.getPropertyValue(property));
            Object.assign(preview.style, {
                visibility: "hidden",
                color: appearance.color,
                textShadow: appearance.textShadow,
                position: "fixed",
                left: bounds.left + "px",
                top: bounds.top + "px",
                width: bounds.width + "px",
                height: bounds.height + "px",
                zIndex: "20000",
            });
            document.body.append(preview);
            return [{ itemId, preview, bounds }];
        }); // 当前组拖动的全部视觉预览。
        const marker = document.createElement("div"); // 当前网格或围栏插入位置标记。
        marker.className = "fence-workspace__insertion-marker";
        marker.hidden = true;
        document.body.append(marker);

        let moved = false; // 当前指针是否已经越过点击抖动范围。
        let delta = { x: 0, y: 0 }; // 当前指针相对拖动起点的位移。
        let gridPreview: Map<string, WorkspacePoint> | undefined; // 当前有效的未归类网格预览位置。
        let insertionBlocked = false; // 当前网格传播是否因容量不足而失败。
        let targetFenceId: string | undefined; // 当前指针命中的目标围栏。
        let rejectedByTile = false; // 内容不支持文件时阻止穿透到下层围栏或桌面。
        let targetItemId: string | undefined; // 当前指针命中的目标围栏成员。
        let targetPlacement: "before" | "after" | undefined; // 当前围栏成员插入方向。
        let autoScrollFrame = 0; // 当前围栏自动滚动动画帧。
        let autoScrollSpeed = 0; // 当前围栏自动滚动每帧速度。
        let latestPointer: { x: number; y: number } | undefined; // 自动滚动持续更新预览使用的最近指针。
        let latestClientPointer = startPointer; // 跨显示器落位使用的最近逐屏客户区指针。
        let freePlacement = event.altKey; // 内部回退也遵循释放时 Alt 自由定位。
        let finished = false; // 当前连续拖动结束标记。
        let prepared: ShellDragPreparation | null = null; // 完整数据对象成功准备后才允许交接 OLE。
        let switchingSubscription: Promise<() => void> | undefined; // 当前拖动期间监听虚拟桌面切换的取消订阅。

        /** 将未归类项目 DOM 恢复到拖动开始时的位置。 */
        const restoreGridPreview = () => {
            for (const element of allItemElements) {
                if (!element.classList.contains("desktop-item--unclassified")) continue;
                const position = startPositions.get(element.dataset.itemId ?? ""); // 当前项目拖动开始时的来源位置。
                if (!position) continue;
                element.style.left = position.left + "px";
                element.style.top = position.top + "px";
            }
            gridPreview = undefined;
            insertionBlocked = false;
        };

        /** 将当前完整网格预览直接投射到未归类项目 DOM。 */
        const applyGridPreview = (positions: Map<string, WorkspacePoint>) => {
            for (const element of allItemElements) {
                if (!element.classList.contains("desktop-item--unclassified")) continue;
                const position = positions.get(element.dataset.itemId ?? ""); // 当前项目在拖动预览中的网格位置。
                if (!position) continue;
                element.style.left = position.left + "px";
                element.style.top = position.top + "px";
            }
        };

        /** 停止当前围栏边缘自动滚动。 */
        const stopAutoScroll = () => {
            window.cancelAnimationFrame(autoScrollFrame);
            autoScrollFrame = 0;
            autoScrollSpeed = 0;
            latestPointer = undefined;
        };

        /** 清理当前围栏投放和插入标记状态。 */
        const clearFencePreview = () => {
            workspaceElement.querySelectorAll(".fence--drop-target").forEach((element) => element.classList.remove("fence--drop-target"));
            targetFenceId = undefined;
            targetItemId = undefined;
            targetPlacement = undefined;
        };

        /** 根据当前指针更新围栏投放、手动顺序和自动滚动预览。 */
        const previewFenceDrop = (pointer: { x: number; y: number }): boolean => {
            clearFencePreview();
            const tile = document
                .elementsFromPoint(pointer.x, pointer.y)
                .map((element) => element.closest<HTMLElement>(".desktop-tile"))
                .find(Boolean); // 只判断最上层公共表面。
            rejectedByTile = Boolean(tile && !tile.dataset.fenceId);
            if (rejectedByTile) {
                stopAutoScroll();
                marker.hidden = true;
                return true;
            }
            const fenceElement = document
                .elementsFromPoint(pointer.x, pointer.y)
                .map((element) => element.closest<HTMLElement>("[data-fence-id]"))
                .find((element): element is HTMLElement => Boolean(element)); // 当前指针命中的最高围栏表面。
            if (!fenceElement) {
                stopAutoScroll();
                return false;
            }
            const fenceId = fenceElement.dataset.fenceId; // 当前目标围栏稳定标识。
            const fence = workspace.fences.find((candidate) => candidate.id === fenceId); // 当前目标围栏权威快照。
            const content = fenceId ? fenceContents.get(fenceId) : undefined; // 当前目标围栏内容滚动节点。
            if (!fenceId || !fence || !content || content.getClientRects().length === 0) {
                stopAutoScroll();
                return false;
            }

            targetFenceId = fenceId;
            fenceElement.classList.add("fence--drop-target");
            content.dataset.dragScrolling = "true";
            const viewportBounds = content.getBoundingClientRect(); // 当前围栏内容区的可见边界。
            autoScrollSpeed = resolveFenceAutoScroll(pointer.y, viewportBounds);
            latestPointer = pointer;
            if (autoScrollSpeed === 0) {
                window.cancelAnimationFrame(autoScrollFrame);
                autoScrollFrame = 0;
            } else if (autoScrollFrame === 0) {
                autoScrollFrame = window.requestAnimationFrame(function scrollFencePreview() {
                    autoScrollFrame = 0;
                    const previousScrollTop = content.scrollTop; // 本帧滚动前的内容位置。
                    content.scrollTop += autoScrollSpeed;
                    if (content.scrollTop === previousScrollTop || !latestPointer) {
                        stopAutoScroll();
                        return;
                    }
                    previewFenceDrop(latestPointer);
                });
            }

            if (fence.sortMode && fence.sortMode !== "none") {
                marker.hidden = true;
                return true;
            }

            const target = document
                .elementsFromPoint(pointer.x, pointer.y)
                .map((element) => element.closest<HTMLElement>("[data-fence-member-id]"))
                .find((element) => element?.dataset.fenceMemberId && !dragItemIds.includes(element.dataset.fenceMemberId)); // 当前指针命中的非拖动围栏成员。
            const targetId = target?.dataset.fenceMemberId; // 当前围栏插入目标成员标识。
            if (!target || !targetId) {
                marker.hidden = true;
                return true;
            }
            const targetBounds = target.getBoundingClientRect(); // 当前围栏插入目标边界。
            const insertion = resolveFenceInsertion(pointer, targetBounds, fence.layoutMode); // 当前围栏布局轴对应的插入结果。
            if (!insertion.active || !insertion.direction || !insertion.placement) {
                marker.hidden = true;
                return true;
            }
            targetItemId = targetId;
            targetPlacement = insertion.placement;
            const left = Math.max(targetBounds.left, viewportBounds.left); // 插入目标在内容视口中的可见左边界。
            const top = Math.max(targetBounds.top, viewportBounds.top); // 插入目标在内容视口中的可见上边界。
            const right = Math.min(targetBounds.right, viewportBounds.right); // 插入目标在内容视口中的可见右边界。
            const bottom = Math.min(targetBounds.bottom, viewportBounds.bottom); // 插入目标在内容视口中的可见下边界。
            const verticalMarker = fence.layoutMode === "tile"; // 平铺使用垂直标记，列表使用水平标记。
            marker.style.left = (insertion.direction === "right" ? right - 3 : left) + "px";
            marker.style.top = (insertion.direction === "down" ? bottom - 3 : top) + "px";
            marker.style.width = (verticalMarker ? 3 : Math.max(0, right - left)) + "px";
            marker.style.height = (verticalMarker ? Math.max(0, bottom - top) : 3) + "px";
            marker.hidden = right <= left || bottom <= top;
            return true;
        };

        /** 根据当前指针更新未归类网格的四向稳定传播预览。 */
        const previewGridDrop = (pointer: { x: number; y: number }) => {
            marker.hidden = true;
            restoreGridPreview();
            const target = allItemElements.find((element) => {
                const itemId = element.dataset.itemId; // 当前候选网格项目稳定引用。
                if (!itemId || dragItemIds.includes(itemId) || !element.classList.contains("desktop-item--unclassified")) return false;
                const bounds = element.getBoundingClientRect(); // 当前候选网格项目客户区边界。
                return pointer.x >= bounds.left && pointer.x <= bounds.right && pointer.y >= bounds.top && pointer.y <= bounds.bottom;
            }); // 当前指针命中的已占网格项目。
            const targetId = target?.dataset.itemId; // 当前网格插入目标项目标识。
            if (!target || !targetId) return;
            const targetBounds = target.getBoundingClientRect(); // 当前网格插入目标边界。
            const insertion = resolveGridInsertion(dragItemIds, targetId, pointer, targetBounds, startPositions, workspace.grid); // 当前四向网格插入结果。
            if (!insertion) {
                insertionBlocked = true;
                return;
            }
            if (!insertion.active || !insertion.direction || !insertion.positions) return;
            gridPreview = insertion.positions;
            applyGridPreview(gridPreview);
            const verticalMarker = insertion.direction === "left" || insertion.direction === "right"; // 左右插入使用垂直标记。
            marker.style.left = (insertion.direction === "right" ? targetBounds.right - 3 : targetBounds.left) + "px";
            marker.style.top = (insertion.direction === "down" ? targetBounds.bottom - 3 : targetBounds.top) + "px";
            marker.style.width = (verticalMarker ? 3 : targetBounds.width) + "px";
            marker.style.height = (verticalMarker ? targetBounds.height : 3) + "px";
            marker.hidden = false;
        };

        /** 指针移动时更新组预览、围栏投放和网格传播。 */
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            if (finished) return;
            freePlacement = moveEvent.altKey;
            latestClientPointer = { x: moveEvent.clientX, y: moveEvent.clientY };
            delta = { x: moveEvent.clientX - startPointer.x, y: moveEvent.clientY - startPointer.y };
            moved ||= Math.abs(delta.x) > 3 || Math.abs(delta.y) > 3;
            if (prepared && (Math.abs(delta.x) >= prepared.threshold.left || Math.abs(delta.y) >= prepared.threshold.top)) {
                void finishDrag("shell-drag", event.pointerId);
                return;
            }
            previews.forEach(({ itemId, preview, bounds }) => {
                if (moved) {
                    preview.style.visibility = "visible";
                    dragElements.get(itemId)?.classList.add("desktop-item--dragging");
                }
                preview.style.left = bounds.left + delta.x + "px";
                preview.style.top = bounds.top + delta.y + "px";
            });
            if (!moved) return;
            if (currentMonitor) {
                const physical = clientPointToPhysical(latestClientPointer, currentMonitor); // 指针统一到物理屏幕空间。
                const target = monitorAtPhysicalPoint(physical, monitors); // 当前目标屏。
                if (target) {
                    crossDrag = {
                        session: dragSession,
                        source: currentMonitor.id,
                        target: target.id,
                        pointer: physicalPointToMonitorDip(physical, target),
                        origin: { left: startPointer.x, top: startPointer.y },
                        delta: { left: delta.x, top: delta.y },
                        kind: "items",
                        itemIds: dragItemIds,
                        visuals: previews.map(({ itemId, bounds }) => ({
                            itemId,
                            bounds: { left: bounds.left, top: bounds.top, width: bounds.width, height: bounds.height },
                        })),
                    };
                    void publishCrossMonitorDrag(crossDrag, "move").catch(() => undefined);
                }
            }
            restoreGridPreview();
            if (previewFenceDrop({ x: moveEvent.clientX, y: moveEvent.clientY })) return;
            if (!freePlacement) previewGridDrop({ x: moveEvent.clientX, y: moveEvent.clientY });
        };

        /** 恢复拖动开始时的网格和围栏滚动状态。 */
        const restoreStartLayout = () => {
            restoreGridPreview();
            for (const [fenceId, scrollTop] of startScrolls) {
                const content = fenceContents.get(fenceId); // 当前待恢复滚动位置的围栏内容节点。
                if (content) content.scrollTop = scrollTop;
            }
        };

        /** 清理连续拖动产生的临时 DOM 和宿主状态。 */
        const cleanupDrag = async (endInput = true) => {
            if (crossDrag) void publishCrossMonitorDrag(crossDrag, "end").catch(() => undefined);
            stopAutoScroll();
            clearFencePreview();
            marker.remove();
            previews.forEach(({ itemId, preview }) => {
                preview.remove();
                dragElements.get(itemId)?.classList.remove("desktop-item--dragging");
            });
            fenceContents.forEach((content) => delete content.dataset.dragScrolling);
            void switchingSubscription?.then((unlisten) => unlisten()).catch(() => undefined);
            if (endInput) await onEndInputDrag().catch(() => undefined);
        };

        /** 结束连续拖动并提交一个布局事务或同步恢复。 */
        const finishDrag = async (finishType: string, pointerId: number) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishPointerDrag);
            handle.removeEventListener("pointercancel", finishPointerDrag);
            handle.removeEventListener("lostpointercapture", finishPointerDrag);
            window.removeEventListener("keydown", handleKeyDown);
            if (finishType !== "lostpointercapture" && handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);

            if (finishType === "shell-drag" && prepared) {
                const token = prepared.token; // 交接时只消费当前单次准备。
                prepared = null;
                restoreStartLayout();
                await cleanupDrag(false);
                try {
                    await onInteraction({ type: "focus-item", itemId: anchorItemId, selectionMode: preservesSelection ? "preserve" : pointerSelectionMode });
                    await onStartShellDrag(token, { left: pointerOffset.x, top: pointerOffset.y });
                } catch (error: unknown) {
                    onFailure(error instanceof Error ? error.message : "启动系统拖放失败");
                } finally {
                    await cancelDesktopItemDragPreparation(token).catch(() => undefined);
                    await onEndInputDrag().catch(() => undefined);
                }
                return;
            }
            if (prepared) {
                void cancelDesktopItemDragPreparation(prepared.token).catch(() => undefined);
                prepared = null;
            }

            if (finishType !== "pointerup" || !moved) {
                restoreStartLayout();
                if (finishType === "pointerup") {
                    await onInteraction({ type: "focus-item", itemId: anchorItemId, selectionMode: pointerSelectionMode }).catch(() => undefined);
                }
                await cleanupDrag();
                if (shouldOpenItemOnPointerRelease(preferences?.doubleClickToOpen ?? true, moved, finishType, openingModifiers)) {
                    await onOpenItems([anchorItemId]).catch((error: unknown) => onFailure(error instanceof Error ? error.message : "打开桌面项目失败"));
                }
                return;
            }

            const fenceScrolls = Array.from(fenceContents, ([fenceId, content]) => ({ fenceId, scrollTop: content.scrollTop })); // 拖动完成时的围栏滚动位置。
            let placements: { itemId: string; position: WorkspacePoint }[] = []; // 未归类投放提交的完整网格位置。
            let beforeItemId: string | null = null; // 围栏投放在移除拖动集合后的后继成员。
            let remoteDrop = false; // 跨屏落点必须以目标 DOM 的最终答复为准。
            if (crossDrag && crossDrag.target !== crossDrag.source) {
                try {
                    const drop = await resolveCrossMonitorDrop(crossDrag); // 释放点的目标围栏与插入位置。
                    if (drop.rejected) throw new Error("此格子不接受文件投放");
                    targetFenceId = drop.fenceId;
                    beforeItemId = drop.beforeItemId ?? null;
                    if (drop.fenceId && drop.scrollTop !== undefined) fenceScrolls.push({ fenceId: drop.fenceId, scrollTop: drop.scrollTop });
                    remoteDrop = true;
                } catch (error: unknown) {
                    restoreStartLayout();
                    onFailure(error instanceof Error ? error.message : "目标屏拖动失败");
                    await cleanupDrag();
                    return;
                }
            }
            if (!remoteDrop && rejectedByTile) {
                restoreStartLayout();
                onFailure("此格子不接受文件投放");
                await cleanupDrag();
                return;
            }
            if (targetFenceId) {
                const targetFence = workspace.fences.find((fence) => fence.id === targetFenceId); // 当前目标围栏权威快照。
                if (!remoteDrop) beforeItemId = targetFence ? beforeItemIdForFenceDrop(targetFence.members, dragItemIds, targetItemId, targetPlacement) : null;
            } else {
                const physicalPointer = currentMonitor ? clientPointToPhysical(latestClientPointer, currentMonitor) : undefined; // 当前释放点的虚拟桌面物理坐标。
                const targetMonitor = physicalPointer ? monitorAtPhysicalPoint(physicalPointer, monitors) : currentMonitor; // 当前释放点最终所属显示器。
                const targetGrid = targetMonitor ? completeWorkspace?.monitorGrids.find((entry) => entry.monitorId === targetMonitor.id)?.grid : undefined; // 目标显示器对应的独立未归类网格。
                const fenceMemberIds = new Set(completeWorkspace?.fences.flatMap((fence) => fence.members) ?? []); // 围栏成员的来源位置只用于解散恢复，不占用当前可见网格。
                const targetPositions = new Map(
                    (completeWorkspace?.unclassifiedItems ?? workspace.unclassifiedItems)
                        .filter(
                            (item) =>
                                !fenceMemberIds.has(item.stableReference) &&
                                (item.projectionMonitorId ?? item.monitorId) === (targetMonitor?.id ?? currentMonitor?.id),
                        )
                        .map((item) => [item.stableReference, item.projectionPosition ?? item.position] as const),
                ); // 目标显示器未归类区域的真实占用位置。
                dragItemIds.forEach((itemId) => {
                    const position = startPositions.get(itemId); // 参与跨屏落位的拖动项目来源位置。
                    if (position) targetPositions.set(itemId, position);
                });
                const targetPointer = physicalPointer && targetMonitor ? physicalPointToMonitorDip(physicalPointer, targetMonitor) : undefined; // 释放点在目标显示器中的 DIP。
                const desiredPosition = targetPointer
                    ? { left: targetPointer.left - pointerOffset.x, top: targetPointer.top - pointerOffset.y }
                    : anchorPosition
                      ? { left: anchorPosition.left + delta.x, top: anchorPosition.top + delta.y }
                      : undefined; // 拖动锚点在目标显示器中的期望 DIP 位置。
                const finalPositions =
                    freePlacement && desiredPosition
                        ? resolveFreeDrop(dragItemIds, desiredPosition, targetPositions)
                        : targetMonitor?.id === currentMonitor?.id && gridPreview
                          ? gridPreview
                          : desiredPosition
                            ? resolveGridDrop(dragItemIds, desiredPosition, targetPositions, targetGrid ?? workspace.grid)
                            : null; // 当前目标显示器未归类网格中的最终完整位置。
                if (!finalPositions) insertionBlocked = true;
                placements = finalPositions
                    ? dragItemIds.flatMap((itemId) => {
                          const position = finalPositions.get(itemId); // 只提交本次实际移动项目的最终网格位置。
                          return position ? [{ itemId, position }] : [];
                      })
                    : [];
                if (targetMonitor) targetFenceId = undefined;
            }

            if (insertionBlocked) {
                restoreStartLayout();
                onFailure("目标区域空间不足，项目已恢复原位");
                await cleanupDrag();
                return;
            }
            try {
                const physicalPointer = currentMonitor ? clientPointToPhysical(latestClientPointer, currentMonitor) : undefined; // 最终提交使用的虚拟桌面物理释放点。
                const targetMonitorId = physicalPointer ? monitorAtPhysicalPoint(physicalPointer, monitors)?.id : undefined; // 未归类跨屏提交的目标显示器身份。
                await onCommand(
                    {
                        type: "commit-item-drag",
                        anchorItemId,
                        itemIds: dragItemIds,
                        placements,
                        targetFenceId: targetFenceId ?? null,
                        beforeItemId,
                        fenceScrolls,
                    },
                    targetMonitorId,
                );
                if (crossDrag) crossDrag.committed = true;
            } catch {
                restoreStartLayout();
            }
            await cleanupDrag();
        };

        /** 将原生指针结束事件转入单一拖动收尾路径。 */
        const finishPointerDrag = (finishEvent: globalThis.PointerEvent) => {
            freePlacement = finishEvent.altKey;
            if (finishEvent.type === "pointerup" && currentMonitor && crossDrag) {
                latestClientPointer = { x: finishEvent.clientX, y: finishEvent.clientY };
                const physical = clientPointToPhysical(latestClientPointer, currentMonitor); // 释放事件可能晚于最后一帧移动。
                const target = monitorAtPhysicalPoint(physical, monitors); // 实际释放屏幕。
                if (target) crossDrag = { ...crossDrag, target: target.id, pointer: physicalPointToMonitorDip(physical, target) };
            }
            void finishDrag(finishEvent.type, finishEvent.pointerId);
        };

        /** Esc 使用与系统取消相同的同步恢复路径。 */
        const handleKeyDown = (keyEvent: globalThis.KeyboardEvent) => {
            if (keyEvent.key !== "Escape") return;
            keyEvent.preventDefault();
            void finishDrag("pointercancel", event.pointerId);
        };

        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishPointerDrag);
        handle.addEventListener("pointercancel", finishPointerDrag);
        handle.addEventListener("lostpointercapture", finishPointerDrag);
        window.addEventListener("keydown", handleKeyDown);
        switchingSubscription = listenVirtualDesktopSwitching(
            () => void finishDrag("virtual-desktop-switch", event.pointerId),
            () => undefined,
        );
        void prepareDesktopItemDrag(dragItemIds)
            .then((result) => {
                if (!result) return; // 无法准备完整选择时保留原指针与布局路径。
                if (finished) {
                    void cancelDesktopItemDragPreparation(result.token).catch(() => undefined);
                    return;
                }
                prepared = result;
                if (Math.abs(delta.x) >= result.threshold.left || Math.abs(delta.y) >= result.threshold.top) void finishDrag("shell-drag", event.pointerId);
            })
            .catch(() => undefined); // 准备失败不破坏正在进行的整组内部拖动。
    };

    return handleItemPointerDown;
};
