import { useEffect, useState } from "react";

import { getApplicationSnapshot, listenApplicationSnapshots, setApplicationStatus } from "../bridge/application";
import { getDesktopItemSnapshot, listenDesktopItemSnapshots } from "../bridge/desktopItems";
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
    // 当前透明宿主使用的权威工作区状态。
    const workspace = useWorkspace(isDesktopHost);
    // 正式透明宿主门槛与最小输入处理器。
    const desktopHost = useDesktopHostSurface(isDesktopHost, desktopItemSnapshot, workspace.snapshot?.revision);

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

    if (isDesktopHost) {
        return (
            <main className="application-shell application-shell--desktop-host">
                <FenceWorkspace
                    desktopItems={desktopItemSnapshot}
                    workspace={workspace.snapshot}
                    failure={desktopItemFailure ?? workspace.failure ?? desktopHost.failure}
                    onCommand={workspace.applyCommand}
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
