import { useEffect, useRef, useState } from "react";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import {
    adoptPluginManifests,
    loadPluginScript,
    pluginScriptFailure,
    readPluginApi,
    subscribePluginShell,
    type PluginShellContext,
} from "../../bridge/pluginScripts";
import type { ComponentManifest } from "../../contracts/component";

const attempted = new Set<string>(); // 已尝试插入脚本的标识，失败也不重试。

const renderFailed = new Set<string>(); // renderTile 抛错的实例。
const tileCleanups = new Map<string, () => void>(); // 每个实例最近一次 renderTile 的清理函数。
const revisions = new Map<string, number>(); // 该插件外壳变化后的重绘代数。
const hostListeners = new Set<(pluginId: string) => void>(); // 宿主重绘订阅。
let stopShell: (() => void) | undefined; // 同一页面只监听一次外壳注册。

/** 通知宿主某个插件的脚本或注册变了。 */
export const emitPluginHost = (pluginId: string) => {
    revisions.set(pluginId, (revisions.get(pluginId) ?? 0) + 1);
    for (const listener of hostListeners) listener(pluginId);
};

/** 订阅外壳注册与脚本失败。 */
export const subscribePluginHost = (onChange: (pluginId: string) => void) => {
    if (!hostListeners.size) stopShell = subscribePluginShell(emitPluginHost);
    hostListeners.add(onChange);
    return () => {
        hostListeners.delete(onChange);
        if (!hostListeners.size) {
            stopShell?.();
            stopShell = undefined;
        }
    };
};

/** 当前插件的重绘代数。 */
export const pluginRevision = (pluginId: string) => revisions.get(pluginId) ?? 0;

/** 脚本加载失败。 */
export const pluginScriptFailed = (pluginId: string) => Boolean(pluginScriptFailure(pluginId));

/** 已注册但没有可用的 renderTile。 */
export const pluginMissingRenderTile = (pluginId: string) => {
    const api = readPluginApi(pluginId); // 首份注册；未知标识不会在这里。
    return api !== undefined && typeof api.renderTile !== "function";
};

/** renderTile 抛错。 */
export const pluginRenderFailed = (instanceId: string) => renderFailed.has(instanceId);

/** 把命令式渲染失败收成格子既有错误态。 */
const markPluginRenderFailed = (pluginId: string, instanceId: string) => {
    if (renderFailed.has(instanceId)) return;
    renderFailed.add(instanceId);
    emitPluginHost(pluginId);
};

/** 调用最近一次 renderTile 留下的清理函数。 */
const releasePluginTile = (instanceId: string) => {
    const cleanup = tileCleanups.get(instanceId); // 上一轮作者返回的清理函数。
    tileCleanups.delete(instanceId);
    try {
        cleanup?.();
    } catch (error) {
        console.warn(`组件清理失败：${instanceId}`, error);
    }
};

/** 重读插件设置再交给格子。读失败时按空设置重绘。 */
const syncPluginTileSettings = async (pluginId: string, paint: (settings: unknown) => void) => {
    try {
        paint((await getPluginSettings(pluginId)) ?? null);
    } catch (error) {
        console.warn(`插件设置读取失败：${pluginId}`, error);
        paint(null);
    }
};

/** 先清理该实例上一轮内容，再把作者内容挂进容器。 */
const mountPluginTile = (container: HTMLElement, context: PluginShellContext) => {
    const renderTile = readPluginApi(context.pluginId)?.renderTile; // 多个实例共用这一份。
    if (typeof renderTile !== "function") return;
    releasePluginTile(context.instanceId);
    container.replaceChildren(); // 重挂前清掉上一轮的 DOM，否则作者用 append 就会叠出两份内容。
    const cleanup = renderTile(container, context); // 作者自行决定容器里的 DOM。
    if (typeof cleanup === "function") tileCleanups.set(context.instanceId, cleanup);
};

/** 有实例的插件按标识导入一次模块。 */
const ensurePluginScript = (pluginId: string, script: string) => {
    if (attempted.has(pluginId)) return Promise.resolve();
    attempted.add(pluginId);
    return loadPluginScript(pluginId, script).then((failure) => {
        if (failure) emitPluginHost(pluginId);
    });
};

/** 从清单里取出当前屏幕真正出现的插件，去重后按标识顺序加载。 */
export const syncPluginScripts = (manifests: readonly { id: string; script?: string }[], instanceComponentIds: readonly string[]) => {
    const live = new Set(instanceComponentIds); // 当前屏幕的实例所属标识。
    const pending = new Map<string, string>(); // 要加载的插件和它的脚本路径。
    for (const manifest of manifests) if (manifest.id.startsWith("plugin.") && live.has(manifest.id)) pending.set(manifest.id, manifest.script ?? "");
    return Promise.all([...pending.keys()].sort().map((pluginId) => ensurePluginScript(pluginId, pending.get(pluginId) ?? "")));
};

/** 把一个插件实例的作者内容挂到外框给出的容器上。 */
export const PluginTileBody = ({
    pluginId,
    instanceId,
    collapsed,
    revision,
}: {
    pluginId: string;
    instanceId: string;
    collapsed: boolean;
    revision: number;
}) => {
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
            void syncPluginTileSettings(pluginId, (settings) => paint(settings, mine));
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

/** 无格子外壳的 plugin:slot，岛与桌面复用同一加载、设置与失败隔离。 */
export const PluginSlot = ({ manifest, instanceId, collapsed }: { manifest: ComponentManifest; instanceId: string; collapsed: boolean }) => {
    const [, setEpoch] = useState(0); // 注册与失败变化触发重绘。
    useEffect(() => {
        const stop = subscribePluginHost((id) => {
            if (id === manifest.id) setEpoch((value) => value + 1);
        });
        adoptPluginManifests([manifest]);
        void syncPluginScripts([manifest], [manifest.id]);
        return stop;
    }, [manifest]);
    const broken = pluginScriptFailed(manifest.id) || pluginMissingRenderTile(manifest.id) || pluginRenderFailed(instanceId);
    return broken ? (
        <p role="status">组件渲染失败</p>
    ) : (
        <PluginTileBody pluginId={manifest.id} instanceId={instanceId} collapsed={collapsed} revision={pluginRevision(manifest.id)} />
    );
};
