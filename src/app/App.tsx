import { useEffect, useMemo, useState } from "react";

import { getApplicationSnapshot, listenApplicationSnapshots, setApplicationStatus } from "../bridge/application";
import {
    deleteDesktopItems,
    getDesktopItemSnapshot,
    listenDesktopItemSnapshots,
    openDesktopItems,
    pasteDesktopItems,
    renameDesktopItem,
    setDesktopItemClipboard,
    startDesktopItemDrag,
} from "../bridge/desktopItems";
import type { ApplicationSnapshot, ApplicationStatus } from "../contracts/application";
import type { DesktopItemSnapshot } from "../contracts/desktopItem";
import { DesktopItemGrid } from "../features/desktop-items/DesktopItemGrid";
import { FenceWorkspace } from "../features/fences/FenceWorkspace";
import { useWorkspace } from "../features/workspace/state/useWorkspace";
import { useDesktopHostSurface } from "../features/workspace/surface/useDesktopHostSurface";
import "./App.css";

// 应用状态对应的用户可见文本。
const STATUS_LABELS: Record<ApplicationStatus, string> = {
    ready: "已就绪",
    active: "活动",
};

/** 将未知错误转换为用户可见说明。 */
const errorMessage = (error: unknown): string => (error instanceof Error ? error.message : "应用状态请求失败");

