import { useEffect, useMemo, useState, type ReactNode } from "react";
import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import type { ComponentInstanceSnapshot, ComponentRuntimeSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceSnapshot, WorkspaceFocusTarget } from "../../contracts/workspace";
import type { TileAppearance } from "../../contracts/tile";
import type { ApplicationPreferences } from "../../contracts/settings";
import { usePreferences } from "../settings/PreferencesProvider";
import { ComponentTile, type ComponentTileProps } from "./ComponentTile";
import { CollectBoxTile } from "../tile-content/collect-box/CollectBoxTile";
import { MusicTile } from "../tile-content/music/MusicTile";
import { ClockTile } from "../tile-content/clock/LocalClock";
import { DebugTile } from "../tile-content/debug/DebugTest";
import { WeatherTile } from "../tile-content/weather/WeatherTile";
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

/** 插件宿主显隐与内容分派所需的当前状态。 */
interface PluginHostContext {
    preferences?: ApplicationPreferences; // 官方功能开关。
    runtime?: ComponentRuntimeSnapshot; // 当前可用清单。
    debugContent?: ReactNode; // 调试实例的宿主诊断内容。
}

// 未列出的插件保持可见；Release 没有 Debug 注册入口。
const pluginVisibility = new Map<string, (context: PluginHostContext) => boolean>([
    ["official.local-clock", ({ preferences }) => preferences?.clockEnabled !== false],
    [
        "official.debug-test",
        ({ preferences, runtime }) =>
            preferences?.debugEnabled !== false && Boolean(runtime?.manifests.some((manifest) => manifest.id === "official.debug-test")),
    ],
    ["official.music-control", ({ preferences }) => preferences?.musicEnabled === true],
    ["official.collect-box", ({ preferences }) => preferences?.collectEnabled !== false],
    ["official.weather", ({ preferences }) => preferences?.weatherEnabled === true],
]);

// 官方 id 仅用于注册快照尚未到达时，和对应 entry 共用同一内容入口。
const pluginRenderers = new Map<string, (props: ComponentTileProps, context: PluginHostContext) => ReactNode>([
    ["official:collect-box", (props) => <CollectBoxTile key={props.instance.instanceId} {...props} />],
    ["official:weather", (props) => <WeatherTile key={props.instance.instanceId} {...props} />],
    ["official:music-control", (props) => <MusicTile key={props.instance.instanceId} {...props} />],
    ["official:local-clock", (props) => <ClockTile key={props.instance.instanceId} {...props} />],
    [
        "official:debug-test",
        (props, { debugContent }) => (
            <DebugTile key={props.instance.instanceId} {...props}>
                {debugContent}
            </DebugTile>
        ),
    ],
    [
        "plugin:slot",
        (props, { runtime }) => (
            <ComponentTile
                key={props.instance.instanceId}
                {...props}
                title={runtime?.manifests.find((manifest) => manifest.id === props.instance.componentId)?.name ?? props.instance.componentId}
            >
                <div data-plugin-container={props.instance.instanceId}>插件内容尚未加载</div>
            </ComponentTile>
        ),
    ],
]);
for (const [entry, render] of Array.from(pluginRenderers)) {
    if (entry.startsWith("official:")) pluginRenderers.set(entry.replace("official:", "official."), render);
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
    const pluginContext = { preferences, runtime, debugContent }; // 本次呈现的偏好、注册表与调试内容。
    const visibleInstances = instances.filter((instance) => pluginVisibility.get(instance.componentId)?.(pluginContext) ?? true); // 插件实例没有独立开关。

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
                const pluginEntry = props.registration?.entry ?? instance.componentId; // 注册入口优先，尚未同步时沿用官方身份。
                const pluginRender = pluginRenderers.get(pluginEntry); // 所有插件实例共用 plugin:slot。
                if (pluginRender) return pluginRender(props, { ...pluginContext, runtime });
                return (
                    <ComponentTile key={instance.instanceId} {...props} title="本地时间" minimum={{ width: 164, height: 90 }}>
                        <p className="official-component__error">组件前端入口不存在</p>
                    </ComponentTile>
                );
            })}
        </>
    );
};
