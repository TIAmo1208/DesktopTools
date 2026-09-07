import { Component, useEffect, useMemo, useRef, useState, type ErrorInfo, type PointerEvent, type ReactNode } from "react";

import { getComponentRuntimeSnapshot, invokeComponentCapability, listenComponentHostEvents, listenComponentRuntimeSnapshots } from "../../bridge/components";
import type { ComponentHostEvent, ComponentInstanceSnapshot, ComponentRegistrationSnapshot, ComponentRuntimeSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceRect, WorkspaceSnapshot } from "../../contracts/workspace";
import { clientPointToPhysical, monitorAtPhysicalPoint, physicalPointToMonitorDip } from "../workspace/interaction/workspaceDrag";
import "./OfficialComponentHost.css";

/** 官方组件宿主接收的工作区实例和命令边界。 */
interface OfficialComponentHostProps {
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

/** 将权威组件边界约束到目标显示器工作区。 */
const constrainBounds = (bounds: WorkspaceRect, monitor: DesktopHostMonitor): WorkspaceRect => {
    const width = Math.min(bounds.width, monitor.workWidth / monitor.scaleFactor); // 目标显示器可容纳的组件宽度。
    const height = Math.min(bounds.height, monitor.workHeight / monitor.scaleFactor); // 目标显示器可容纳的组件高度。
    return {
        left: Math.max(0, Math.min(bounds.left, monitor.workWidth / monitor.scaleFactor - width)),
        top: Math.max(0, Math.min(bounds.top, monitor.workHeight / monitor.scaleFactor - height)),
        width,
        height,
    };
};

/** 呈现原型已验证视觉路径对应的官方本地时间组件。 */
const LocalClock = ({
    instance,
    registration,
    hostEvent,
}: {
    instance: ComponentInstanceSnapshot;
    registration: ComponentRegistrationSnapshot;
    hostEvent?: ComponentHostEvent;
}) => {
    const [unixMillis, setUnixMillis] = useState<number>(); // 宿主授权能力或事件提供的当前时间。
    const [failure, setFailure] = useState<string>(); // 当前实例能力调用失败说明。
    const canReadTime = registration.authorizedCapabilities.includes("read-host-time"); // 当前清单实际获得的宿主时间能力。
    const state = typeof instance.state === "object" && instance.state !== null ? (instance.state as Record<string, unknown>) : {}; // 当前版本化组件状态。
    const hour12 = state.format === "12-hour"; // 当前持久状态选择的时间格式。

    /** 通过 Component Runtime 能力边界刷新本地时间。 */
    const refresh = async () => {
        if (!canReadTime) {
            setFailure("组件未获得宿主时间能力");
            return;
        }
        try {
            const result = await invokeComponentCapability(instance.instanceId, "read-host-time");
            setUnixMillis(result.payload.unixMillis);
            setFailure(undefined);
        } catch (error: unknown) {
            setFailure(error instanceof Error ? error.message : "组件时间读取失败");
        }
    };

    useEffect(() => {
        void refresh();
    }, [instance.instanceId]);

    useEffect(() => {
        if (canReadTime && hostEvent?.kind === "time-tick") setUnixMillis(hostEvent.unixMillis);
    }, [canReadTime, hostEvent]);

    const text = unixMillis
        ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12 }).format(new Date(unixMillis))
        : "--:--:--"; // 当前组件显示的宿主时间文本。

    return (
        <>
            <time className="official-component__clock" dateTime={unixMillis ? new Date(unixMillis).toISOString() : undefined}>
                {text}
            </time>
            <button
                className="official-component__action"
                type="button"
                data-hit-id={`${instance.instanceId}:refresh`}
                data-hit-kind="component-control"
                data-hit-priority="110"
                onClick={() => void refresh()}
            >
                刷新
            </button>
            {failure && <p className="official-component__error">{failure}</p>}
        </>
    );
};

