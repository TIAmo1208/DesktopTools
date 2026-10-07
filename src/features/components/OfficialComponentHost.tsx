import { useEffect, useMemo, useState, type ReactNode } from "react";
import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import type { ComponentInstanceSnapshot, ComponentRuntimeSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceSnapshot, WorkspaceFocusTarget } from "../../contracts/workspace";
import type { TileAppearance } from "../../contracts/tile";
import { usePreferences } from "../settings/PreferencesProvider";
import { ComponentTile, type ComponentTileProps } from "./ComponentTile";
import { CollectBoxTile } from "../tile-content/collect-box/CollectBoxTile";
import { MusicTile } from "../tile-content/music/MusicTile";
import { ClockTile } from "../tile-content/clock/LocalClock";
import { DebugTile } from "../tile-content/debug/DebugTest";
import "./OfficialComponentHost.css";

/** 官方组件宿主接收的工作区实例和命令边界。 */
interface OfficialComponentHostProps {
    tileAppearances?: Record<string, TileAppearance>; // 共享宿主外观。
    keyboardFocus?: WorkspaceFocusTarget; // 共享焦点和层级。
    debugContent?: ReactNode; // 调试格子的宿主信息，Release 不呈现。
    instances: ComponentInstanceSnapshot[];
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>;
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
}

/** 同步 Component Runtime 并呈现当前显示器的全部官方组件实例。 */
export const OfficialComponentHost = ({
    tileAppearances,
    keyboardFocus,
    debugContent,
    instances,
    currentMonitor,
    monitors,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
}: OfficialComponentHostProps) => {
    const { preferences } = usePreferences(); // 开关只控制可见实例。
    const [runtime, setRuntime] = useState<ComponentRuntimeSnapshot>(); // 当前后端组件注册表和实例状态。
    const [failure, setFailure] = useState<string>(); // 组件运行时 command 或 event 错误说明。
    const registrations = useMemo(() => new Map(runtime?.instances.map((instance) => [instance.instanceId, instance]) ?? []), [runtime]);
    const visibleInstances = instances.filter((instance) =>
        instance.componentId === "official.local-clock"
            ? preferences?.clockEnabled !== false
            : instance.componentId === "official.debug-test"
              ? preferences?.debugEnabled !== false && runtime?.manifests.some((manifest) => manifest.id === "official.debug-test")
              : instance.componentId === "official.music-control"
                ? preferences?.musicEnabled === true
                : instance.componentId === "official.collect-box"
                  ? preferences?.collectEnabled !== false
                  : true,
    ); // Release 没有 Debug 注册入口。

    useEffect(() => {
        let mounted = true; // 当前组件宿主挂载标记。
        const handleFailure = (error: unknown) => {
            if (mounted) setFailure(error instanceof Error ? error.message : "组件运行时请求失败");
        };
        const runtimeSubscription = listenComponentRuntimeSnapshots((snapshot) => mounted && setRuntime(snapshot), handleFailure);
        void runtimeSubscription.catch(handleFailure);
        return () => {
            mounted = false;
            void runtimeSubscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    useEffect(() => {
        void getComponentRuntimeSnapshot()
            .then((snapshot) => {
                setRuntime(snapshot);
                setFailure(undefined);
            })
            .catch((error: unknown) => setFailure(error instanceof Error ? error.message : "组件运行时请求失败"));
    }, [instances]);

    return (
        <>
            {(Boolean(failure) || Boolean(runtime?.failures.length)) && (
                <aside
                    className="component-runtime-failure"
                    role="alert"
                    data-hit-id="component-runtime-failure"
                    data-hit-kind="component-status"
                    data-hit-priority="85"
                >
                    {failure ?? runtime?.failures.map((item) => `${item.componentId}：${item.message}`).join("；")}
                </aside>
            )}
            {visibleInstances.map((instance) => {
                const props: ComponentTileProps = {
                    instance,
                    registration: registrations.get(instance.instanceId),
                    appearance: tileAppearances?.[`component:${instance.instanceId}`],
                    keyboardFocus,
                    currentMonitor,
                    monitors,
                    onCommand,
                    onInteraction,
                    onBeginInputDrag,
                    onEndInputDrag,
                }; // 每个实例沿用同一运行时与命令契约。
                if (instance.componentId === "official.collect-box") return <CollectBoxTile key={instance.instanceId} {...props} />;
                if (instance.componentId === "official.music-control") return <MusicTile key={instance.instanceId} {...props} />;
                if (instance.componentId === "official.local-clock") return <ClockTile key={instance.instanceId} {...props} />;
                if (instance.componentId === "official.debug-test")
                    return (
                        <DebugTile key={instance.instanceId} {...props}>
                            {debugContent}
                        </DebugTile>
                    );
                return (
                    <ComponentTile key={instance.instanceId} {...props} title="本地时间" minimum={{ width: 164, height: 90 }}>
                        <p className="official-component__error">组件前端入口不存在</p>
                    </ComponentTile>
                );
            })}
        </>
    );
};
