import { Component, useState, type ErrorInfo, type ReactNode } from "react";
import type { ComponentInstanceSnapshot, ComponentRegistrationSnapshot } from "../../contracts/component";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { TileAppearance, TileAppearanceChange } from "../../contracts/tile";
import type { WorkspaceCommand, WorkspaceInteractionCommand, WorkspaceFocusTarget, WorkspaceSnapshot } from "../../contracts/workspace";
import { usePreferences } from "../settings/PreferencesProvider";
import { publishMessage } from "../notifications/messageNotification";
import { DesktopTileBase } from "../tiles/DesktopTileBase";
import { DesktopTileHeader } from "../tiles/header/DesktopTileHeader";
import { DesktopTileViewport } from "../tiles/DesktopTileViewport";
import type { DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import fenceToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import fenceToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import "./ComponentTile.css";

/** 组件业务内容接收的实例、能力和工作区提交边界。 */
export interface ComponentTileProps {
    appearance?: TileAppearance; // 当前实例的共享外观。
    keyboardFocus?: WorkspaceFocusTarget; // 工作区语义焦点。
    instance: ComponentInstanceSnapshot; // 原有组件身份与布局。
    registration?: ComponentRegistrationSnapshot; // 运行时能力与加载结果。
    currentMonitor?: DesktopHostMonitor; // 实际呈现显示器。
    monitors: DesktopHostMonitor[]; // 跨屏目标集合。
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>; // 工作区业务命令。
    onInteraction: (command: WorkspaceInteractionCommand) => Promise<WorkspaceSnapshot>; // 实例语义焦点。
    onBeginInputDrag: () => Promise<void>; // 原生连续输入入口。
    onEndInputDrag: () => Promise<void>; // 原生连续输入释放。
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

/** 将组件身份、能力和失败隔离接入公共外框，不识别具体业务类型。 */
export const ComponentTile = ({
    appearance: savedAppearance,
    keyboardFocus,
    instance,
    registration,
    currentMonitor,
    monitors,
    onCommand,
    onInteraction,
    onBeginInputDrag,
    onEndInputDrag,
    title: defaultTitle,
    collapsedTitle,
    subtitle,
    minimum,
    collapsedSize,
    commands,
    menuItems,
    sortItems,
    scrollable = true,
    expandOnDrop = false,
    children,
}: ComponentTileProps & {
    title: string; // 功能提供的默认标题，空用户标题不会回退。
    collapsedTitle?: string; // 内容提供的折叠标题。
    subtitle?: string; // 内容计算的折叠摘要。
    minimum?: { width: number; height: number }; // 功能覆盖外框下限，缺省使用公共数据底线。
    collapsedSize?: { width?: number; height?: number }; // 与命中及预览一致的折叠尺寸。
    commands?: (sortMenuOpen: boolean) => ReactNode; // 内容工具保留公共 commands 插槽。
    menuItems?: DesktopTileMenuItem[]; // 实例业务菜单。
    sortItems?: DesktopTileMenuItem[]; // 公共排序菜单。
    scrollable?: boolean; // 固定内容使用填充布局。
    expandOnDrop?: boolean; // 内容模块声明匹配白名单后临时展开，不改变悬停偏好。
    children?: ReactNode | ((collapsed: boolean) => ReactNode); // 活动内容读取实际展开状态。
}) => {
    const [editingTitle, setEditingTitle] = useState(false); // 原有标题编辑状态。
    const [failure, setFailure] = useState<string>(); // 实例局部操作错误。
    const { preferences } = usePreferences(); // 现有功能格子吸附偏好。
    const appearance: TileAppearance = savedAppearance ?? { title: null, titleMode: "fixed", buttonMode: "hover", collapsed: false, autoExpand: false }; // 当前版本默认外观。
    const title = appearance.title ?? defaultTitle; // 允许用户设置空标题。
    const loading = !registration; // 运行时初始同步状态。
    const failed = registration?.status === "failed"; // 后端隔离的失败状态。
    const handleFailure = (error: unknown) => {
        const message = error instanceof Error ? error.message : "格子操作失败"; // 沿用原有错误呈现。
        setFailure(message);
        publishMessage({ type: "error", content: message, source: "desktop-tile" });
    }; // 保存失败保持原有状态并通知。
    const handleAppearance = (change: TileAppearanceChange) => {
        void onCommand({ type: "set-tile-appearance", target: { type: "component", componentId: instance.instanceId }, change }).catch(handleFailure);
    }; // 外观不进入组件 state 或布局历史。
    return (
        <DesktopTileBase
            appearance={appearance}
            collapsedSize={collapsedSize}
            renaming={editingTitle}
            onRenameStart={() => setEditingTitle(true)}
            onAppearanceChange={handleAppearance}
            menuItems={menuItems}
            sortItems={sortItems}
            geometry={{
                bounds: instance.bounds,
                title,
                minimum,
                snap: preferences?.componentSnap !== false,
                currentMonitor,
                monitors,
                onFocus: () =>
                    void onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(handleFailure),
                onCommit: (bounds, monitorId) => onCommand({ type: "update-component-bounds", instanceId: instance.instanceId, bounds }, monitorId),
                onBeginInputDrag,
                onEndInputDrag,
                onFailure: handleFailure,
            }}
            className={`official-component${failed ? " official-component--failed" : ""}`}
            data-component-id={instance.instanceId}
            data-drop-component-type={!loading && !failed ? instance.componentId : undefined}
            data-expand-on-drop={expandOnDrop ? "true" : undefined}
            data-hit-id={instance.instanceId}
            data-hit-kind="component"
            data-hit-priority="60"
            data-keyboard-focus={keyboardFocus?.type === "component" && keyboardFocus.componentId === instance.instanceId ? "true" : undefined}
            tabIndex={0}
            aria-label={loading ? "组件正在加载" : title}
            onPointerDown={(event) => event.stopPropagation()}
            onFocus={() => void onInteraction({ type: "focus-surface", focus: { type: "component", componentId: instance.instanceId } }).catch(() => undefined)}
        >
            {(collapsed) => (
                <>
                    <DesktopTileHeader
                        title={collapsed ? (collapsedTitle ?? title) : title}
                        subtitle={collapsed ? subtitle : undefined}
                        editing={editingTitle}
                        onRenameStart={() => setEditingTitle(true)}
                        onRenameEnd={() => setEditingTitle(false)}
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
                                {commands?.(sortMenuOpen)}
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
                        scrollable={scrollable}
                        className={`official-component__content${scrollable ? "" : " official-component__content--fill"}`}
                    >
                        <ComponentErrorBoundary instanceId={instance.instanceId}>
                            {loading ? (
                                <p className="official-component__status">正在加载组件...</p>
                            ) : failed ? (
                                <p className="official-component__error">{registration?.error ?? "组件清单不存在"}</p>
                            ) : typeof children === "function" ? (
                                children(collapsed)
                            ) : (
                                children
                            )}
                        </ComponentErrorBoundary>
                        {failure && (
                            <p className="official-component__error" role="alert">
                                {failure}
                            </p>
                        )}
                    </DesktopTileViewport>
                </>
            )}
        </DesktopTileBase>
    );
};
