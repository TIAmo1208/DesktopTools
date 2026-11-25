import { useEffect, useMemo, useRef, useState } from "react";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import { pluginCapabilityContext } from "../../bridge/pluginCapabilityHost";
import { clearPluginMenuItems, setPluginMenuItems } from "../../bridge/pluginMenu";
import {
    adoptPluginManifests,
    loadPluginScript,
    pluginScriptFailure,
    readPluginApi,
    subscribePluginShell,
    type PluginShellContext,
} from "../../bridge/pluginScripts";
import { beginPluginMountSession, createPluginMountSessions, endPluginMountSession } from "./pluginMountSession.ts";
import type { ComponentCapability, ComponentManifest } from "../../contracts/component";
import { mountPluginHover } from "../../contracts/plugin";

const attempted = new Set<string>(); // 已尝试插入脚本的标识，失败也不重试。

const renderFailed = new Set<string>(); // renderTile 抛错的实例。
const tileCleanups = new Map<string, { session: number; cleanup: void | (() => void) }>(); // 每个实例最近一次 renderTile 的清理函数与会话。
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

/** 插件实例无法渲染：脚本失败、缺 renderTile 或本轮 renderTile 抛错。 */
export const pluginTileFailed = (pluginId: string, instanceId: string) => pluginScriptFailed(pluginId) || pluginMissingRenderTile(pluginId) || pluginRenderFailed(instanceId);

/** 读取实例 state 里保存的格子类型。state 不是对象或字段不是字符串时按未声明处理。 */
export const pluginTileType = (state: unknown) => {
    const value = typeof state === "object" && state !== null ? (state as Record<string, unknown>).tileType : undefined; // 类型写在 state 里，不占实例字段。
    return typeof value === "string" ? value : "";
};

/** 把命令式渲染失败收成格子既有错误态。 */
const markPluginRenderFailed = (pluginId: string, instanceId: string) => {
    if (renderFailed.has(instanceId)) return;
    renderFailed.add(instanceId);
    emitPluginHost(pluginId);
};

/** 调用最近一次 renderTile 留下的清理函数，并撤下该实例的插件菜单项。 */
const hoverCleanups = new Map<string, { session: number; cleanup: void | (() => void) }>(); // hover 只读视图的清理函数与所属会话，与 renderTile 分开。
const tileSessions = createPluginMountSessions(); // renderTile 会话；旧清理不能释放新实例。
const hoverSessions = createPluginMountSessions(); // hover 只读视图会话。

/** 释放指定会话的 hover 视图；旧会话的迟到清理不影响新会话。 */
const releasePluginHover = (instanceId: string, session: number) => {
    if (!endPluginMountSession(hoverSessions, instanceId, session)) return;
    const entry = hoverCleanups.get(instanceId);
    if (!entry || entry.session !== session) return;
    hoverCleanups.delete(instanceId);
    try {
        entry.cleanup?.();
    } catch (error) {
        console.warn(`组件 hover 视图清理失败：${instanceId}`, error);
    }
};

/** 同步释放该实例当前的 hover 视图；换挂载位前先结束旧会话。 */
const releasePluginHoverNow = (instanceId: string) => {
    const session = hoverSessions.current.get(instanceId);
    if (session !== undefined) releasePluginHover(instanceId, session);
};

/** 释放指定会话的 renderTile；旧会话的迟到清理不能释放新实例。 */
const releasePluginTileSession = (instanceId: string, session: number) => {
    if (!endPluginMountSession(tileSessions, instanceId, session)) return;
    const entry = tileCleanups.get(instanceId);
    if (entry?.session === session) {
        tileCleanups.delete(instanceId);
        try {
            entry.cleanup?.();
        } catch (error) {
            console.warn(`组件清理失败：${instanceId}`, error);
        }
    }
    clearPluginMenuItems(instanceId); // 菜单与清理函数同一生命周期；没有清理函数时也要撤下菜单。
};

/** 按实例释放当前 renderTile；供宿主与测试使用的稳定入口，内部换位仍用带会话版本。 */
export const releasePluginTile = (instanceId: string) => releasePluginTileNow(instanceId);