/** 呈现正式应用状态并验证 command/event 纵向链路。 */
export const App = () => {
    const isDesktopHost = new URLSearchParams(window.location.search).get("desktop-host") === "1"; // 正式透明宿主页面标记。
    const monitorIndex = Number.parseInt(new URLSearchParams(window.location.search).get("monitor") ?? "0", 10) || 0; // 当前逐屏宿主的枚举索引。
    // 当前已发布的应用状态快照。
    const [snapshot, setSnapshot] = useState<ApplicationSnapshot>();
    // 当前应用状态错误说明。
    const [failure, setFailure] = useState<string>();
    // 应用状态命令执行标记。
    const [isUpdating, setIsUpdating] = useState(false);
    // 当前已发布的桌面项目快照。
    const [desktopItemSnapshot, setDesktopItemSnapshot] = useState<DesktopItemSnapshot>();
    // 当前 Shell 枚举或桌面项目契约错误说明。
    const [desktopItemFailure, setDesktopItemFailure] = useState<string>();
    // 当前用户可见的 Shell 操作实际结果。
    const [desktopItemStatus, setDesktopItemStatus] = useState<string>();
    // 当前透明宿主使用的权威工作区状态。
    const workspace = useWorkspace(isDesktopHost);
    useEffect(() => {
        if (workspace.status) setDesktopItemStatus(workspace.status);
    }, [workspace.status]); // 统一撤销或重做的部分恢复说明复用工作区状态区域呈现。
    // 正式透明宿主门槛与最小输入处理器。
    const desktopHost = useDesktopHostSurface(isDesktopHost, monitorIndex, desktopItemSnapshot, workspace.snapshot?.revision);
    const projectedWorkspace = useMemo(() => {
        if (!workspace.snapshot || !desktopHost.host) return workspace.snapshot;
        const monitorId = desktopHost.host.monitor.id; // 当前页面所属的显示器稳定身份。
        const fences = workspace.snapshot.fences
            .filter((fence) => (fence.projectionMonitorId ?? fence.monitorId) === monitorId)
            .map((fence) => ({ ...fence, bounds: fence.projectionMonitorId === monitorId && fence.projectionBounds ? fence.projectionBounds : fence.bounds })); // 当前显示器可见的持久或临时投射围栏。
        const unclassifiedItems = workspace.snapshot.unclassifiedItems
            .filter((item) => (item.projectionMonitorId ?? item.monitorId) === monitorId)
            .map((item) => ({
                ...item,
                position: item.projectionMonitorId === monitorId && item.projectionPosition ? item.projectionPosition : item.position,
            })); // 当前显示器可见的持久或临时投射项目位置。
        const components = workspace.snapshot.components
            .filter((component) => (component.projectionMonitorId ?? component.monitorId) === monitorId)
            .map((component) => ({
                ...component,
                bounds: component.projectionMonitorId === monitorId && component.projectionBounds ? component.projectionBounds : component.bounds,
            })); // 当前显示器可见的持久或临时投射组件实例。
        const visibleItemIds = new Set([...unclassifiedItems.map((item) => item.stableReference), ...fences.flatMap((fence) => fence.members)]); // 当前页面可以呈现的完整项目集合。
        const focus = workspace.snapshot.interaction.keyboardFocus; // 当前全局工作区的键盘焦点。
        const interaction = {
            ...workspace.snapshot.interaction,
            selectedItemIds: workspace.snapshot.interaction.selectedItemIds.filter((itemId) => visibleItemIds.has(itemId)),
            selectionAnchorId:
                workspace.snapshot.interaction.selectionAnchorId && visibleItemIds.has(workspace.snapshot.interaction.selectionAnchorId)
                    ? workspace.snapshot.interaction.selectionAnchorId
                    : null,
            keyboardFocus:
                focus.type === "item" && !visibleItemIds.has(focus.itemId)
                    ? ({ type: "background" } as const)
                    : focus.type === "fence" && !fences.some((fence) => fence.id === focus.fenceId)
                      ? ({ type: "background" } as const)
                      : focus,
        }; // 当前逐屏页面隔离后的选择、锚点和焦点投影。
        return {
            ...workspace.snapshot,
            grid: workspace.snapshot.monitorGrids.find((entry) => entry.monitorId === monitorId)?.grid ?? workspace.snapshot.grid,
            fences,
            unclassifiedItems,
            components,
            interaction,
        };
    }, [desktopHost.host, workspace.snapshot]);
    const projectedDesktopItems = useMemo(() => {
        if (!desktopItemSnapshot || !projectedWorkspace) return desktopItemSnapshot;
        const visibleItemIds = new Set([
            ...projectedWorkspace.unclassifiedItems.map((item) => item.stableReference),
            ...projectedWorkspace.fences.flatMap((fence) => fence.members),
        ]); // 当前逐屏工作区实际引用的 Shell 项目集合。
        return { ...desktopItemSnapshot, items: desktopItemSnapshot.items.filter((item) => visibleItemIds.has(item.stableReference)) };
    }, [desktopItemSnapshot, projectedWorkspace]);

    useEffect(() => {
        if (isDesktopHost) return;
        // 当前状态页面挂载标记。
        let isMounted = true;

        // 接收 Rust 发布的权威应用快照。
        const handleSnapshot = (nextSnapshot: ApplicationSnapshot) => {
            if (!isMounted) return;
            setSnapshot(nextSnapshot);
            setFailure(undefined);
        };

        // 呈现 command、event 或契约错误。
        const handleFailure = (error: unknown) => {
            if (!isMounted) return;
            setFailure(errorMessage(error));
        };

        // 应用快照事件订阅。
        const subscription = listenApplicationSnapshots(handleSnapshot, handleFailure);
        void subscription.catch(handleFailure);
        void getApplicationSnapshot().then(handleSnapshot).catch(handleFailure);

        // 释放当前页面持有的事件订阅。
        return () => {
            isMounted = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [isDesktopHost]);

    useEffect(() => {
        // 当前桌面项目页面挂载标记。
        let isMounted = true;

        // 接收 Rust 发布的权威桌面项目快照。
        const handleSnapshot = (nextSnapshot: DesktopItemSnapshot) => {
            if (!isMounted) return;
            setDesktopItemSnapshot(nextSnapshot);
            setDesktopItemFailure(undefined);
        };

        // 呈现 Shell command、event 或契约错误并保留当前快照。
        const handleFailure = (error: unknown) => {
            if (!isMounted) return;
            setDesktopItemFailure(errorMessage(error));
        };

        // 桌面项目快照事件订阅。
        const subscription = listenDesktopItemSnapshots(handleSnapshot, handleFailure);
        void subscription.catch(handleFailure);
        void getDesktopItemSnapshot().then(handleSnapshot).catch(handleFailure);

        // 释放当前页面持有的桌面项目事件订阅。
        return () => {
            isMounted = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    /** 提交下一应用状态，页面由 event 收敛到权威快照。 */
    const handleToggleStatus = async () => {
        // 用户请求的下一应用状态。
        const nextStatus: ApplicationStatus = snapshot?.status === "active" ? "ready" : "active";
        setIsUpdating(true);
        setFailure(undefined);

        try {
            await setApplicationStatus(nextStatus);
        } catch (error: unknown) {
            setFailure(errorMessage(error));
        } finally {
            setIsUpdating(false);
        }
    };

    /** 使用 Shell 默认动词打开当前项目选择并呈现失败原因。 */
    const handleOpenItems = async (itemIds: string[]) => {
        try {
            await openDesktopItems(itemIds);
            setDesktopItemFailure(undefined);
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 使用 IFileOperation 重命名并呈现 Shell 实际结果。 */
    const handleRenameItem = async (itemId: string, newName: string) => {
        try {
            const result = await renameDesktopItem(itemId, newName); // Shell 确认的实际重命名结果。
            setDesktopItemFailure(undefined);
            setDesktopItemStatus(result.message);
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 将选择集合移入回收站或永久删除并呈现操作属性。 */
    const handleDeleteItems = async (itemIds: string[], permanent: boolean) => {
        try {
            const result = await deleteDesktopItems(itemIds, permanent); // Shell 确认的实际删除结果。
            setDesktopItemFailure(undefined);
            setDesktopItemStatus(result.message + (result.permanent ? "（不可撤销）" : ""));
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 将选择集合复制或剪切到系统 Shell 剪贴板。 */
    const handleSetClipboard = async (itemIds: string[], cut: boolean) => {
        try {
            const result = await setDesktopItemClipboard(itemIds, cut); // 系统剪贴板接受数据对象后的结果。
            setDesktopItemFailure(undefined);
            setDesktopItemStatus(result.message);
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 将系统 Shell 剪贴板粘贴到桌面命名空间。 */
    const handlePasteItems = async () => {
        try {
            const result = await pasteDesktopItems(); // Shell 确认的粘贴实际结果。
            setDesktopItemFailure(undefined);
            setDesktopItemStatus(result.message);
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 将当前选择集合交给系统 OLE 拖放。 */
    const handleStartShellDrag = async (itemIds: string[]) => {
        try {
            const result = await startDesktopItemDrag(itemIds); // 系统目标协商后的拖放结果。
            setDesktopItemFailure(undefined);
            setDesktopItemStatus(result.message);
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 使用当前或显式目标显示器提交逐屏布局事务。 */
    const handleWorkspaceCommand = async (command: Parameters<typeof workspace.applyCommand>[1], targetMonitorId?: string) => {
        const monitorId = targetMonitorId ?? desktopHost.host?.monitor.id; // 当前布局事务最终所属的显示器身份。
        if (!monitorId) throw new Error("当前显示器宿主尚未就绪");
        return workspace.applyCommand(monitorId, command);
    };

    /** 使用当前显示器提交逐屏交互事务。 */
    const handleWorkspaceInteraction = async (command: Parameters<typeof workspace.applyInteraction>[1]) => {
        const monitorId = desktopHost.host?.monitor.id; // 当前交互事务所属的显示器身份。
        if (!monitorId) throw new Error("当前显示器宿主尚未就绪");
        return workspace.applyInteraction(monitorId, command);
    };

    /** 使用当前显示器物理工作区显示原生菜单。 */
    const handleWorkspaceContextMenu = async (target: Parameters<typeof workspace.showContextMenu>[1], clientX: number, clientY: number) => {
        const monitorId = desktopHost.host?.monitor.id; // 当前菜单锚点所属的显示器身份。
        if (!monitorId) throw new Error("当前显示器宿主尚未就绪");
        return workspace.showContextMenu(monitorId, target, clientX, clientY);
    };

    if (isDesktopHost) {
        return (
            <main className="application-shell application-shell--desktop-host">
                <FenceWorkspace
                    desktopItems={projectedDesktopItems}
                    workspace={projectedWorkspace}
                    completeWorkspace={workspace.snapshot}
                    currentMonitor={desktopHost.host?.monitor}
                    monitors={desktopHost.snapshot?.hosts.map((host) => host.monitor) ?? []}
                    hostSnapshot={desktopHost.snapshot}
                    failure={desktopItemFailure ?? workspace.failure ?? desktopHost.failure}
                    status={desktopItemStatus}
                    onCommand={handleWorkspaceCommand}
                    onInteraction={handleWorkspaceInteraction}
                    onOpenItems={handleOpenItems}
                    onRenameItem={handleRenameItem}
                    onDeleteItems={handleDeleteItems}
                    onSetClipboard={handleSetClipboard}
                    onPasteItems={handlePasteItems}
                    onStartShellDrag={handleStartShellDrag}
                    onContextMenu={handleWorkspaceContextMenu}
                    onItemInput={desktopHost.handleItemInput}
                    onBeginInputDrag={desktopHost.beginInputDrag}
                    onEndInputDrag={desktopHost.endInputDrag}
                />
            </main>
        );
    }

    return (
        <main className="application-shell">
            <header className="application-header">
                <p className="application-brand">Desktop Tools</p>
                <span className="application-version">0.1.0</span>
            </header>

            <div className="application-content">
                <section className="status-workspace" aria-labelledby="status-title">
                    <div className="status-heading">
                        <div>
                            <p className="status-eyebrow">应用运行状态</p>
                            <h1 id="status-title">{snapshot ? STATUS_LABELS[snapshot.status] : "正在连接"}</h1>
                        </div>
                        <span className={`status-indicator status-indicator--${snapshot?.status ?? "loading"}`} aria-hidden="true" />
                    </div>

                    <dl className="status-details">
                        <div>
                            <dt>通信边界</dt>
                            <dd>Tauri command / event</dd>
                        </div>
                        <div>
                            <dt>快照修订</dt>
                            <dd>{snapshot?.revision ?? "--"}</dd>
                        </div>
                    </dl>

                    {failure && (
                        <p className="status-error" role="alert">
                            {failure}
                        </p>
                    )}

                    <button className="status-action" type="button" onClick={handleToggleStatus} disabled={!snapshot || isUpdating}>
                        {isUpdating ? "正在更新" : "切换运行状态"}
                    </button>
                </section>

                <DesktopItemGrid snapshot={desktopItemSnapshot} failure={desktopItemFailure} />
            </div>
        </main>
    );
};
