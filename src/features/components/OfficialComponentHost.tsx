import { useEffect, useMemo, useState, type ReactNode } from "react";
import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import { adoptPluginManifests, commandErrorMessage } from "../../bridge/pluginScripts";
import { readPluginMenuItems, subscribePluginMenuItems } from "../../bridge/pluginMenu";
import type { ComponentInstanceSnapshot, ComponentManifest, ComponentRuntimeSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceSnapshot, WorkspaceFocusTarget } from "../../contracts/workspace";
import type { TileAppearance } from "../../contracts/tile";
import type { ApplicationPreferences } from "../../contracts/settings";
import { usePreferences } from "../settings/PreferencesProvider";
import { ComponentTile, type ComponentTileProps } from "./ComponentTile";
import { tileRemoveMenuItem } from "../tiles/DesktopTileBase";
import { publishMessage } from "../notifications/messageNotification";
import { CollectBoxTile } from "../tile-content/collect-box/CollectBoxTile";
import { MusicTile } from "../tile-content/music/MusicTile";
import { ClockTile } from "../tile-content/clock/LocalClock";
import { DebugTile } from "../tile-content/debug/DebugTest";
import { WeatherTile } from "../tile-content/weather/WeatherTile";
import { pluginMenuItems as toPluginMenuItems, pluginTileMinimum, pluginTileTitle } from "./pluginTile";

import {
    subscribePluginHost,
    pluginRevision,
    pluginTileFailed,
    pluginTileType,
    syncPluginScripts,
    PluginTileBody,
} from "./PluginSlot";

const reportedRuntimeFailures = new Set<string>(); // 已经提示过的标识和原因。

/** 把运行时失败交给消息和日志。同一条还在时不再重复。 */
export const reportRuntimeFailures = (
    commandFailure: string | undefined,
    failures: readonly { componentId: string; message: string }[] | undefined,
    notify: (content: string) => void = notifyRuntimeFailure,
) => {
    const live = new Set<string>(); // 这一轮仍然存在的失败。
    if (commandFailure) live.add(commandFailure);
    for (const item of failures ?? []) live.add(`${item.componentId}：${item.message}`);
    for (const line of live) {
        if (reportedRuntimeFailures.has(line)) continue;
        reportedRuntimeFailures.add(line);
        notify(line);
    }
    for (const line of reportedRuntimeFailures) if (!live.has(line)) reportedRuntimeFailures.delete(line);
};

const notifyRuntimeFailure = (content: string) => {
    console.warn(content); // WARN：标识和原因原样留下。
    publishMessage({ type: "error", content, source: "component-runtime" });
};

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
    pluginEpoch?: number; // 外壳注册或脚本失败后的重绘计数。
}

/** 插件格子外框：合并作者菜单、类型级最小尺寸与移除项。 */
const PluginTileFrame = ({ tileProps, pluginId, tileType, runtime }: { tileProps: ComponentTileProps; pluginId: string; tileType: string; runtime?: ComponentRuntimeSnapshot }) => {
    const instanceId = tileProps.instance.instanceId; // 菜单项按实例归属。
    const [entries, setEntries] = useState(() => readPluginMenuItems(instanceId)); // 最近一轮 renderTile 注册的菜单。
    useEffect(() => {
        setEntries(readPluginMenuItems(instanceId)); // 重挂前先对齐当前实例。
        return subscribePluginMenuItems(instanceId, () => setEntries(readPluginMenuItems(instanceId)));
    }, [instanceId]);
    const manifest = runtime?.manifests.find((item) => item.id === pluginId); // 当前插件的类型声明。
    const broken = pluginTileFailed(pluginId, instanceId); // 脚本、注册或作者渲染失败。
    const removable = manifest?.removable === true; // 清单缺失时不可移除。
    return (
        <ComponentTile
            key={instanceId}
            {...tileProps}
            title={pluginTileTitle(manifest, tileType, manifest?.name ?? pluginId)} // 每类格子用自己的类型名作标题。
            minimum={pluginTileMinimum(manifest, tileType)}
            menuItems={[
                ...toPluginMenuItems(entries),
                tileRemoveMenuItem(
                    removable,
                    () =>
                        void tileProps.onCommand({ type: "remove-component", instanceId }).catch((error: unknown) => {
                            publishMessage({ type: "error", content: commandErrorMessage(error), source: "desktop-tile" });
                        }),
                ),
            ]}
        >
            {broken ? (
                <p className="official-component__error">组件渲染失败</p>
            ) : (
                (collapsed) => <PluginTileBody pluginId={pluginId} instanceId={instanceId} tileType={tileType} collapsed={collapsed} revision={pluginRevision(pluginId)} />
            )}
        </ComponentTile>
    );
};

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

/** 官方开关和插件停用名单走同一次判定。停用的插件整格不出现。 */
const instanceVisible = (componentId: string, context: PluginHostContext) => {
    const listed = pluginVisibility.get(componentId); // 官方组件的显隐开关。
    if (listed) return listed(context);
    if (componentId.startsWith("plugin.")) return !(context.preferences?.disabledPlugins?.includes(componentId) ?? false); // 名单里的插件不渲染。
    return true;
};

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
            <PluginTileFrame
                key={props.instance.instanceId}
                tileProps={props}
                pluginId={props.instance.componentId}
                tileType={pluginTileType(props.instance.state)}
                runtime={runtime}
            />
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
    const [pluginEpoch, setPluginEpoch] = useState(0); // 外壳注册或脚本状态变化。
    const registrations = useMemo(() => new Map(runtime?.instances.map((instance) => [instance.instanceId, instance]) ?? []), [runtime]);
    const pluginContext = { preferences, runtime, debugContent, pluginEpoch }; // 本次呈现的偏好、注册表与调试内容。
    const visibleInstances = instances.filter((instance) => instanceVisible(instance.componentId, pluginContext)); // 停用插件与关闭的官方组件都不进列表。
    const pluginLoadKey = visibleInstances
        .map((instance) => instance.componentId)
        .filter((id) => id.startsWith("plugin."))
        .sort()
        .join("\n"); // 当前屏幕出现过的插件标识。

    useEffect(() => {
        const stopHost = subscribePluginHost(() => setPluginEpoch((value) => value + 1)); // 脚本失败后重绘格子。
        return () => {
            stopHost();
        };
    }, []);

    useEffect(() => {
        let mounted = true; // 当前组件宿主挂载标记。
        const handleFailure = (error: unknown) => {
            if (mounted) setFailure(commandErrorMessage(error));
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
            .catch((error: unknown) => setFailure(commandErrorMessage(error)));
    }, [instances]);

    useEffect(() => {
        reportRuntimeFailures(failure, runtime?.failures);
    }, [failure, runtime]);

    useEffect(() => {
        const manifests: ComponentManifest[] = runtime?.manifests ?? []; // 启动时冻结的清单。
        adoptPluginManifests(manifests);
        syncPluginScripts(manifests, pluginLoadKey ? pluginLoadKey.split("\n") : []);
    }, [runtime, pluginLoadKey]);

    return (
        <>
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
