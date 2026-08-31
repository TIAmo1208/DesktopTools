import { useEffect, useMemo, useRef, type PointerEvent } from "react";

import type { DesktopItemSnapshot } from "../../contracts/desktopItem";
import {
    FENCE_MIN_HEIGHT,
    FENCE_MIN_WIDTH,
    type FenceSnapshot,
    type WorkspaceCommand,
    type WorkspaceRect,
    type WorkspaceSnapshot,
} from "../../contracts/workspace";
import { DesktopItemTile } from "../desktop-items/DesktopItemGrid";
import { projectDesktopItems, type DesktopItemViewModel } from "../desktop-items/desktopItemViewModel";
import { moveFence, resizeFence, type FenceResizeDirection } from "./fenceGeometry";
import "./FenceWorkspace.css";

/** 围栏工作区接收的权威快照与命令边界。 */
interface FenceWorkspaceProps {
    desktopItems?: DesktopItemSnapshot;
    workspace?: WorkspaceSnapshot;
    failure?: string;
    onCommand: (command: WorkspaceCommand) => Promise<WorkspaceSnapshot>;
    onItemInput?: (itemId: string) => void;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
}

/** 单个围栏的呈现与交互参数。 */
interface FencePanelProps {
    fence: FenceSnapshot;
    items: DesktopItemViewModel[];
    onCommand: (command: WorkspaceCommand) => Promise<WorkspaceSnapshot>;
    onDissolve: (fenceId: string) => void;
    onItemPointerDown: (event: PointerEvent<HTMLLIElement>, item: DesktopItemViewModel) => void;
    onItemInput?: (itemId: string) => void;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
}

// 围栏提供的八个缩放边框方向。
const RESIZE_DIRECTIONS: FenceResizeDirection[] = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];

/** 将权威围栏边界直接投射到交互预览 DOM。 */
const applyFenceBounds = (element: HTMLElement, bounds: WorkspaceRect, collapsed = false) => {
    element.style.left = `${bounds.left}px`;
    element.style.top = `${bounds.top}px`;
    element.style.width = `${bounds.width}px`;
    element.style.height = `${collapsed ? 44 : bounds.height}px`;
};

