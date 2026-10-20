import { useCallback, useEffect, useRef, useState } from "react";

import applicationIcon from "../../../resources/application.png";
import { getDesktopHostSnapshot } from "../../bridge/desktopHost";
import { getSettings, openLogs, savePreferences, setAutostart, settingsErrorMessage } from "../../bridge/settings";
import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../bridge/workspace";
import type { DesktopHostSnapshot, TakeoverPhase } from "../../contracts/desktopHost";
import type { ApplicationPreferences, SettingsSnapshot } from "../../contracts/settings";
import type { WorkspaceCommand, WorkspaceSnapshot } from "../../contracts/workspace";
import { usePreferences } from "./PreferencesProvider";
import { AppearanceSettings } from "./AppearanceSettings";
import { GridSettings } from "./GridSettings";
import { FeatureSettings } from "./FeatureSettings";
import { SettingsSwitch } from "./SettingsSwitch";
import { SettingsInfo } from "./SettingsInfo";
import { invokeRuntime } from "../../bridge/runtime";
import { MessageNotificationHost } from "../notifications/MessageNotificationHost";
import { publishMessage } from "../notifications/messageNotification";
import "./SettingsPage.css";

// 设置页使用 Windows 自带的 Fluent 图标字体，不新增图标依赖。
const PAGES = [
    { id: "general", label: "常规", icon: "\uE80F" },
    { id: "appearance", label: "外观", icon: "\uE790" },
    { id: "features", label: "功能格子", icon: "\uE80A" },
    { id: "performance", label: "性能", icon: "\uE9D9" },
    { id: "diagnostics", label: "诊断与维护", icon: "\uE90F" },
    { id: "about", label: "关于", icon: "\uE946" },
] as const;

// 宿主实际阶段对应的用户可见状态。
const PHASE_LABELS: Record<TakeoverPhase, string> = {
    starting: "启动中",
    ready: "已就绪",
    committing: "接管中",
    committed: "运行中",
    failed: "恢复处理中",
    restored: "已恢复系统桌面",
};

