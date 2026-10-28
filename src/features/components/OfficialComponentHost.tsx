import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import { adoptPluginManifests, publishPluginShell, readPluginApi, subscribePluginShell, type PluginShellContext } from "../../bridge/pluginScripts";
import type { ComponentInstanceSnapshot, ComponentManifest, ComponentRuntimeSnapshot } from "../../contracts/component";
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
    pluginEpoch?: number; // 外壳注册或脚本失败后的重绘计数。
}

const attempted = new Set<string>(); // 已尝试插入脚本的标识，失败也不重试。
const scriptFailed = new Set<string>(); // 脚本 onerror 或标识无效。
const renderFailed = new Set<string>(); // renderTile 抛错的实例。
const tileCleanups = new Map<string, () => void>(); // 每个实例最近一次 renderTile 的清理函数。
const revisions = new Map<string, number>(); // 该插件外壳变化后的重绘代数。
const hostListeners = new Set<(pluginId: string) => void>(); // 宿主重绘订阅。

/** 通知宿主某个插件的脚本或注册变了。 */
const emitPluginHost = (pluginId: string) => {
    revisions.set(pluginId, (revisions.get(pluginId) ?? 0) + 1);
    for (const listener of hostListeners) listener(pluginId);
};

/** 订阅外壳注册与脚本失败。 */
const subscribePluginHost = (onChange: (pluginId: string) => void) => {
    hostListeners.add(onChange);
    return () => {
        hostListeners.delete(onChange);
    };
};

/** 当前插件的重绘代数。 */
const pluginRevision = (pluginId: string) => revisions.get(pluginId) ?? 0;

/** 脚本加载失败。 */
const pluginScriptFailed = (pluginId: string) => scriptFailed.has(pluginId);

/** 已注册但没有可用的 renderTile。 */
const pluginMissingRenderTile = (pluginId: string) => {
    const api = readPluginApi(pluginId); // 首份注册；未知标识不会在这里。
    return api !== undefined && typeof api.renderTile !== "function";
};

/** renderTile 抛错。 */
const pluginRenderFailed = (instanceId: string) => renderFailed.has(instanceId);

/** 把命令式渲染失败收成格子既有错误态。 */
const markPluginRenderFailed = (pluginId: string, instanceId: string) => {
    if (renderFailed.has(instanceId)) return;
    renderFailed.add(instanceId);
    emitPluginHost(pluginId);
};

/** 标识可以安全成为单一脚本路径段。 */
const pluginScriptName = (pluginId: string) =>
    pluginId.startsWith("plugin.") && !pluginId.includes("/") && !pluginId.includes("\\") && !pluginId.includes("%") && !pluginId.includes("..");

/** 调用最近一次 renderTile 留下的清理函数。 */
const releasePluginTile = (instanceId: string) => {
    const cleanup = tileCleanups.get(instanceId); // 上一轮作者返回的清理函数。
    tileCleanups.delete(instanceId);
    cleanup?.();
};

/** 先清理该实例上一轮内容，再把作者内容挂进容器。 */
const mountPluginTile = (container: HTMLElement, context: PluginShellContext) => {
    const renderTile = readPluginApi(context.pluginId)?.renderTile; // 多个实例共用这一份。
    if (typeof renderTile !== "function") return;
    releasePluginTile(context.instanceId);
    const cleanup = renderTile(container, context); // 作者自行决定容器里的 DOM。
    if (typeof cleanup === "function") tileCleanups.set(context.instanceId, cleanup);
};

/** 有实例的插件按标识插一次脚本。 */
const ensurePluginScript = (pluginId: string) => {
    if (attempted.has(pluginId)) return;
    attempted.add(pluginId);
    const owner = globalThis.document; // 测试与页面共用这一个入口。
    if (!owner || !pluginScriptName(pluginId)) {
        scriptFailed.add(pluginId);
        console.warn(`插件脚本无法加载：${pluginId}`);
        emitPluginHost(pluginId);
        return;
    }
    const script = owner.createElement("script"); // 只插入外部脚本，不放宽 CSP。
    script.src = `/plugins/${pluginId}.js`;
    script.async = true;
    script.onerror = () => {
        scriptFailed.add(pluginId);
        console.warn(`插件脚本加载失败：${pluginId}`);
        emitPluginHost(pluginId);
    };
    (owner.head ?? owner.documentElement).appendChild(script);
};

/** 从清单里取出当前屏幕真正出现的插件，去重后按标识顺序加载。 */
const syncPluginScripts = (manifests: readonly { id: string }[], instanceComponentIds: readonly string[]) => {
    publishPluginShell();
    const live = new Set(instanceComponentIds); // 当前屏幕的实例所属标识。
    const ids = new Set<string>(); // 要加载的插件标识。
    for (const manifest of manifests) if (manifest.id.startsWith("plugin.") && live.has(manifest.id)) ids.add(manifest.id);
    for (const pluginId of [...ids].sort()) ensurePluginScript(pluginId);
};