/** 呈现一个可移动、缩放、折叠、滚动和接收项目的围栏。 */
const FencePanel = ({ fence, items, onCommand, onDissolve, onItemPointerDown, onItemInput, onBeginInputDrag, onEndInputDrag }: FencePanelProps) => {
    const fenceRef = useRef<HTMLElement>(null); // 当前围栏表面节点。
    const contentRef = useRef<HTMLUListElement>(null); // 当前围栏滚动内容节点。

    useEffect(() => {
        if (contentRef.current) contentRef.current.scrollTop = fence.scrollTop;
    }, [fence.scrollTop]);

    /** 开始围栏移动或缩放，并在指针释放时提交一次边界。 */
    const beginGeometry = async (event: PointerEvent<HTMLElement>, direction?: FenceResizeDirection) => {
        if (event.button !== 0 || (event.target instanceof Element && event.target.closest("button"))) return;
        event.preventDefault();
        event.stopPropagation();
        // 捕获当前连续指针交互的围栏节点。
        const element = fenceRef.current;
        if (!element) return;
        // 指针捕获和原生监听所属的交互节点。
        const handle = event.currentTarget;
        // 当前交互开始时的指针位置。
        const startPointer = { x: event.clientX, y: event.clientY };
        // 当前交互开始时的权威围栏边界。
        const startBounds = { ...fence.bounds };
        // 指针移动期间持续更新的预览边界。
        let preview = startBounds;
        let finished = false; // 当前连续指针交互结束标记。
        try {
            await onBeginInputDrag();
        } catch {
            return;
        }
        try {
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        // 根据最新指针更新直接 DOM 预览。
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            // 当前指针相对交互起点的位移。
            const delta = { x: moveEvent.clientX - startPointer.x, y: moveEvent.clientY - startPointer.y };
            // 当前透明宿主工作区尺寸。
            const viewport = { width: window.innerWidth, height: window.innerHeight };
            preview = direction ? resizeFence(direction, startBounds, delta, viewport) : moveFence(startBounds, delta, viewport);
            applyFenceBounds(element, preview, fence.collapsed);
        };

        // 结束指针交互并提交或恢复权威边界。
        const finishGeometry = (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishGeometry);
            handle.removeEventListener("pointercancel", finishGeometry);
            handle.removeEventListener("lostpointercapture", finishGeometry);
            if (finishEvent.type === "pointerup") {
                void onCommand({ type: "update-fence-bounds", fenceId: fence.id, bounds: preview })
                    .catch(() => applyFenceBounds(element, startBounds, fence.collapsed))
                    .finally(() => onEndInputDrag().catch(() => undefined));
                return;
            }
            applyFenceBounds(element, startBounds, fence.collapsed);
            void onEndInputDrag().catch(() => undefined);
        };

        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishGeometry);
        handle.addEventListener("pointercancel", finishGeometry);
        handle.addEventListener("lostpointercapture", finishGeometry);
    };

    /** 将当前原生滚动位置立即提交到权威工作区。 */
    const handleScroll = () => {
        void onCommand({ type: "set-fence-scroll", fenceId: fence.id, scrollTop: contentRef.current?.scrollTop ?? 0 }).catch(() => undefined);
    };

    return (
        <section
            ref={fenceRef}
            className={`fence${fence.collapsed ? " fence--collapsed" : ""}`}
            data-fence-id={fence.id}
            data-layout-mode={fence.layoutMode}
            data-hit-id={fence.id}
            data-hit-kind="fence"
            data-hit-priority="50"
            style={{
                left: fence.bounds.left,
                top: fence.bounds.top,
                width: fence.bounds.width,
                height: fence.collapsed ? 44 : fence.bounds.height,
                minWidth: FENCE_MIN_WIDTH,
                minHeight: fence.collapsed ? 42 : FENCE_MIN_HEIGHT,
                zIndex: fence.zIndex,
            }}
            aria-label={`围栏 ${fence.title}`}
        >
            <header className="fence__header" onPointerDown={(event) => void beginGeometry(event)}>
                <h2>{fence.title}</h2>
                <span className="fence__count">{fence.members.length}</span>
                <div className="fence__commands">
                    <button
                        type="button"
                        title={fence.layoutMode === "tile" ? "切换为列表" : "切换为平铺"}
                        onClick={() =>
                            void onCommand({
                                type: "set-fence-layout",
                                fenceId: fence.id,
                                layoutMode: fence.layoutMode === "tile" ? "list" : "tile",
                            }).catch(() => undefined)
                        }
                    >
                        {fence.layoutMode === "tile" ? "列表" : "平铺"}
                    </button>
                    <button
                        type="button"
                        title={fence.collapsed ? "展开围栏" : "折叠围栏"}
                        onClick={() => void onCommand({ type: "set-fence-collapsed", fenceId: fence.id, collapsed: !fence.collapsed }).catch(() => undefined)}
                    >
                        {fence.collapsed ? "展开" : "收起"}
                    </button>
                    <button type="button" title="解散围栏" onClick={() => onDissolve(fence.id)}>
                        解散
                    </button>
                </div>
            </header>

            {!fence.collapsed && (
                <ul ref={contentRef} className="fence__items" onScroll={handleScroll}>
                    {items.map((item) => (
                        <DesktopItemTile
                            key={item.key}
                            item={item}
                            desktopHost
                            className="desktop-item--fence"
                            fenceMemberId={item.key}
                            onItemInput={onItemInput}
                            onItemPointerDown={(event) => onItemPointerDown(event, item)}
                        />
                    ))}
                    {items.length === 0 && <li className="fence__empty">拖入桌面项目</li>}
                </ul>
            )}

            {!fence.collapsed &&
                RESIZE_DIRECTIONS.map((direction) => (
                    <span
                        className={`fence__resize fence__resize--${direction}`}
                        key={direction}
                        aria-hidden="true"
                        onPointerDown={(event) => void beginGeometry(event, direction)}
                    />
                ))}
        </section>
    );
};