/** 设置窗口页面及真实配置操作，不装配桌面工作区交互。 */
export const SettingsPage = () => {
    const { preferences, failure: preferencesFailure, onPreferences, reducedMotion, animationsEnabled } = usePreferences(); // 当前窗口共用的偏好及系统动画限制。
    const [page, setPage] = useState<(typeof PAGES)[number]["id"]>("general"); // 当前设置导航页面。
    const [collapsed, setCollapsed] = useState(false); // 窄侧栏状态。
    const [gridDetail, setGridDetail] = useState<"desktopGrid" | "fenceGrid">(); // 外观下当前打开的网格二级菜单。
    const [snapshot, setSnapshot] = useState<SettingsSnapshot>(); // Windows 启动状态与构建版本。
    const [host, setHost] = useState<DesktopHostSnapshot>(); // 最近读取的实际宿主状态。
    const [workspace, setWorkspace] = useState<WorkspaceSnapshot>(); // 当前工作区映射快照。
    const [busy, setBusy] = useState(false); // 配置变更的控件禁用状态。
    const [directoryDraft, setDirectoryDraft] = useState<string>(); // 尚未保存的配置目录输入。
    const pending = useRef(false); // 阻止同一事件循环中的重复命令。
    const leaveGuard = useRef<((action: () => void) => void) | undefined>(undefined); // 子页未保存草稿的导航保护。
    const requestRevision = useRef(0); // 防止较早的加载结果覆盖更新后的设置。
    const title = PAGES.find((entry) => entry.id === page)!.label; // 当前页面可见标题。

    /** 使用 WebView2 原生视图过渡切换设置分类，关闭动画时直接更新。 */
    const handlePageChange = (next: (typeof PAGES)[number]["id"]) => {
        if (next === page && gridDetail === undefined) return;
        const update = () => {
            setPage(next);
            setGridDetail(undefined);
        };
        if (leaveGuard.current) {
            leaveGuard.current(update);
            return;
        }
        if (!animationsEnabled) {
            update();
            return;
        }
        document.startViewTransition(update);
    };

    /** 重新读取 Windows 实际启动状态，同时刷新诊断信息。 */
    const handleRefresh = useCallback(async () => {
        if (pending.current) return;
        const revision = ++requestRevision.current; // 本轮读取的有效序号。
        try {
            const next = await getSettings(); // 后端提供的实际配置状态。
            if (revision !== requestRevision.current) return;
            setSnapshot(next);
            onPreferences(next.preferences);
        } catch (error: unknown) {
            if (revision === requestRevision.current) publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings" });
        }
        try {
            const next = await getDesktopHostSnapshot(); // 后端当前桌面运行状态。
            if (revision !== requestRevision.current) return;
            setHost(next);
        } catch (error: unknown) {
            if (revision === requestRevision.current) publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-host" });
        }
    }, [onPreferences]);

    useEffect(() => {
        let active = true;
        const handleSnapshot = (next: WorkspaceSnapshot) => {
            if (active) setWorkspace(next);
        };
        const handleFailure = (error: unknown) => {
            if (active) publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-workspace" });
        };
        const subscription = listenWorkspaceSnapshots(handleSnapshot, handleFailure);
        void subscription.catch(handleFailure);
        void getWorkspaceSnapshot().then(handleSnapshot).catch(handleFailure);
        return () => {
            active = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    useEffect(() => {
        void handleRefresh();
        /** 托盘恢复设置窗口时重新读取系统配置。 */
        const handleFocus = () => {
            void handleRefresh();
        };
        window.addEventListener("focus", handleFocus);
        return () => {
            requestRevision.current++;
            window.removeEventListener("focus", handleFocus);
        };
    }, [handleRefresh]);

    useEffect(() => {
        if (preferencesFailure) publishMessage({ type: "error", content: preferencesFailure, source: "settings-preferences" });
    }, [preferencesFailure]);

    /** 串行执行设置命令，只在后端成功后更新控件。 */
    const handleCommand = async (command: () => Promise<void>) => {
        if (pending.current) return;
        pending.current = true;
        requestRevision.current++;
        setBusy(true);
        try {
            await command();
        } catch (error: unknown) {
            publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-command" });
        } finally {
            pending.current = false;
            setBusy(false);
        }
    };

    /** 合并一项用户意图，并接收原子保存后的完整偏好。 */
    const handlePreferences = (patch: Partial<ApplicationPreferences>) => {
        if (!preferences) return;
        void handleCommand(async () => {
            onPreferences(await savePreferences({ ...preferences, ...patch }));
        });
    };

    const handleWorkspaceCommand = async (command: WorkspaceCommand) => {
        const monitorId = host?.hosts[0]?.monitor.id;
        if (!monitorId) {
            publishMessage({ type: "error", content: "桌面宿主尚未就绪", source: "settings-workspace" });
            return;
        }
        try {
            await applyWorkspaceCommand(monitorId, command);
            publishMessage({
                type: "success",
                content:
                    command.type === "remove-folder-portal"
                        ? "文件夹映射已移除"
                        : command.type === "remap-folder-portal"
                          ? "文件夹映射路径已更新"
                          : "文件夹映射已创建",
                source: "settings-workspace",
            });
        } catch (error: unknown) {
            publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-workspace" });
        }
    };

    const handleCreateCollectBox = async () => {
        await handleCommand(async () => {
            await invokeRuntime("create_collect_box");
            publishMessage({ type: "success", content: "收集箱已创建", source: "settings-workspace" });
        });
    };

    const chooseFolderPortalDirectory = async () => {
        try {
            const selected = await invokeRuntime<unknown>("choose_folder_portal_directory");
            if (selected === null) return null;
            if (typeof selected !== "string" || !selected) throw new Error("选择目录返回数据无效");
            return selected;
        } catch (error: unknown) {
            publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-folder-chooser" });
            return null;
        }
    };

    return (
        <div className={`settings${collapsed ? " settings--collapsed" : ""}`}>
            <header
                className="settings__titlebar"
                onMouseDown={(event) => {
                    if (event.button === 0 && event.target instanceof Element && !event.target.closest("button"))
                        void invokeRuntime("control_settings_window", { action: event.detail === 2 ? "maximize" : "drag" }).catch((error) =>
                            publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-window" }),
                        );
                }}
            >
                <img src={applicationIcon} alt="" width="20" height="20" />
                <span>Desktop Tools 设置</span>
                <div className="settings__window-controls">
                    {(
                        [
                            ["minimize", "最小化", "\uE921"],
                            ["maximize", "最大化或还原", "\uE922"],
                            ["close", "关闭设置", "\uE8BB"],
                        ] as const
                    ).map(([action, label, icon]) => (
                        <button
                            key={action}
                            type="button"
                            title={label}
                            aria-label={label}
                            onClick={() =>
                                void invokeRuntime("control_settings_window", { action }).catch((error) =>
                                    publishMessage({ type: "error", content: settingsErrorMessage(error), source: "settings-window" }),
                                )
                            }
                        >
                            {icon}
                        </button>
                    ))}
                </div>
            </header>
            <aside className="settings__sidebar">
                <button
                    type="button"
                    className="settings__navigation-toggle"
                    title={collapsed ? "展开导航" : "收起导航"}
                    aria-label={collapsed ? "展开导航" : "收起导航"}
                    aria-expanded={!collapsed}
                    onClick={() => setCollapsed(!collapsed)}
                >
                    <span className="settings__icon" aria-hidden="true">
                        {"\uE700"}
                    </span>
                </button>
                <nav className="settings__navigation" aria-label="设置分类">
                    {PAGES.map((entry) => (
                        <button
                            key={entry.id}
                            type="button"
                            className="settings__navigation-item"
                            aria-current={page === entry.id ? "page" : undefined}
                            title={entry.label}
                            aria-label={entry.label}
                            onClick={() => handlePageChange(entry.id)}
                        >
                            <span className="settings__icon" aria-hidden="true">
                                {entry.icon}
                            </span>
                            <span className="settings__navigation-label">{entry.label}</span>
                        </button>
                    ))}
                </nav>
            </aside>
            <main className="settings__content" aria-labelledby="settings-title" aria-busy={busy}>
                {page === "appearance" && gridDetail && (
                    <button className="settings__back" type="button" title="返回外观" aria-label="返回外观" onClick={() => setGridDetail(undefined)}>
                        {"\uE72B"}
                    </button>
                )}
                <h1 id="settings-title">{page === "appearance" && gridDetail ? (gridDetail === "desktopGrid" ? "桌面网格" : "围栏内网格") : title}</h1>
                {!snapshot && (
                    <p className="settings__status" role="status">
                        正在读取设置…
                    </p>
                )}
                {page === "general" && (
                    <div className="settings__rows">
                        <form
                            className="settings__row settings__directory"
                            onSubmit={(event) => {
                                event.preventDefault();
                                if (!preferences || directoryDraft === undefined) return;
                                void handleCommand(async () => {
                                    onPreferences(await savePreferences({ ...preferences, configDirectory: directoryDraft.trim() }));
                                    setDirectoryDraft(undefined);
                                });
                            }}
                        >
                            <span className="settings__row-title" title={directoryDraft ?? preferences?.configDirectory}>
                                配置目录（下次启动生效）
                                <span className="settings__directory-path">
                                    {directoryDraft ?? preferences?.configDirectory ?? "%APPDATA%\\TIAmo\\DesktopTools"}
                                </span>
                            </span>
                            <button
                                type="button"
                                className="settings__button settings__icon-button"
                                title="选择配置目录"
                                aria-label="选择配置目录"
                                disabled={busy || !preferences}
                                onClick={() =>
                                    void handleCommand(async () => {
                                        const selected = await invokeRuntime<unknown>("choose_config_directory"); // 原生弹窗选中路径，取消返回 null。
                                        if (selected === null) return;
                                        if (typeof selected !== "string" || !selected) throw new Error("选择目录返回数据无效");
                                        setDirectoryDraft(selected);
                                    })
                                }
                            >
                                <span className="settings__directory-icon settings__directory-icon--folder" aria-hidden="true" />
                            </button>
                            <button
                                type="button"
                                className="settings__button settings__icon-button"
                                title="恢复默认目录"
                                aria-label="恢复默认目录"
                                disabled={busy || !preferences}
                                onClick={() => setDirectoryDraft("%APPDATA%\\TIAmo\\DesktopTools")}
                            >
                                <span className="settings__directory-icon settings__directory-icon--reset" aria-hidden="true" />
                            </button>
                            <button
                                type="submit"
                                className="settings__button"
                                disabled={busy || !preferences || directoryDraft === undefined || !directoryDraft.trim()}
                            >
                                保存
                            </button>
                        </form>
                        <div className="settings__row">
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE7E8"}
                            </span>
                            <label className="settings__row-title" htmlFor="autostart">
                                开机自动启动
                            </label>
                            <SettingsSwitch
                                id="autostart"
                                label="开机自动启动"
                                checked={snapshot?.autostart ?? false}
                                disabled={busy || !snapshot}
                                onChange={(enabled) => {
                                    void handleCommand(async () => {
                                        const actual = await setAutostart(enabled); // Windows 回读的真实开机启动状态。
                                        setSnapshot((current) => (current ? { ...current, autostart: actual } : current));
                                    });
                                }}
                            />
                        </div>
                        <div className="settings__row">
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE8B0"}
                            </span>
                            <label className="settings__row-title" htmlFor="double-click">
                                双击打开项目
                            </label>
                            <SettingsSwitch
                                id="double-click"
                                label="双击打开项目"
                                checked={preferences?.doubleClickToOpen ?? true}
                                disabled={busy || !preferences}
                                onChange={(checked) => handlePreferences({ doubleClickToOpen: checked })}
                            />
                        </div>
                    </div>
                )}
                {page === "appearance" && !gridDetail && (
                    <div className="settings__rows">
                        <AppearanceSettings preferences={preferences} disabled={busy} onPreferences={handlePreferences} />
                        {(
                            [
                                ["desktopGrid", "桌面网格"],
                                ["fenceGrid", "围栏内网格"],
                            ] as const
                        ).map(([kind, label]) => (
                            <button key={kind} className="settings__row settings__row--link" type="button" onClick={() => setGridDetail(kind)}>
                                <span className="settings__row-title">{label}</span>
                                <span className="settings__icon" aria-hidden="true">
                                    {"\uE76C"}
                                </span>
                            </button>
                        ))}
                    </div>
                )}
                {page === "appearance" && gridDetail && (
                    <GridSettings key={gridDetail} kind={gridDetail} preferences={preferences} disabled={busy} onPreferences={handlePreferences} />
                )}
                {page === "features" && (
                    <FeatureSettings
                        preferences={preferences}
                        disabled={busy}
                        debugBuild={snapshot?.debugBuild ?? false}
                        onPreferences={handlePreferences}
                        workspace={workspace}
                        onWorkspaceCommand={handleWorkspaceCommand}
                        onChooseFolder={chooseFolderPortalDirectory}
                        onCreateCollectBox={handleCreateCollectBox}
                        leaveGuard={leaveGuard}
                    />
                )}
                {page === "performance" && (
                    <div className="settings__rows">
                        {(
                            [
                                ["pauseOnFullscreen", "有应用全屏、最大化时暂停刷新", "仅暂停被前台窗口完全覆盖的显示器"],
                                ["pauseOnFocus", "有应用获取焦点时暂停刷新", "其他应用在前台时暂停全部桌面；可见桌面内容也会停止更新"],
                            ] as const
                        ).map(([key, label, description]) => (
                            <div className="settings__row" key={key}>
                                <span className="settings__row-title">
                                    <label htmlFor={key}>{label}</label>
                                    <SettingsInfo label={label} description={description} />
                                </span>
                                <SettingsSwitch
                                    id={key}
                                    label={label}
                                    checked={preferences?.[key] ?? false}
                                    disabled={busy || !preferences}
                                    onChange={(checked) => handlePreferences({ [key]: checked })}
                                />
                            </div>
                        ))}
                        <div className="settings__row">
                            <span className="settings__row-title">
                                <label htmlFor="follow-system-reduced-motion">跟随系统减少动态效果控制</label>
                                <SettingsInfo label="跟随系统减少动态效果控制" description="开启时由 Windows 动态效果设置控制实际动画，并保留界面动画开关值" />
                            </span>
                            <SettingsSwitch
                                id="follow-system-reduced-motion"
                                label="跟随系统减少动态效果控制"
                                checked={preferences?.followSystemReducedMotion ?? true}
                                disabled={busy || !preferences}
                                onChange={(checked) => handlePreferences({ followSystemReducedMotion: checked })}
                            />
                        </div>
                        <div className="settings__row">
                            <span className="settings__row-title">
                                <label htmlFor="interface-animations">界面动画</label>
                                <SettingsInfo label="界面动画" description="未跟随系统减少动态效果时，启用布局切换与状态过渡动画" />
                            </span>
                            <SettingsSwitch
                                id="interface-animations"
                                label="界面动画"
                                checked={preferences?.interfaceAnimations ?? true}
                                disabled={busy || !preferences || (preferences?.followSystemReducedMotion ?? true)}
                                onChange={(checked) => handlePreferences({ interfaceAnimations: checked })}
                            />
                        </div>
                        {(preferences?.followSystemReducedMotion ?? true) && reducedMotion && (
                            <p className="settings__status" role="status">
                                系统已启用减少动态效果，当前动画已关闭
                            </p>
                        )}
                    </div>
                )}
                {page === "diagnostics" && (
                    <div className="settings__rows">
                        <div className="settings__row">
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE9D9"}
                            </span>
                            <span className="settings__row-title">桌面工作区</span>
                            <span className="settings__value">{host ? PHASE_LABELS[host.phase] : "未获取"}</span>
                        </div>
                        <div className="settings__row">
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE7F4"}
                            </span>
                            <span className="settings__row-title">显示器</span>
                            <span className="settings__value">{host ? `${host.hosts.length} 台` : "未获取"}</span>
                        </div>
                        {host?.hosts.map((entry) => (
                            <div className="settings__row" key={entry.monitor.id}>
                                <span className="settings__icon" aria-hidden="true">
                                    {"\uE950"}
                                </span>
                                <span className="settings__row-title">{entry.monitor.name}</span>
                                <span className="settings__value">
                                    {entry.monitor.width} × {entry.monitor.height} · {Math.round(entry.monitor.scaleFactor * 100)}%
                                </span>
                            </div>
                        ))}
                        <div className="settings__row">
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE8B7"}
                            </span>
                            <span className="settings__row-title">应用日志</span>
                            <button type="button" className="settings__button" disabled={busy} onClick={() => void handleCommand(openLogs)}>
                                <span className="settings__icon" aria-hidden="true">
                                    {"\uE8DA"}
                                </span>
                                打开目录
                            </button>
                        </div>
                        <button type="button" className="settings__button settings__refresh" disabled={busy} onClick={() => void handleRefresh()}>
                            <span className="settings__icon" aria-hidden="true">
                                {"\uE72C"}
                            </span>
                            刷新状态
                        </button>
                    </div>
                )}
                {page === "about" && (
                    <div className="settings__about">
                        <img src={applicationIcon} alt="" width="64" height="64" />
                        <h2>Desktop Tools</h2>
                        <dl>
                            <dt>版本</dt>
                            <dd>{snapshot?.version ?? "未获取"}</dd>
                            <dt>平台</dt>
                            <dd>Windows</dd>
                        </dl>
                    </div>
                )}
            </main>
            <MessageNotificationHost />
        </div>
    );
};