/** 呈现一个可聚焦、可拖动并隔离加载失败的组件实例。 */
const ComponentSurface = ({
    instance,
    registration,
    hostEvent,
    currentMonitor,
    monitors,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
}: {
    instance: ComponentInstanceSnapshot;
    registration?: ComponentRegistrationSnapshot;
    hostEvent?: ComponentHostEvent;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    onCommand: OfficialComponentHostProps["onCommand"];
    onInteraction: OfficialComponentHostProps["onInteraction"];
    onBeginInputDrag: OfficialComponentHostProps["onBeginInputDrag"];
    onEndInputDrag: OfficialComponentHostProps["onEndInputDrag"];
}) => {
    const surfaceRef = useRef<HTMLElement>(null); // 当前组件实例表面。
    const [previewBounds, setPreviewBounds] = useState<WorkspaceRect>(); // 连续拖动期间的临时边界。
    const bounds = previewBounds ?? instance.bounds; // 当前呈现使用的预览或权威边界。

    /** 从组件标题开始连续拖动并在释放时提交一次布局事务。 */
    const beginDrag = async (event: PointerEvent<HTMLElement>) => {
        if (event.button !== 0 || (event.target instanceof Element && event.target.closest("button"))) return;
        event.preventDefault();
        event.stopPropagation();
        await onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(() => undefined);
        try {
            await onBeginInputDrag();
        } catch {
            return;
        }
        const handle = event.currentTarget; // 指针捕获和原生监听所属的标题节点。
        const startPointer = { x: event.clientX, y: event.clientY }; // 拖动开始时的指针位置。
        const startBounds = { ...instance.bounds }; // 拖动开始时的权威组件边界。
        let preview = startBounds; // 指针移动期间持续更新的临时边界。
        let finished = false; // 当前连续拖动结束标记。
        try {
            handle.setPointerCapture(event.pointerId);
        } catch {
            await onEndInputDrag().catch(() => undefined);
            return;
        }

        // 使用当前指针位移更新组件预览位置。
        const handlePointerMove = (moveEvent: globalThis.PointerEvent) => {
            preview = {
                ...startBounds,
                left: startBounds.left + moveEvent.clientX - startPointer.x,
                top: startBounds.top + moveEvent.clientY - startPointer.y,
            };
            setPreviewBounds(preview);
        };
        // 提交释放位置或恢复权威边界。
        const finishDrag = (finishEvent: globalThis.PointerEvent) => {
            if (finished) return;
            finished = true;
            handle.removeEventListener("pointermove", handlePointerMove);
            handle.removeEventListener("pointerup", finishDrag);
            handle.removeEventListener("pointercancel", finishDrag);
            handle.removeEventListener("lostpointercapture", finishDrag);
            setPreviewBounds(undefined);
            if (finishEvent.type === "pointerup") {
                const centerPhysical = currentMonitor
                    ? clientPointToPhysical({ x: preview.left + preview.width / 2, y: preview.top + preview.height / 2 }, currentMonitor)
                    : undefined; // 组件中心在 Windows 虚拟屏幕中的物理坐标。
                const targetMonitor = centerPhysical ? monitorAtPhysicalPoint(centerPhysical, monitors) : currentMonitor; // 组件释放后的目标显示器。
                const targetCenter = centerPhysical && targetMonitor ? physicalPointToMonitorDip(centerPhysical, targetMonitor) : undefined; // 组件中心在目标显示器中的 DIP。
                const targetBounds = targetMonitor
                    ? constrainBounds(
                          {
                              ...preview,
                              left: (targetCenter?.left ?? preview.left + preview.width / 2) - preview.width / 2,
                              top: (targetCenter?.top ?? preview.top + preview.height / 2) - preview.height / 2,
                          },
                          targetMonitor,
                      )
                    : preview; // 保持逻辑尺寸并约束到目标工作区的最终边界。
                void onCommand({ type: "update-component-bounds", instanceId: instance.instanceId, bounds: targetBounds }, targetMonitor?.id).finally(() =>
                    onEndInputDrag().catch(() => undefined),
                );
            } else {
                void onEndInputDrag().catch(() => undefined);
            }
        };
        handle.addEventListener("pointermove", handlePointerMove);
        handle.addEventListener("pointerup", finishDrag);
        handle.addEventListener("pointercancel", finishDrag);
        handle.addEventListener("lostpointercapture", finishDrag);
    };

    const loading = !registration; // Component Runtime 首次同步前的短暂加载状态。
    const failed = registration?.status === "failed"; // 当前实例是否由后端隔离为加载失败。
    return (
        <section
            ref={surfaceRef}
            className={`official-component${failed ? " official-component--failed" : ""}`}
            style={{ left: bounds.left, top: bounds.top, width: bounds.width, height: bounds.height }}
            data-component-id={instance.instanceId}
            data-hit-id={instance.instanceId}
            data-hit-kind="component"
            data-hit-priority="60"
            tabIndex={0}
            aria-label={loading ? "组件正在加载" : registration?.componentId === "official.local-clock" ? "本地时间组件" : "组件加载失败"}
            onFocus={() => void onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(() => undefined)}
        >
            <header className="official-component__header" onPointerDown={(event) => void beginDrag(event)}>
                <strong>{registration?.componentId === "official.local-clock" ? "本地时间" : instance.componentId}</strong>
            </header>
            <ComponentErrorBoundary instanceId={instance.instanceId}>
                {loading ? (
                    <p className="official-component__status">正在加载组件...</p>
                ) : failed ? (
                    <p className="official-component__error">{registration?.error ?? "组件清单不存在"}</p>
                ) : registration.entry === "official:local-clock" ? (
                    <LocalClock instance={instance} registration={registration} hostEvent={hostEvent} />
                ) : (
                    <p className="official-component__error">组件前端入口不存在</p>
                )}
            </ComponentErrorBoundary>
        </section>
    );
};

/** 同步 Component Runtime 并呈现当前显示器的全部官方组件实例。 */
export const OfficialComponentHost = ({
    instances,
    currentMonitor,
    monitors,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
}: OfficialComponentHostProps) => {
    const [runtime, setRuntime] = useState<ComponentRuntimeSnapshot>(); // 当前后端组件注册表和实例状态。
    const [hostEvent, setHostEvent] = useState<ComponentHostEvent>(); // 最近一次通过宿主事件边界发布的时间事件。
    const [failure, setFailure] = useState<string>(); // 组件运行时 command 或 event 错误说明。
    const registrations = useMemo(() => new Map(runtime?.instances.map((instance) => [instance.instanceId, instance]) ?? []), [runtime]);

    useEffect(() => {
        let mounted = true; // 当前组件宿主挂载标记。
        const handleFailure = (error: unknown) => {
            if (mounted) setFailure(error instanceof Error ? error.message : "组件运行时请求失败");
        };
        const runtimeSubscription = listenComponentRuntimeSnapshots((snapshot) => mounted && setRuntime(snapshot), handleFailure);
        const eventSubscription = listenComponentHostEvents((event) => mounted && setHostEvent(event), handleFailure);
        void runtimeSubscription.catch(handleFailure);
        void eventSubscription.catch(handleFailure);
        return () => {
            mounted = false;
            void runtimeSubscription.then((unlisten) => unlisten()).catch(() => undefined);
            void eventSubscription.then((unlisten) => unlisten()).catch(() => undefined);
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
            {(failure || runtime?.failures.length) && (
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
            {instances.map((instance) => (
                <ComponentSurface
                    key={instance.instanceId}
                    instance={instance}
                    registration={registrations.get(instance.instanceId)}
                    hostEvent={hostEvent}
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
