import { Component, useCallback, useEffect, useMemo, useRef, useState, type ErrorInfo, type PointerEvent, type ReactNode } from "react";

import { getComponentRuntimeSnapshot, listenComponentHostEvents, listenComponentRuntimeSnapshots } from "../../bridge/components";
import type { ComponentInstanceSnapshot, ComponentRegistrationSnapshot, ComponentRuntimeSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceRect, WorkspaceSnapshot } from "../../contracts/workspace";

import { usePreferences } from "../settings/PreferencesProvider";
import "./OfficialComponentHost.css";
import { MUSIC_CONTROL_MINIMUM_SIZE, type MusicControlMinimumSize } from "../music/MusicControl";
import { MusicWidget } from "../music/MusicWidget";
import { CollectBox, type CollectBoxToolbarActions } from "./CollectBox";
import { publishMessage } from "../notifications/messageNotification";
import { DesktopTileBase } from "../tiles/DesktopTileBase";
import { DesktopTileHeader } from "../tiles/DesktopTileHeader";
import { DesktopTileViewport } from "../tiles/DesktopTileViewport";
import { useDesktopTileGeometry } from "../tiles/useDesktopTileGeometry";
import { useDesktopTileExpansion } from "../tiles/useDesktopTileExpansion";
import type { TileAppearance, TileAppearanceChange } from "../../contracts/tile";
import type { WorkspaceFocusTarget } from "../../contracts/workspace";
import fenceToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import fenceToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import fenceSortIcon from "../../../resources/fence-sort.svg?raw";
import collectNewIcon from "../../../resources/collect-new.svg?raw";

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

/** 单个组件渲染失败时保持其他组件运行的错误边界。 */
class ComponentErrorBoundary extends Component<{ children: ReactNode; instanceId: string }, { failed: boolean }> {
    state = { failed: false }; // 当前实例是否已经触发 React 渲染错误。

    /** 将当前实例切换为隔离失败表面。 */
    static getDerivedStateFromError() {
        return { failed: true };
    }

    /** 记录组件渲染失败但不向工作区传播异常。 */
    componentDidCatch(error: Error, _info: ErrorInfo) {
        console.error(`组件渲染失败：${this.props.instanceId}`, error);
    }

    /** 呈现当前组件或隔离失败说明。 */
    render() {
        return this.state.failed ? <p className="official-component__error">组件渲染失败</p> : this.props.children;
    }
}

/** 呈现官方本地时间组件。 */
const LocalClock = ({
    instance,
    registration,
    visible,
    monitorId,
}: {
    instance: ComponentInstanceSnapshot;
    registration: ComponentRegistrationSnapshot;
    visible: boolean; // 实际展开状态包含临时悬停展开。
    monitorId: string; // 当前呈现宿主，不等同于离线显示器归属。
}) => {
    const [unixMillis, setUnixMillis] = useState<number>(); // 宿主授权能力或事件提供的当前时间。
    const [failure, setFailure] = useState<string>(); // 当前实例能力调用失败说明。
    const canReadTime = registration.authorizedCapabilities.includes("read-host-time"); // 当前清单实际获得的宿主时间能力。
    const { preferences } = usePreferences(); // 跨窗口同步的时间格式。
    const hour12 = preferences?.clockHour12 ?? false; // 用户设置的时间制式。
    const [paused, setPaused] = useState(() => window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false); // 当前页面实际暂停状态。

    useEffect(() => {
        const handleRefreshState = () => setPaused(window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false); // 恢复后重新建立活动订阅。
        window.addEventListener("desktop-refresh-changed", handleRefreshState);
        return () => window.removeEventListener("desktop-refresh-changed", handleRefreshState);
    }, []);
    useEffect(() => {
        if (!visible || paused || !canReadTime) return;
        let active = true; // 折叠、暂停及重新挂载后拒绝旧结果。
        let stop: (() => void) | undefined; // 当前时间订阅的释放入口。
        const handleFailure = (error: unknown) => {
            if (active) setFailure(error instanceof Error ? error.message : "组件时间读取失败");
        }; // 错误只影响当前时钟。
        void listenComponentHostEvents(
            instance.instanceId,
            monitorId,
            (event) => {
                if (active && event.kind === "time-tick") {
                    setUnixMillis(event.unixMillis);
                    setFailure(undefined);
                }
            },
            handleFailure,
        )
            .then((unlisten) => {
                if (!active) {
                    unlisten();
                    return;
                }
                stop = unlisten;
            })
            .catch(handleFailure);
        return () => {
            active = false;
            stop?.();
        };
    }, [instance.instanceId, monitorId, visible, paused, canReadTime]);

    const text = unixMillis
        ? new Intl.DateTimeFormat("zh-CN", {
              hour: "2-digit",
              minute: "2-digit",
              second: preferences?.clockShowSeconds === false ? undefined : "2-digit",
              hour12,
          }).format(new Date(unixMillis))
        : "--:--:--"; // 当前组件显示的宿主时间文本。

    return (
        <>
            <time className="official-component__clock" dateTime={unixMillis ? new Date(unixMillis).toISOString() : undefined}>
                {text}
            </time>
            {failure && <p className="official-component__error">{failure}</p>}
        </>
    );
};