/** 呈现正式透明宿主中的未归类区域与全部围栏。 */
export const FenceWorkspace = ({ desktopItems, workspace, failure, onCommand, onItemInput, onBeginInputDrag, onEndInputDrag }: FenceWorkspaceProps) => {
    // 当前 Shell 项目对应的页面展示模型。
    const items = useMemo(() => (desktopItems ? projectDesktopItems(desktopItems) : []), [desktopItems]);
    // 当前稳定引用对应的桌面项目展示模型。
    const itemsById = useMemo(() => new Map(items.map((item) => [item.key, item])), [items]);
    // 当前全部围栏成员标识。
    const memberIds = useMemo(() => new Set(workspace?.fences.flatMap((fence) => fence.members) ?? []), [workspace]);
    // 当前保留的未归类项目来源位置。
    const sourcePositions = useMemo(() => new Map(workspace?.unclassifiedItems.map((item) => [item.stableReference, item.position]) ?? []), [workspace]);
    // 当前未归属围栏的桌面项目。
    const unclassifiedItems = items.filter((item) => !memberIds.has(item.key));

    /** 按原型路径等待宿主解除裁剪，再建立桌面项目连续指针拖动。 */
    const handleItemPointerDown = async (event: PointerEvent<HTMLLIElement>, item: DesktopItemViewModel) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        // 当前拖动项目的交互节点。
        const handle = event.currentTarget;
        if (!sourcePositions.has(item.key)) return;
        // 当前连续拖动开始时的项目边界。
        const startBounds = handle.getBoundingClientRect();
        // 当前连续拖动开始时的指针位置。
        const startPointer = { x: event.clientX, y: event.clientY };
        const startedInFence = memberIds.has(item.key); // 当前项目拖动开始时的逻辑归属。

        try {
            await onBeginInputDrag();
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        // 跟随指针且不参与命中计算的项目预览。
        const preview = handle.cloneNode(true) as HTMLElement;
        preview.removeAttribute("data-hit-id");
        preview.removeAttribute("data-fence-member-id");
        preview.classList.add("desktop-item--drag-preview");
        Object.assign(preview.style, {
            left: `${startBounds.left}px`,
            top: `${startBounds.top}px`,
            width: `${startBounds.width}px`,
            height: `${startBounds.height}px`,
        });
        document.body.append(preview);
        let hoveredFence: HTMLElement | undefined; // 当前指针命中的围栏表面。
        let finished = false; // 当前连续指针拖动结束标记。

        /** 返回当前客户区位置命中的围栏和可选后继成员。 */
        const resolveDropTarget = (clientX: number, clientY: number) => {
            // 当前指针位置下等待归属判断的 DOM 元素。
            for (const element of document.elementsFromPoint(clientX, clientY)) {
                // 当前命中元素所属的最高围栏表面。
                const fence = element.closest<HTMLElement>("[data-fence-id]");
                if (!fence) continue;
                // 当前命中元素所属的可选围栏成员。
                const member = element.closest<HTMLElement>("[data-fence-member-id]");
                return { fence, beforeItemId: member?.dataset.fenceMemberId ?? null };
            }
            return undefined;
        };

        // 直接更新拖动预览与围栏投放状态。
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            preview.style.left = `${startBounds.left + moveEvent.clientX - startPointer.x}px`;
            preview.style.top = `${startBounds.top + moveEvent.clientY - startPointer.y}px`;
            // 当前指针位置命中的投放围栏。
            const targetFence = resolveDropTarget(moveEvent.clientX, moveEvent.clientY)?.fence;
            if (targetFence === hoveredFence) return;
            hoveredFence?.classList.remove("fence--drop-target");
            hoveredFence = targetFence;
            hoveredFence?.classList.add("fence--drop-target");
        };

        // 结束连续拖动并提交一次围栏归属事务。
        const finishPointerDrag = async (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishPointerDrag);
            handle.removeEventListener("pointercancel", finishPointerDrag);
            handle.removeEventListener("lostpointercapture", finishPointerDrag);
            hoveredFence?.classList.remove("fence--drop-target");
            preview.remove();

            try {
                if (finishEvent.type !== "pointerup") return;
                // 指针释放位置对应的最终投放目标。
                const dropTarget = resolveDropTarget(finishEvent.clientX, finishEvent.clientY);
                if (dropTarget && dropTarget.beforeItemId !== item.key) {
                    await onCommand({
                        type: "move-item-to-fence",
                        itemId: item.key,
                        fenceId: dropTarget.fence.dataset.fenceId ?? "",
                        beforeItemId: dropTarget.beforeItemId,
                    });
                } else if (!dropTarget && startedInFence) {
                    await onCommand({ type: "remove-item-from-fence", itemId: item.key });
                }
            } catch {
                // 工作区 hook 已保留可见错误和最近权威快照。
            } finally {
                await onEndInputDrag().catch(() => undefined);
            }
        };

        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishPointerDrag);
        handle.addEventListener("pointercancel", finishPointerDrag);
        handle.addEventListener("lostpointercapture", finishPointerDrag);
    };

    /** 提交解散围栏领域意图。 */
    const handleDissolveFence = (fenceId: string) => {
        void onCommand({ type: "dissolve-fence", fenceId }).catch(() => undefined);
    };

    /** 创建位于当前工作区可见范围内的新空围栏。 */
    const handleCreateFence = () => {
        // 当前围栏数量用于错开新围栏起点。
        const index = workspace?.fences.length ?? 0;
        // 当前宿主可以容纳的围栏初始宽度。
        const width = Math.max(FENCE_MIN_WIDTH, Math.min(340, window.innerWidth - 40));
        // 当前宿主可以容纳的围栏初始高度。
        const height = Math.max(FENCE_MIN_HEIGHT, Math.min(260, window.innerHeight - 40));
        // 新围栏在可见工作区中的初始位置。
        const bounds = {
            left: Math.max(0, Math.min(window.innerWidth - width, 120 + (index % 6) * 24)),
            top: Math.max(0, Math.min(window.innerHeight - height, 80 + (index % 6) * 24)),
            width,
            height,
        };
        void onCommand({ type: "create-fence", title: `新围栏 ${index + 1}`, bounds }).catch(() => undefined);
    };

    return (
        <main className="fence-workspace">
            <div className="fence-workspace__toolbar" data-hit-id="workspace-toolbar" data-hit-kind="workspace-command" data-hit-priority="90">
                <button type="button" onClick={handleCreateFence} disabled={!workspace}>
                    + 新建围栏
                </button>
                <span>布局修订 {workspace?.revision ?? "--"}</span>
            </div>

            {failure && (
                <p className="fence-workspace__error" role="alert" data-hit-id="workspace-error" data-hit-kind="workspace-status" data-hit-priority="80">
                    {failure}
                </p>
            )}

            <ul className="fence-workspace__unclassified" aria-label="未归类区域">
                {unclassifiedItems.map((item) => {
                    // 项目持久化位置或首次呈现的 Windows 风格纵向网格位置。
                    const position = sourcePositions.get(item.key);
                    if (!position) return null;
                    return (
                        <DesktopItemTile
                            key={item.key}
                            item={item}
                            desktopHost
                            className="desktop-item--unclassified"
                            style={{ left: position.left, top: position.top }}
                            onItemInput={onItemInput}
                            onItemPointerDown={(event) => void handleItemPointerDown(event, item)}
                        />
                    );
                })}
            </ul>

            {workspace?.fences.map((fence) => (
                <FencePanel
                    key={fence.id}
                    fence={fence}
                    items={fence.members.flatMap((itemId) => {
                        const item = itemsById.get(itemId); // Shell 当前可解析或保留的围栏成员。
                        return item ? [item] : [];
                    })}
                    onCommand={onCommand}
                    onDissolve={handleDissolveFence}
                    onItemPointerDown={(event, item) => void handleItemPointerDown(event, item)}
                    onItemInput={onItemInput}
                    onBeginInputDrag={onBeginInputDrag}
                    onEndInputDrag={onEndInputDrag}
                />
            ))}
        </main>
    );
};