/** 把一个插件实例的作者内容挂到外框给出的容器上。 */
const PluginTileBody = ({ pluginId, instanceId, collapsed, revision }: { pluginId: string; instanceId: string; collapsed: boolean; revision: number }) => {
    const containerRef = useRef<HTMLDivElement>(null); // 作者内容的挂载点。
    const collapsedRef = useRef(collapsed); // 读取返回时使用当前折叠状态。
    const settingsRef = useRef<{ ready: boolean; value: unknown }>({ ready: false, value: null }); // 已读过的设置，折叠时不再等命令。
    collapsedRef.current = collapsed;

    useEffect(() => {
        const container = containerRef.current;
        if (!container) return;
        let alive = true; // 退订后拒绝迟到的设置读取。
        let ticket = 0; // 只采纳最新一次读取。
        const paint = (settings: unknown, mine: number) => {
            if (!alive || mine !== ticket || !containerRef.current) return;
            settingsRef.current = { ready: true, value: settings };
            try {
                mountPluginTile(containerRef.current, {
                    pluginId,
                    instanceId,
                    collapsed: collapsedRef.current,
                    settings,
                    saveSettings: (value) => savePluginSettings(pluginId, value),
                });
            } catch (error) {
                console.error(`组件渲染失败：${instanceId}`, error);
                markPluginRenderFailed(pluginId, instanceId);
            }
        };
        const read = () => {
            const mine = ++ticket; // 这一次读取的代号。
            void getPluginSettings(pluginId)
                .then((settings) => paint(settings ?? null, mine))
                .catch((error: unknown) => {
                    console.warn(`插件设置读取失败：${pluginId}`, error);
                    paint(null, mine);
                });
        };
        read();
        const subscription = listenPluginSettings(
            (changedId) => {
                if (changedId === pluginId) read();
            },
            (error) => console.warn("插件设置事件无效", error),
        );
        const handleRefresh = () => {
            if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            read();
        }; // 暂停期间事件不到达，恢复后按既有刷新通知重读。
        window.addEventListener("desktop-refresh-changed", handleRefresh);
        return () => {
            alive = false;
            settingsRef.current = { ready: false, value: null };
            window.removeEventListener("desktop-refresh-changed", handleRefresh);
            releasePluginTile(instanceId);
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [pluginId, instanceId, revision]);

    useEffect(() => {
        const container = containerRef.current;
        if (!container || !settingsRef.current.ready) return;
        try {
            mountPluginTile(container, {
                pluginId,
                instanceId,
                collapsed,
                settings: settingsRef.current.value,
                saveSettings: (value) => savePluginSettings(pluginId, value),
            });
        } catch (error) {
            console.error(`组件渲染失败：${instanceId}`, error);
            markPluginRenderFailed(pluginId, instanceId);
        }
    }, [collapsed, pluginId, instanceId]);

    return <div ref={containerRef} data-plugin-container={instanceId} />;
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
        (props, { runtime, preferences }) => {
            const pluginId = props.instance.componentId; // 格子所属插件，不按槽入口合并。
            const turnedOff = preferences?.disabledPlugins.includes(pluginId) ?? false; // 停用不是渲染失败。
            const broken = !turnedOff && (pluginScriptFailed(pluginId) || pluginMissingRenderTile(pluginId) || pluginRenderFailed(props.instance.instanceId)); // 脚本、注册或作者渲染失败。
            return (
                <ComponentTile
                    key={props.instance.instanceId}
                    {...props}
                    title={runtime?.manifests.find((manifest) => manifest.id === pluginId)?.name ?? pluginId}
                >
                    {turnedOff ? (
                        <p className="official-component__error">插件已停用</p>
                    ) : broken ? (
                        <p className="official-component__error">组件渲染失败</p>
                    ) : (
                        (collapsed) => (
                            <PluginTileBody
                                pluginId={pluginId}
                                instanceId={props.instance.instanceId}
                                collapsed={collapsed}
                                revision={pluginRevision(pluginId)}
                            />
                        )
                    )}
                </ComponentTile>
            );
        },
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
    const visibleInstances = instances.filter((instance) => pluginVisibility.get(instance.componentId)?.(pluginContext) ?? true); // 插件实例没有独立开关。
    const pluginLoadKey = visibleInstances
        .map((instance) => instance.componentId)
        .filter((id) => id.startsWith("plugin."))
        .sort()
        .join("\n"); // 当前屏幕出现过的插件标识。

    useEffect(() => {
        const stopHost = subscribePluginHost(() => setPluginEpoch((value) => value + 1)); // 脚本失败后重绘格子。
        const stopShell = subscribePluginShell((pluginId) => emitPluginHost(pluginId)); // 注册到达后重绘格子。
        return () => {
            stopHost();
            stopShell();
        };
    }, []);

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

    useEffect(() => {
        const manifests: ComponentManifest[] = runtime?.manifests ?? []; // 启动时冻结的清单。
        adoptPluginManifests(manifests);
        syncPluginScripts(manifests, pluginLoadKey ? pluginLoadKey.split("\n") : []);
    }, [runtime, pluginLoadKey]);

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