/** 呈现一个可聚焦、可拖动并隔离加载失败的组件实例。 */
const ComponentSurface = ({
    appearance: savedAppearance,
    keyboardFocus,
    debugContent,
    instance,
    registration,
    currentMonitor,
    monitors,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
}: {
    appearance?: TileAppearance; // 当前实例共享外观。
    keyboardFocus?: WorkspaceFocusTarget; // 工作区语义焦点。
    debugContent?: ReactNode; // 已装配的调试内容。
    instance: ComponentInstanceSnapshot;
    registration?: ComponentRegistrationSnapshot;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    onCommand: OfficialComponentHostProps["onCommand"];
    onInteraction: OfficialComponentHostProps["onInteraction"];
    onBeginInputDrag: OfficialComponentHostProps["onBeginInputDrag"];
    onEndInputDrag: OfficialComponentHostProps["onEndInputDrag"];
}) => {
    const surfaceRef = useRef<HTMLElement>(null); // 当前组件实例表面。
    const [editingTitle, setEditingTitle] = useState(false); // 双击标题进入编辑，允许空字符串。
    const [collectPeek, setCollectPeek] = useState({ title: "收集箱", summary: "" }); // 折叠态显示的最新条目预览。
    const [collectToolbar, setCollectToolbar] = useState<CollectBoxToolbarActions>(); // 折叠标题轨道复用的收集箱命令。
    const collectToolbarReady = useCallback((actions: CollectBoxToolbarActions) => setCollectToolbar(actions), []); // 保持回调稳定，避免重复注册。
    const appearance: TileAppearance = savedAppearance ?? { title: null, titleMode: "fixed", buttonMode: "hover", collapsed: false, autoExpand: false }; // 未设置外观采用公共默认值。
    const expansion = useDesktopTileExpansion(surfaceRef, appearance.collapsed, appearance.autoExpand, currentMonitor?.id); // 临时展开同步标题和摘要状态。
    const title =
        instance.componentId === "official.collect-box" && expansion.collapsed
            ? collectPeek.title
            : (appearance.title ??
              (instance.componentId === "official.music-control"
                  ? "音乐控制"
                  : instance.componentId === "official.debug-test"
                    ? "Debug 测试"
                    : instance.componentId === "official.collect-box"
                      ? "收集箱"
                      : "本地时间")); // 空标题不回退。
    const collapsedSize = instance.componentId === "official.collect-box" ? { height: 56 } : undefined; // 收集箱使用两倍标题轨道，其余格子沿用基类默认值。
    const music = instance.componentId === "official.music-control"; // 音乐内容自适应外壳，同时限制最小可读尺寸。
    const [contentMinimum, setContentMinimum] = useState(MUSIC_CONTROL_MINIMUM_SIZE); // 组件上报的最终内容需求，不解释内部布局。
    const minimum = music
        ? { width: contentMinimum.width + 24, height: contentMinimum.height + 24 + (appearance.titleMode === "fixed" ? 28 : 0) }
        : instance.componentId === "official.debug-test"
          ? { width: 280, height: 154 }
          : instance.componentId === "official.collect-box"
            ? { width: 240, height: 120 }
            : { width: 164, height: 90 }; // 音乐包含内容、20px 留白和4px外壳占用。
    const bounds = music
        ? { ...instance.bounds, width: Math.max(instance.bounds.width, minimum.width), height: Math.max(instance.bounds.height, minimum.height) }
        : instance.bounds; // 旧小布局先按有效下限呈现，下次几何提交保存修正值。
    const { preferences } = usePreferences(); // 功能格子吸附偏好。
    const [failure, setFailure] = useState<string>(); // 保存或输入错误反馈。
    const handleFailure = (error: unknown) => {
        const message = error instanceof Error ? error.message : "格子操作失败";
        setFailure(message);
        publishMessage({ type: "error", content: message, source: "desktop-tile" });
    }; // 失败保持权威状态并通知。
    const handleAppearance = (change: TileAppearanceChange) => {
        void onCommand({ type: "set-tile-appearance", target: { type: "component", componentId: instance.instanceId }, change }).catch(handleFailure);
    }; // 外观不混入组件 state。
    const { handlePointerDown: handleGeometry, handleKeyDown: handleGeometryKey } = useDesktopTileGeometry({
        surfaceRef,
        bounds,
        collapsed: expansion.collapsed,
        collapsedSize,
        title,
        minimum,
        snap: preferences?.componentSnap !== false,
        currentMonitor,
        monitors,
        onFocus: () => {
            surfaceRef.current?.focus({ preventScroll: true });
            void onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(handleFailure);
        },
        onCommit: (bounds, monitorId) => onCommand({ type: "update-component-bounds", instanceId: instance.instanceId, bounds }, monitorId),
        onBeginInputDrag,
        onEndInputDrag,
        onFailure: handleFailure,
    }); // 与围栏共用整体拖动、缩放、吸附、跨屏和取消恢复。

    const loading = !registration; // Component Runtime 首次同步前的短暂加载状态。
    const failed = registration?.status === "failed"; // 当前实例是否由后端隔离为加载失败。
    return (
        <DesktopTileBase
            ref={surfaceRef}
            appearance={appearance}
            collapsed={expansion.collapsed}
            collapsedSize={collapsedSize}
            renaming={editingTitle}
            onRenameStart={() => setEditingTitle(true)}
            onAppearanceChange={handleAppearance}
            sortItems={instance.componentId === "official.collect-box" ? collectToolbar?.sortItems : undefined}
            menuItems={
                instance.componentId === "official.collect-box"
                    ? [
                          {
                              label: "新建收集箱格子",
                              onSelect: () =>
                                  void onCommand({
                                      type: "create-component",
                                      componentId: "official.collect-box",
                                      bounds: { left: bounds.left + 24, top: bounds.top + 24, width: bounds.width, height: bounds.height },
                                  }).catch(handleFailure),
                          },
                          {
                              label: "恢复最近删除",
                              disabled: !collectToolbar,
                              onSelect: () => collectToolbar?.restore(),
                          },
                      ]
                    : []
            }
            onPointerEnter={expansion.onPointerEnter}
            onPointerLeave={expansion.onPointerLeave}
            data-keyboard-focus={keyboardFocus?.type === "component" && keyboardFocus.componentId === instance.instanceId ? "true" : undefined}
            onResize={(event, direction) => void handleGeometry(event, direction)}
            className={`official-component${failed ? " official-component--failed" : ""}`}
            style={{ left: bounds.left, top: bounds.top, width: bounds.width, height: bounds.height }}
            data-component-id={instance.instanceId}
            data-hit-id={instance.instanceId}
            data-hit-kind="component"
            data-hit-priority="60"
            tabIndex={0}
            aria-label={loading ? "组件正在加载" : title}
            onPointerDown={(event) => event.stopPropagation()}
            onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
            }}
            onKeyDown={handleGeometryKey}
            onFocus={() => void onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(() => undefined)}
        >
            <DesktopTileHeader
                title={title}
                subtitle={instance.componentId === "official.collect-box" && expansion.collapsed ? collectPeek.summary : undefined}
                editing={editingTitle}
                onRenameStart={() => setEditingTitle(true)}
                onRenameEnd={() => setEditingTitle(false)}
                onPointerDown={(event) => void handleGeometry(event)}
                onRename={(nextTitle) =>
                    onCommand({
                        type: "set-tile-appearance",
                        target: { type: "component", componentId: instance.instanceId },
                        change: { type: "title", value: nextTitle },
                    }).catch((error) => {
                        handleFailure(error);
                        throw error;
                    })
                }
                commands={(sortMenuOpen) => (
                    <>
                        {instance.componentId === "official.collect-box" && collectToolbar && (
                            <>
                                <button
                                    type="button"
                                    title="排序方式"
                                    aria-label="排序方式"
                                    aria-haspopup="menu"
                                    aria-expanded={sortMenuOpen}
                                    onClick={(event) => {
                                        const rect = event.currentTarget.getBoundingClientRect(); // 与围栏共用菜单锚点与键盘焦点恢复。
                                        event.currentTarget.closest(".desktop-tile")?.dispatchEvent(
                                            new CustomEvent("desktop-tile-menu", {
                                                detail: { left: rect.left, top: rect.bottom, sorting: true, trigger: event.currentTarget },
                                            }),
                                        );
                                    }}
                                >
                                    <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fenceSortIcon }} />
                                </button>
                                <button type="button" title="新建收集项" aria-label="新建收集项" onClick={collectToolbar.create}>
                                    <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: collectNewIcon }} />
                                </button>
                            </>
                        )}
                        <button
                            type="button"
                            title={appearance.collapsed ? "展开格子" : "折叠格子"}
                            aria-label={appearance.collapsed ? "展开格子" : "折叠格子"}
                            onClick={() => handleAppearance({ type: "collapsed", value: !appearance.collapsed })}
                        >
                            <span
                                aria-hidden="true"
                                dangerouslySetInnerHTML={{ __html: appearance.collapsed ? fenceToggleExpandIcon : fenceToggleCollapseIcon }}
                            />
                        </button>
                    </>
                )}
            />
            <DesktopTileViewport
                scrollable={!music}
                className={`official-component__content${music ? " official-component__content--music" : ""}`}
                collapsed={expansion.collapsed}
            >
                <ComponentErrorBoundary instanceId={instance.instanceId}>
                    {loading ? (
                        <p className="official-component__status">正在加载组件...</p>
                    ) : failed ? (
                        <p className="official-component__error">{registration?.error ?? "组件清单不存在"}</p>
                    ) : registration.entry === "official:local-clock" ? (
                        <LocalClock
                            instance={instance}
                            registration={registration}
                            visible={!expansion.collapsed}
                            monitorId={currentMonitor?.id ?? instance.monitorId}
                        />
                    ) : registration.entry === "official:debug-test" ? (
                        debugContent
                    ) : registration.entry === "official:music-control" ? (
                        <MusicWidget onMinimumSizeChange={setContentMinimum} />
                    ) : registration.entry === "official:collect-box" ? (
                        <CollectBox onLatestChange={setCollectPeek} onToolbarReady={collectToolbarReady} />
                    ) : (
                        <p className="official-component__error">组件前端入口不存在</p>
                    )}
                </ComponentErrorBoundary>
                {failure && (
                    <p className="official-component__error" role="alert">
                        {failure}
                    </p>
                )}
            </DesktopTileViewport>
        </DesktopTileBase>
    );
};

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
            {visibleInstances.map((instance) => (
                <ComponentSurface
                    key={instance.instanceId}
                    appearance={tileAppearances?.[`component:${instance.instanceId}`]}
                    keyboardFocus={keyboardFocus}
                    debugContent={debugContent}
                    instance={instance}
                    registration={registrations.get(instance.instanceId)}
                    currentMonitor={currentMonitor}
                    monitors={monitors}
                    onCommand={onCommand}
                    onInteraction={onInteraction}
                    onBeginInputDrag={onBeginInputDrag}
                    onEndInputDrag={onEndInputDrag}
                />
            ))}
        </>
    );
};
