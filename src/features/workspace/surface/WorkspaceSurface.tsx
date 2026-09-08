import { useEffect, useMemo, useState } from "react";

import {
    deleteDesktopItems,
    getDesktopItemSnapshot,
    listenDesktopItemSnapshots,
    openDesktopItems,
    pasteDesktopItems,
    renameDesktopItem,
    setDesktopItemClipboard,
    startDesktopItemDrag,
} from "../../../bridge/desktopItems";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import { FenceWorkspace } from "../../fences/FenceWorkspace";
import { useWorkspace } from "../state/useWorkspace";
import { useDesktopHostSurface } from "./useDesktopHostSurface";
import { useCrossMonitorPreview } from "./useCrossMonitorPreview";
import { projectWorkspaceSurface } from "./workspaceSurfaceProjection";

/** 将未知错误转换为桌面项目操作说明。 */
const errorMessage = (error: unknown): string => (error instanceof Error ? error.message : "桌面项目请求失败");

/** 装配逐屏展示、全局拖动上下文和原生命令结果。 */
export const WorkspaceSurface = () => {
    const monitorIndex = Number.parseInt(new URLSearchParams(window.location.search).get("monitor") ?? "0", 10) || 0; // 当前宿主索引。
    // 当前已发布的桌面项目快照。
    const [desktopItemSnapshot, setDesktopItemSnapshot] = useState<DesktopItemSnapshot>();
    // 当前 Shell 枚举或桌面项目契约错误说明。
    const [desktopItemFailure, setDesktopItemFailure] = useState<string>();
    // 当前用户可见的 Shell 操作实际结果。
    const [desktopItemStatus, setDesktopItemStatus] = useState<string>();
    // 当前透明宿主使用的权威工作区状态。
    const workspace = useWorkspace(true);
    useEffect(() => {
        if (workspace.status) setDesktopItemStatus(workspace.status);
    }, [workspace.status]); // 统一撤销或重做的部分恢复说明复用工作区状态区域呈现。
    // 正式透明宿主门槛与最小输入处理器。
    const desktopHost = useDesktopHostSurface(true, monitorIndex, desktopItemSnapshot, workspace.snapshot?.revision);
    useCrossMonitorPreview(desktopHost.host?.monitor.id, desktopItemSnapshot, workspace.snapshot);
    const projectedWorkspace = useMemo(
        () => projectWorkspaceSurface(workspace.snapshot, desktopHost.host?.monitor.id),
        [workspace.snapshot, desktopHost.host?.monitor.id],
    );
    const projectedDesktopItems = useMemo(() => {
        if (!desktopItemSnapshot || !projectedWorkspace) return desktopItemSnapshot;
        const visibleItemIds = new Set([
            ...projectedWorkspace.unclassifiedItems.map((item) => item.stableReference),
            ...projectedWorkspace.fences.flatMap((fence) => fence.members),
        ]); // 当前逐屏工作区实际引用的 Shell 项目集合。
        return { ...desktopItemSnapshot, items: desktopItemSnapshot.items.filter((item) => visibleItemIds.has(item.stableReference)) };
    }, [desktopItemSnapshot, projectedWorkspace]);

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

    /** 统一呈现 Shell 的实际结果，失败继续交给交互调用方恢复。 */
    const runShellOperation = async (operation: () => Promise<void | { message: string; permanent?: boolean }>) => {
        try {
            const result = await operation(); // Windows 确认的操作结果，打开操作没有状态消息。
            setDesktopItemFailure(undefined);
            if (result) setDesktopItemStatus(result.message + (result.permanent ? "（不可撤销）" : ""));
        } catch (error: unknown) {
            setDesktopItemFailure(errorMessage(error));
            throw error;
        }
    };

    /** 打开当前项目选择。 */
    const handleOpenItems = (itemIds: string[]) => runShellOperation(() => openDesktopItems(itemIds));
    /** 提交原生重命名。 */
    const handleRenameItem = (itemId: string, newName: string) => runShellOperation(() => renameDesktopItem(itemId, newName));
    /** 删除项目并保留永久删除提示。 */
    const handleDeleteItems = (itemIds: string[], permanent: boolean) => runShellOperation(() => deleteDesktopItems(itemIds, permanent));
    /** 将选择放入系统剪贴板。 */
    const handleSetClipboard = (itemIds: string[], cut: boolean) => runShellOperation(() => setDesktopItemClipboard(itemIds, cut));
    /** 执行系统粘贴。 */
    const handlePasteItems = () => runShellOperation(pasteDesktopItems);
    /** 将选择交给 OLE 拖放。 */
    const handleStartShellDrag = (itemIds: string[]) => runShellOperation(() => startDesktopItemDrag(itemIds));

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
};