/** 同步释放该实例当前的 renderTile；先结束旧会话，再挂新会话。 */
const releasePluginTileNow = (instanceId: string) => {
    const session = tileSessions.current.get(instanceId);
    if (session !== undefined) releasePluginTileSession(instanceId, session);
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
const mountPluginTile = (container: HTMLElement, context: PluginShellContext, hover = false) => {
    const renderTile = hover ? readPluginApi(context.pluginId)?.renderIslandHover : readPluginApi(context.pluginId)?.renderTile; // 多个实例共用这一份。
    if (typeof renderTile !== "function") return undefined;
    if (hover) releasePluginHoverNow(context.instanceId); // 同一实例任一时刻只保留一个 hover 视图会话。
    else releasePluginTileNow(context.instanceId); // 先同步结束旧 renderTile，再创建新会话。
    container.replaceChildren(); // 重挂前清掉上一轮的 DOM，否则作者用 append 就会叠出两份内容。
    if (hover) {
        const session = beginPluginMountSession(hoverSessions, context.instanceId);
        const cleanup = mountPluginHover(renderTile, container, context, () => markPluginRenderFailed(context.pluginId, context.instanceId));
        hoverCleanups.set(context.instanceId, { session, cleanup });
        return session;
    }
    const session = beginPluginMountSession(tileSessions, context.instanceId);
    const cleanup = renderTile(container, {
        ...context,
        menu: {
            register: (items) => {
                setPluginMenuItems(context.instanceId, items); // 菜单项归实例，重复注册以最后一次为准。
                return () => clearPluginMenuItems(context.instanceId);
            },
        },
    }); // 作者自行决定容器里的 DOM。
    if (typeof cleanup === "function") tileCleanups.set(context.instanceId, { session, cleanup });
    return session;
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
    tileType,
    collapsed,
    revision,
    capabilities,
    onIslandSummary,
    hover = false,
}: {
    pluginId: string;
    instanceId: string;
    tileType: string;
    collapsed: boolean;
    revision: number;
    capabilities?: readonly ComponentCapability[]; // 实例已授权能力；通用能力只按它暴露。
    onIslandSummary?: (text: string | null, status?: string, navigation?: { uri: string }) => void; // 浮岛摘要/状态/导航；null 显式隐藏锚点，桌面格子不提供。
    hover?: boolean; // hover 与完整格子共用设置、加载和清理，不共用作者 DOM。
}) => {
    const containerRef = useRef<HTMLDivElement>(null); // 作者内容的挂载点。
    const sessionRef = useRef<number | undefined>(undefined); // 本组件最近一次创建的挂载会话。
    const collapsedRef = useRef(collapsed); // 读取返回时使用当前折叠状态。
    const settingsRef = useRef<{ ready: boolean; value: unknown }>({ ready: false, value: null }); // 已读过的设置，折叠时不再等命令。
    // 作者上下文里的通用能力按实例授权裁剪；未声明的能力保持 undefined。
    const capabilityContext = useMemo(() => pluginCapabilityContext(instanceId, capabilities ?? []), [instanceId, capabilities]);
    // 浮岛卡额外提供一行摘要入口；桌面格子保持 undefined。
    const islandContext = useMemo(() => (onIslandSummary ? { setIslandSummary: onIslandSummary } : {}), [onIslandSummary]);
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
                sessionRef.current = mountPluginTile(containerRef.current, {
                    pluginId,
                    instanceId,
                    tileType,
                    collapsed: collapsedRef.current,
                    settings,
                    saveSettings: (value) => savePluginSettings(pluginId, value),
                    ...capabilityContext,
                    ...islandContext,
                }, hover) ?? sessionRef.current;
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
            if (sessionRef.current !== undefined) {
                if (hover) releasePluginHover(instanceId, sessionRef.current);
                else releasePluginTileSession(instanceId, sessionRef.current); // 会话已被新视图接管时这里自动无操作。
            }
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [pluginId, instanceId, revision, hover, tileType, capabilityContext, islandContext]);

    useEffect(() => {
        const container = containerRef.current;
        if (!container || !settingsRef.current.ready) return;
        try {
            sessionRef.current = mountPluginTile(container, {
                pluginId,
                instanceId,
                tileType,
                collapsed,
                settings: settingsRef.current.value,
                saveSettings: (value) => savePluginSettings(pluginId, value),
                ...capabilityContext,
                ...islandContext,
            }, hover) ?? sessionRef.current;
        } catch (error) {
            console.error(`组件渲染失败：${instanceId}`, error);
            markPluginRenderFailed(pluginId, instanceId);
        }
    }, [collapsed, pluginId, instanceId, hover, capabilityContext, islandContext]);

    return <div ref={containerRef} data-plugin-container={instanceId} />;
};

/** 无格子外壳的 plugin:slot，岛与桌面复用同一加载、设置与失败隔离。 */
export const PluginSlot = ({
    manifest,
    instanceId,
    tileType = "",
    collapsed,
    capabilities,
    onIslandSummary,
    hover = false,
    fallback,
}: {
    manifest: ComponentManifest;
    instanceId: string;
    tileType?: string;
    collapsed: boolean;
    capabilities?: readonly ComponentCapability[];
    onIslandSummary?: (text: string | null, status?: string, navigation?: { uri: string }) => void;
    hover?: boolean;
    fallback?: import("react").ReactNode;
}) => {
    const [, setEpoch] = useState(0); // 注册与失败变化触发重绘。
    useEffect(() => {
        const stop = subscribePluginHost((id) => {
            if (id === manifest.id) setEpoch((value) => value + 1);
        });
        adoptPluginManifests([manifest]);
        void syncPluginScripts([manifest], [manifest.id]);
        return stop;
    }, [manifest]);
    const broken = hover ? pluginScriptFailed(manifest.id) || pluginRenderFailed(instanceId) : pluginTileFailed(manifest.id, instanceId);
    if (!broken && hover && !readPluginApi(manifest.id)?.renderIslandHover) return <>{fallback}</>;
    return broken ? (
        <p role="status">组件渲染失败</p>
    ) : (
        <PluginTileBody pluginId={manifest.id} instanceId={instanceId} tileType={tileType} collapsed={collapsed} capabilities={capabilities} onIslandSummary={onIslandSummary} hover={hover} revision={pluginRevision(manifest.id)} />
    );
};
