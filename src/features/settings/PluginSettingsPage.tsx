import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import {
    applyPluginInstall,
    cancelPluginInstall,
    choosePluginDirectory,
    commandErrorMessage,
    createPluginComponent,
    listenPluginInstallProgress,
    openPluginDirectory,
    preparePluginInstallsFromArchive,
    preparePluginInstallsFromFolder,
    listenPluginScripts,
    loadPluginScript,
    pluginScriptLoaded,
    pluginScriptFailure,
    pluginSettingsShape,
    readPluginApi,
    subscribePluginShell,
    uninstallPlugin,
    type PluginSettingsEntry,
} from "../../bridge/pluginScripts";
import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../bridge/workspace";
import type { ComponentInstanceSnapshot, ComponentManifest } from "../../contracts/component";
import type { PluginInstallPlan } from "../../contracts/plugin";
import type { ApplicationPreferences } from "../../contracts/settings";
import { invokeRuntime } from "../../bridge/runtime";
import { publishMessage } from "../notifications/messageNotification";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import backIcon from "../../../resources/weather-back.svg?raw";
import chevronIcon from "../../../resources/weather-chevron-right.svg?raw";
import trashIcon from "../../../resources/weather-trash.svg?raw";
import plusIcon from "../../../resources/collect-new.svg?raw";
import linkIcon from "../../../resources/plugin-repository.svg?raw";
import { SettingsSwitch } from "./SettingsSwitch";
import { PluginTaskMenu } from "./PluginTaskMenu";
import "./PluginSettingsPage.css";

/** 设置页里的一个插件清单。 */
interface PluginRow {
    id: string; // 清单标识。
    name: string; // 清单显示名。
    creatable: boolean; // 清单是否允许创建格子。
    maxTiles: number; // 格子上限。0 表示不限。
    version?: string; // 清单版本。没有就不展示。
    description?: string; // 插件简介。没有就不展示。
    author?: string; // 作者。没有就不拼 @。
    repository?: string; // 有仓库才显示入口，不拿来拼地址。
    script: string; // 插件根内的脚本相对路径。
}

/** 设置页「插件」：一行一个插件，条目在本页展开。卸载确认、右键菜单和失效节不在这里。 */
export const PluginSettingsPage = ({
    preferences,
    disabled,
    onPreferences,
}: {
    preferences?: ApplicationPreferences; // 当前偏好；未就绪时不能改停用名单。
    disabled: boolean; // 保存进行中。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 写 disabledPlugins。
}) => {
    const [manifests, setManifests] = useState<ComponentManifest[]>(); // 未赋值表示清单还没到。
    const [components, setComponents] = useState<ComponentInstanceSnapshot[]>(); // 工作区实例；未到时不判断失效。
    const [removedIds, setRemovedIds] = useState<ReadonlySet<string>>(new Set()); // 本次已卸载、快照尚未刷新的插件。
    const [selectedId, setSelectedId] = useState<string>(); // 当前就地展开的插件；离开本页即丢掉。
    const [openEntryIds, setOpenEntryIds] = useState<ReadonlySet<string>>(new Set()); // 本插件内展开的条目，互不联动。
    const [, setShellEpoch] = useState(0); // 脚本注册或加载成功后重读条目。
    const [loadedIds, setLoadedIds] = useState<ReadonlySet<string>>(new Set()); // 设置窗口里已经执行成功的脚本。
    const [reloaded, setReloaded] = useState<Readonly<Record<string, string>>>({}); // 重新加载后的结果；ok 表示不再沿用旧失败。
    const [menu, setMenu] = useState<{ pluginId: string; left: number; top: number; anchor: HTMLElement }>(); // 右键菜单锚点。
    const [confirm, setConfirm] = useState<PluginConfirmRequest>(); // 当前模态确认。
    const [confirming, setConfirming] = useState(false); // 确认命令进行中。
    const [creating, setCreating] = useState(false); // 创建格子进行中。
    const [installing, setInstalling] = useState(false); // 正在选择安装来源。
    const [addOpen, setAddOpen] = useState(false); // 添加插件弹窗。
    const [picked, setPicked] = useState<PluginInstallPlan[]>([]); // 弹窗里已选、尚未开始的计划。
    const [taskMenu, setTaskMenu] = useState(false); // 任务进度菜单是否打开。
    const [taskMenuUp, setTaskMenuUp] = useState(false); // 下方空间不够时向上展开。
    const [tasks, setTasks] = useState<InstallTask[]>([]); // 本次打开设置页期间的安装任务，最多 20 条。
    const installRest = useRef<PluginInstallPlan[]>([]); // 冲突确认之后还要继续的计划。
    const [directoryDraft, setDirectoryDraft] = useState<string>(); // 尚未保存的插件目录。
    const plugins = manifests === undefined ? undefined : pluginRows(manifests).filter((plugin) => !removedIds.has(plugin.id)); // 仍显示的插件行。
    const selected = plugins?.find((plugin) => plugin.id === selectedId); // 仍在清单里的当前插件。
    const stale = manifests && components ? components.filter((item) => !manifests.some((manifest) => manifest.id === item.componentId)) : []; // 清单里已经没有的实例。
    const disabledIds = preferences?.disabledPlugins ?? []; // 停用名单。
    const preferencesRef = useRef(preferences); // 安装完成时读最新偏好，避免闭包里的旧名单。
    preferencesRef.current = preferences;

    useEffect(() => {
        let alive = true; // 卸载后忽略迟到快照。
        const subscription = listenComponentRuntimeSnapshots(
            (snapshot) => alive && setManifests(snapshot.manifests),
            () => undefined,
        );
        void getComponentRuntimeSnapshot()
            .then((snapshot) => alive && setManifests(snapshot.manifests))
            .catch(() => undefined);
        const stopShell = subscribePluginShell(() => setShellEpoch((value) => value + 1));
        const stopScripts = listenPluginScripts((pluginId) => {
            setLoadedIds((current) => new Set(current).add(pluginId));
            setShellEpoch((value) => value + 1);
        });
        return () => {
            alive = false;
            stopShell();
            stopScripts();
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    useEffect(() => {
        if (!plugins || !preferences) return;
        for (const plugin of plugins) if (!preferences.disabledPlugins.includes(plugin.id)) void loadPluginScript(plugin.id, plugin.script);
    }, [plugins, preferences]);

    useEffect(() => {
        let alive = true; // 卸载后忽略迟到的工作区快照。
        const apply = (snapshot: { components: ComponentInstanceSnapshot[] }) => alive && setComponents(snapshot.components);
        const subscription = listenWorkspaceSnapshots(apply, () => undefined);
        void getWorkspaceSnapshot()
            .then(apply)
            .catch(() => undefined);
        return () => {
            alive = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    useEffect(() => {
        setOpenEntryIds(new Set());
    }, [selectedId]);

    useEffect(() => {
        if (!taskMenu) return;
        const menu = document.querySelector(".plugin-settings__task-menu"); // 刚打开的任务菜单。
        const frame = menu?.closest(".settings__content"); // 内容区右内边距就是菜单的右边界。
        if (menu instanceof HTMLElement && frame instanceof HTMLElement) {
            const menuBox = menu.getBoundingClientRect();
            const frameBox = frame.getBoundingClientRect();
            const place = taskMenuPlacement(
                { top: menuBox.top, bottom: menuBox.bottom, height: menu.scrollHeight },
                { top: frameBox.top, bottom: frameBox.bottom },
            );
            setTaskMenuUp(place.flipUp);
        }
        const close = (event: PointerEvent) => {
            const target = event.target; // 点在菜单或下拉按钮上时不关。
            if (!(target instanceof Element)) return;
            if (target.closest(".plugin-settings__task-menu") || target.closest(".plugin-settings__add-more")) return;
            setTaskMenu(false);
        };
        const onKey = (event: globalThis.KeyboardEvent) => {
            if (event.key === "Escape") setTaskMenu(false);
        };
        document.addEventListener("pointerdown", close);
        document.addEventListener("keydown", onKey);
        return () => {
            document.removeEventListener("pointerdown", close);
            document.removeEventListener("keydown", onKey);
        };
    }, [taskMenu]);

    /** 写入停用名单。启用后的加载由上面的效果在偏好落地后再发起。 */
    const handleEnabled = (pluginId: string, enabled: boolean) => {
        if (!preferences) return;
        const next = enabled ? preferences.disabledPlugins.filter((id) => id !== pluginId) : [...preferences.disabledPlugins, pluginId];
        onPreferences({ disabledPlugins: next });
        if (!enabled) setOpenEntryIds(new Set());
    };

    useEffect(() => {
        let alive = true; // 离开页面后不再写入进度。
        let stop = () => {}; // 退订安装进度。
        void listenPluginInstallProgress((progress) => {
            if (!alive) return;
            setTasks((current) => {
                const existing = current.find((task) => task.id === progress.taskId); // 已结束的行不再被迟到进度改回进行中。
                if (existing && existing.status !== "running") return current;
                const status = progress.stage === "done" ? "done" : progress.stage === "failed" ? "failed" : "running";
                return rememberInstallTask(current, {
                    id: progress.taskId,
                    name: existing?.name || progress.sourceName,
                    status,
                    stage: progress.stage,
                    percent: progress.percent,
                });
            });
        })
            .then((unlisten) => {
                if (!alive) unlisten();
                else stop = unlisten;
            })
            .catch(() => undefined);
        return () => {
            alive = false;
            stop();
        };
    }, []);

    /** 执行计划并写入任务行。用户已取消的行不被失败覆盖。 */
    const runInstall = (plan: PluginInstallPlan, confirmDowngrade: boolean) => {
        setTasks((current) => rememberInstallTask(current, { id: plan.planId, name: plan.name, status: "running", stage: "validate", percent: 0 }));
        void applyPluginInstall(plan.planId, confirmDowngrade)
            .then(async (applied) => {
                setTasks((current) => rememberInstallTask(current, { id: plan.planId, name: plan.name, status: "done", stage: "done", percent: 100 }));
                publishMessage({ type: "success", content: installNotice(plan.name, installSuccessMessage()), source: "plugin-settings" });
                const installedId = applied.pluginId; // 重新装回同一个标识时，之前记下的「已卸载」必须撤销，否则该行会被一直过滤掉。
                const currentPreferences = preferencesRef.current; // 装完默认停用，等用户点启动才加载脚本。
                if (installedId && currentPreferences && !currentPreferences.disabledPlugins.includes(installedId)) {
                    onPreferences({ disabledPlugins: [...currentPreferences.disabledPlugins, installedId] });
                }
                if (installedId) {
                    setRemovedIds((current) => {
                        if (!current.has(installedId)) return current;
                        const next = new Set(current);
                        next.delete(installedId);
                        return next;
                    });
                }
                try {
                    const snapshot = applied.snapshot ?? (await getComponentRuntimeSnapshot()); // 事件到不了设置窗口时也要刷新列表。
                    setManifests(snapshot.manifests);
                } catch {
                    // 读不到快照时保留成功提示，列表仍可由订阅补上。
                }
            })
            .catch((error: unknown) => {
                const message = commandErrorMessage(error); // 后端 Err 原样显示。
                setTasks((current) => {
                    const existing = current.find((task) => task.id === plan.planId); // 用户已经取消时保留取消结果。
                    if (existing?.status === "cancelled") return current;
                    return rememberInstallTask(current, {
                        id: plan.planId,
                        name: plan.name,
                        status: message === "安装已取消" ? "cancelled" : "failed",
                        stage: message === "安装已取消" ? "cancelled" : "failed",
                        percent: existing?.percent ?? 0,
                        detail: message === "安装已取消" ? message : installFailureMessage(message),
                    });
                });
                if (message !== "安装已取消")
                    publishMessage({ type: "error", content: installNotice(plan.name, installFailureMessage(message)), source: "plugin-settings" });
            });
    };

    /** 把已选计划送进任务。有冲突的先确认，取消则跳过这一条。 */
    const enqueueInstalls = (plans: PluginInstallPlan[]) => {
        const pending = plans.length ? plans : installRest.current.splice(0); // 确认结束后继续剩下的。
        const [next, ...rest] = pending;
        if (!next) return;
        if (next.conflict) {
            installRest.current = rest;
            setConfirm({ kind: "install", plan: next, returnFocus: document.activeElement instanceof HTMLElement ? document.activeElement : document.body });
            return;
        }
        runInstall(next, false);
        enqueueInstalls(rest);
    };

    /** 弹窗里再选一个来源。当前命令一次一个，多个来源就多次选择。 */
    const pickInstallSource = (source: "folder" | "archive") => {
        if (installing) return;
        setInstalling(true);
        const prepare = source === "folder" ? preparePluginInstallsFromFolder : preparePluginInstallsFromArchive;
        void prepare()
            .then((batch) => {
                if (batch.plans.length === 0 && batch.errors.length === 0) {
                    publishMessage({ type: "info", content: "已取消", source: "plugin-settings" });
                    return;
                }
                for (const message of batch.errors) publishMessage({ type: "error", content: installFailureMessage(message), source: "plugin-settings" });
                if (batch.plans.length) setPicked((current) => [...current, ...batch.plans]);
            })
            .catch((error: unknown) => publishMessage({ type: "error", content: installFailureMessage(commandErrorMessage(error)), source: "plugin-settings" }))
            .finally(() => setInstalling(false));
    };

    /** 选择目录只改待保存值。取消不改。 */
    const handleChooseDirectory = () => {
        void choosePluginDirectory()
            .then((path) => {
                if (path === undefined) return;
                setDirectoryDraft(path);
            })
            .catch((error: unknown) => publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }));
    };

    /** 在桌面创建一个该插件的格子。失败把错误说出来。 */
    const handleCreate = (pluginId: string) => {
        if (creating) return;
        setCreating(true);
        void createPluginComponent(pluginId)
            .then(async () => {
                publishMessage({ type: "success", content: "已在桌面创建插件格子", source: "plugin-settings" });
                const snapshot = await getWorkspaceSnapshot().catch(() => undefined); // 设置窗口收不到工作区事件时也要更新剩余额度。
                if (snapshot) setComponents(snapshot.components);
            })
            .catch((error: unknown) => publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }))
            .finally(() => setCreating(false));
    };

    /** 打开清单声明的仓库。只走专用命令，不拼地址。 */
    const handleOpenRepository = (pluginId: string) => {
        void invokeRuntime("open_plugin_repository", { pluginId }).catch((error: unknown) =>
            publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }),
        );
    };

    /** 打开卸载确认，焦点稍后落到「卸载」。 */
    const handleUninstallRequest = (plugin: PluginRow, trigger: HTMLElement) => {
        setMenu(undefined);
        setConfirm({ kind: "uninstall", plugin, returnFocus: trigger });
    };

    /** 打开失效实例的移除确认，焦点稍后落到「移除」。 */
    const handleRemoveRequest = (instance: ComponentInstanceSnapshot, trigger: HTMLElement) => {
        setConfirm({ kind: "remove", instance, returnFocus: trigger });
    };

    /** 执行当前确认。卸载时 removeSettings 决定是否同时清掉插件配置。失败留在弹窗里。 */
    const handleConfirm = async (removeSettings = false) => {
        if (!confirm || confirming) return;
        setConfirming(true);
        try {
            if (confirm.kind === "uninstall") {
                await uninstallPlugin(confirm.plugin.id, removeSettings);
                setRemovedIds((current) => new Set(current).add(confirm.plugin.id));
                if (selectedId === confirm.plugin.id) setSelectedId(undefined);
                publishMessage({
                    type: "success",
                    content: removeSettings ? "已卸载并清理配置，桌面上的格子已移除，重装后设置不再恢复" : "已卸载，桌面上的格子已移除",
                    source: "plugin-settings",
                });
            } else if (confirm.kind === "install") {
                const plan = confirm.plan; // 确认后才执行，取消不改磁盘。
                setConfirm(undefined);
                runInstall(plan, Boolean(plan.conflict?.downgrade));
                enqueueInstalls([]);
                return;
            } else {
                await applyWorkspaceCommand(confirm.instance.monitorId, { type: "remove-component", instanceId: confirm.instance.instanceId });
            }
            setConfirm(undefined);
        } catch (error) {
            publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" });
        } finally {
            setConfirming(false);
        }
    };

    const menuPlugin = plugins?.find((plugin) => plugin.id === menu?.pluginId); // 右键时仍在列表里的插件。
    const menuUsed = menuPlugin ? pluginTileCount(components, menuPlugin.id) : 0; // 这个插件已经在桌面上的格子。
    const menuItems: DesktopTileMenuItem[] = menuPlugin
        ? pluginContextMenu(menuPlugin, {
              used: menuUsed,
              pluginDisabled: disabledIds.includes(menuPlugin.id),
              onOpenSettings: () => setSelectedId(menuPlugin.id),
              onOpenDirectory: () =>
                  void openPluginDirectory(menuPlugin.id).catch((error: unknown) =>
                      publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }),
                  ),
              onCreate: () => handleCreate(menuPlugin.id),
              busy: creating,
              onToggle: () => handleEnabled(menuPlugin.id, disabledIds.includes(menuPlugin.id)),
              onUninstall: () => menu && handleUninstallRequest(menuPlugin, menu.anchor),
          })
        : [];
    const overlays = (
        <>
            {menu && menuItems.length > 0 && <DesktopTileMenu items={menuItems} left={menu.left} top={menu.top} onClose={() => setMenu(undefined)} />}
            {addOpen && (
                <AddPluginDialog
                    picked={picked}
                    busy={installing}
                    onPick={pickInstallSource}
                    onCancel={() => {
                        setAddOpen(false);
                        setPicked([]);
                    }}
                    onStart={() => {
                        const plans = picked; // 一次把已选来源都送进任务菜单。
                        setPicked([]);
                        setAddOpen(false);
                        enqueueInstalls(plans);
                    }}
                />
            )}
            {confirm && (
                <PluginConfirmDialog
                    request={confirm}
                    pending={confirming}
                    onCancel={() => {
                        if (confirming || !confirm) return;
                        const kind = confirm.kind; // 取消冲突安装时跳过这一条，继续后面的。
                        setConfirm(undefined);
                        if (kind === "install") enqueueInstalls([]);
                    }}
                    onConfirm={() => void handleConfirm(false)}
                    onPurge={() => void handleConfirm(true)}
                />
            )}
        </>
    );

    if (selected) {
        const pluginDisabled = disabledIds.includes(selected.id); // 当前插件是否停用。
        const entries = entriesFor(selected.id); // 已注册的设置条目；脚本未到时为 undefined。
        const shape = pluginSettingsShape(selected.id); // 导出了条目但不是数组。
        const failure = scriptFailure(selected.id, reloaded); // 脚本失败说明；重新加载成功后不再沿用旧失败。
        const pending = !pluginDisabled && entries === undefined && !failure && !pluginScriptLoaded(selected.id); // 只有还没加载完才显示加载态。
        const usedTiles = pluginTileCount(components, selected.id); // 已在桌面上的格子数。
        const tileRoom = pluginTileRoom(selected.maxTiles, usedTiles); // 上限满了就不能再加。
        return (
            <>
                <section className="plugin-settings plugin-settings--detail">
                    <div className="plugin-settings__detail-head">
                        <button className="plugin-settings__back" type="button" aria-label="返回插件" onClick={() => setSelectedId(undefined)}>
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: backIcon }} />
                        </button>
                        <div className="plugin-settings__identity">
                            <h2>{selected.name}</h2>
                        </div>
                        <div className="plugin-settings__commands">
                            <button
                                className="settings__button settings__icon-button plugin-settings__trash"
                                type="button"
                                title="卸载"
                                aria-label="卸载"
                                onClick={(event) => handleUninstallRequest(selected, event.currentTarget)}
                            >
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: removeIcon }} />
                            </button>
                            <button
                                className="settings__button settings__icon-button"
                                type="button"
                                disabled={!showCreatePlugin(selected.creatable) || createPluginBlocked(pluginDisabled) || creating || tileRoom.full}
                                title={
                                    !showCreatePlugin(selected.creatable)
                                        ? "添加到桌面"
                                        : createPluginBlocked(pluginDisabled)
                                          ? "启用后可创建"
                                          : addToDesktopHint(selected.maxTiles, usedTiles)
                                }
                                aria-label="添加到桌面"
                                onClick={() => handleCreate(selected.id)}
                            >
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: addIcon }} />
                            </button>
                            <SettingsSwitch
                                label={pluginPowerLabel(pluginDisabled)}
                                checked={!pluginDisabled}
                                disabled={disabled || !preferences}
                                onChange={(checked) => handleEnabled(selected.id, checked)}
                            />
                        </div>
                    </div>
                    <p className="plugin-settings__meta">
                        <span>{pluginIdentity(selected.author, selected.id)}</span>
                        {selected.repository ? (
                            <button
                                className="plugin-settings__repo"
                                type="button"
                                title="打开仓库"
                                aria-label="打开仓库"
                                onClick={() => handleOpenRepository(selected.id)}
                            >
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: repoIcon }} />
                            </button>
                        ) : null}
                    </p>
                    {selected.description ? <PluginAbout text={selected.description} /> : null}
                    {pending ? (
                        <p className="plugin-settings__status" role="status" aria-busy="true">
                            正在加载插件设置…
                        </p>
                    ) : failure ? (
                        <div className="plugin-settings__error" role="alert">
                            <p>{failure}</p>
                            <button type="button" onClick={() => handleReload(selected.id, selected.script, setReloaded, setLoadedIds, setShellEpoch)}>
                                重新加载
                            </button>
                        </div>
                    ) : pluginDisabled && !entries?.length ? (
                        <p className="plugin-settings__status">插件已停用，启用后显示设置条目。</p>
                    ) : entries && entries.length > 0 ? (
                        <div className="plugin-settings__entries">
                            <h3>设置条目</h3>
                            {entries.map((entry) => (
                                <PluginEntry
                                    key={entry.id}
                                    pluginId={selected.id}
                                    entry={entry}
                                    open={openEntryIds.has(entry.id)}
                                    locked={pluginDisabled}
                                    onToggle={() =>
                                        setOpenEntryIds((current) => {
                                            const next = new Set(current);
                                            if (next.has(entry.id)) next.delete(entry.id);
                                            else next.add(entry.id);
                                            return next;
                                        })
                                    }
                                />
                            ))}
                        </div>
                    ) : shape ? (
                        <p className="plugin-settings__empty">{shape}</p>
                    ) : (
                        <p className="plugin-settings__empty">插件未提供设置条目</p>
                    )}
                </section>
                {overlays}
            </>
        );
    }

    return (
        <>
            <section className="plugin-settings">
                <div className="plugin-settings__add">
                    <button className="plugin-settings__add-main" type="button" disabled={disabled} onClick={() => setAddOpen(true)}>
                        ＋ 添加插件
                    </button>
                    <button
                        className="plugin-settings__add-more"
                        type="button"
                        aria-label="任务进度"
                        aria-expanded={taskMenu}
                        onClick={() => setTaskMenu((open) => !open)}
                    >
                        <span className="plugin-settings__add-chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                    </button>
                    <PluginTaskMenu
                        tasks={tasks}
                        open={taskMenu}
                        flipped={taskMenuUp}
                        onCollapse={() => setTaskMenu(false)}
                        onCancel={(taskId) => {
                            const task = tasks.find((item) => item.id === taskId); // 只取消菜单里仍在的那一条。
                            if (!task) return;
                            setTasks((current) => rememberInstallTask(current, { ...task, status: "cancelled", stage: "cancelled" }));
                            void cancelPluginInstall(taskId).catch((error: unknown) =>
                                publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }),
                            );
                        }}
                    />
                </div>
                <div className="plugin-settings__body">
                    <form
                        className="settings__row settings__directory"
                        onSubmit={(event) => {
                            event.preventDefault();
                            if (!pluginDirectoryDirty(directoryDraft, preferences?.pluginDirectory)) return;
                            onPreferences({ pluginDirectory: directoryDraft ?? "" });
                            setDirectoryDraft(undefined);
                        }}
                    >
                        <span className="settings__row-title" title={pluginDirectoryShown(directoryDraft, preferences?.pluginDirectory)}>
                            插件目录（下次启动生效）
                            <span className="settings__directory-path">{pluginDirectoryShown(directoryDraft, preferences?.pluginDirectory)}</span>
                        </span>
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            title="选择插件目录"
                            aria-label="选择插件目录"
                            disabled={disabled || !preferences}
                            onClick={handleChooseDirectory}
                        >
                            <span className="settings__directory-icon settings__directory-icon--folder" aria-hidden="true" />
                        </button>
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            title="恢复默认目录"
                            aria-label="恢复默认目录"
                            disabled={disabled || !preferences}
                            onClick={() => setDirectoryDraft("")}
                        >
                            <span className="settings__directory-icon settings__directory-icon--reset" aria-hidden="true" />
                        </button>
                        <button
                            className="settings__button"
                            type="submit"
                            disabled={disabled || !preferences || !pluginDirectoryDirty(directoryDraft, preferences?.pluginDirectory)}
                        >
                            保存
                        </button>
                    </form>
                    <div className="plugin-settings__list">
                        <h2 className="plugin-settings__label">已安装插件</h2>
                        {plugins === undefined ? null : plugins.length === 0 ? (
                            <p className="plugin-settings__empty">
                                尚未安装插件
                                <small>将插件放入插件目录后重新打开程序。</small>
                            </p>
                        ) : (
                            plugins.map((plugin) => {
                                const pluginDisabled = disabledIds.includes(plugin.id); // 本行是否停用。
                                return (
                                    <div
                                        className={`plugin-settings__row plugin-settings__row--link${pluginDisabled ? " plugin-settings__row--disabled" : ""}${menu?.pluginId === plugin.id ? " plugin-settings__row--selected" : ""}`}
                                        key={plugin.id}
                                        onContextMenu={(event) => {
                                            event.preventDefault();
                                            setMenu({ pluginId: plugin.id, left: event.clientX, top: event.clientY, anchor: event.currentTarget });
                                        }}
                                    >
                                        <button
                                            className="plugin-settings__open"
                                            type="button"
                                            aria-label={`${plugin.name}设置`}
                                            onClick={() => setSelectedId(plugin.id)}
                                        >
                                            <span className="plugin-settings__copy">
                                                {plugin.name}
                                                <small>
                                                    {entrySummary(
                                                        plugin.id,
                                                        pluginDisabled,
                                                        loadedIds.has(plugin.id) || pluginScriptLoaded(plugin.id) || reloaded[plugin.id] === "ok",
                                                        reloaded[plugin.id] === "ok",
                                                    )}
                                                </small>
                                            </span>
                                        </button>
                                        <button
                                            className="plugin-settings__uninstall"
                                            type="button"
                                            aria-label={`卸载${plugin.name}`}
                                            title="卸载"
                                            onClick={(event) => handleUninstallRequest(plugin, event.currentTarget)}
                                        >
                                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: trashIcon }} />
                                        </button>
                                        <SettingsSwitch
                                            label={`启用${plugin.name}`}
                                            checked={!pluginDisabled}
                                            disabled={disabled || !preferences}
                                            onChange={(checked) => handleEnabled(plugin.id, checked)}
                                        />
                                        <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                                    </div>
                                );
                            })
                        )}
                    </div>
                    {plugins !== undefined && stale.length > 0 && (
                        <div className="plugin-settings__stale">
                            <h2>失效插件</h2>
                            <p>插件因外部原因不可用，例如清单被外部删除或脚本加载失败。卸载不会留在这一节。</p>
                            {stale.map((instance) => (
                                <div className="plugin-settings__row" key={instance.instanceId}>
                                    <span className="plugin-settings__copy">
                                        {instance.componentId}
                                        <small>清单已不存在</small>
                                    </span>
                                    <button
                                        className="plugin-settings__remove"
                                        type="button"
                                        onClick={(event) => handleRemoveRequest(instance, event.currentTarget)}
                                    >
                                        移除
                                    </button>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </section>
            {overlays}
        </>
    );
};

/** 添加插件：文件夹和压缩包都能多次选择，选中的来源一起排队。 */
const AddPluginDialog = ({
    picked,
    busy,
    onPick,
    onCancel,
    onStart,
}: {
    picked: PluginInstallPlan[]; // 已经选中、还没开始的计划。
    busy: boolean; // 系统选择框还开着。
    onPick: (source: "folder" | "archive") => void;
    onCancel: () => void;
    onStart: () => void;
}) => {
    const dialogRef = useRef<HTMLDialogElement>(null); // 添加插件弹窗。
    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog || dialog.open) return;
        dialog.showModal();
    }, []);
    return (
        <dialog
            ref={dialogRef}
            className="plugin-settings__dialog plugin-settings__dialog--add"
            aria-label="添加插件"
            onCancel={(event) => {
                event.preventDefault();
                onCancel();
            }}
        >
            <header>
                <h2>添加插件</h2>
            </header>
            <div className="plugin-settings__dialog-body">
                <p>可以多次选择文件夹或压缩包，选中的来源会一起安装。</p>
                {picked.length === 0 ? (
                    <p>还没有选择来源</p>
                ) : (
                    <ul className="plugin-settings__picks">
                        {picked.map((plan) => (
                            <li key={plan.planId}>
                                {plan.name} {plan.version}
                                {plan.conflict ? "（需确认）" : ""}
                            </li>
                        ))}
                    </ul>
                )}
            </div>
            <footer className="plugin-settings__dialog-actions">
                <button type="button" disabled={busy} onClick={() => onPick("folder")}>
                    选择文件夹
                </button>
                <button type="button" disabled={busy} onClick={() => onPick("archive")}>
                    选择压缩包
                </button>
                <button type="button" disabled={busy || picked.length === 0} onClick={onStart}>
                    开始安装
                </button>
                <button type="button" disabled={busy} onClick={onCancel}>
                    取消
                </button>
            </footer>
        </dialog>
    );
};

/** 卸载或移除共用的确认请求。 */
type PluginConfirmRequest =
    | { kind: "uninstall"; plugin: PluginRow; returnFocus: HTMLElement }
    | { kind: "remove"; instance: ComponentInstanceSnapshot; returnFocus: HTMLElement }
    | { kind: "install"; plan: PluginInstallPlan; returnFocus: HTMLElement };

/** 模态确认。焦点显式落在确认按钮，不依赖对话框的默认聚焦。 */
const PluginConfirmDialog = ({
    request,
    pending,
    onCancel,
    onConfirm,
    onPurge,
}: {
    request: PluginConfirmRequest;
    pending: boolean; // 命令进行中，按钮都不可用。
    onCancel: () => void;
    onConfirm: () => void; // 卸载或移除。卸载时不清理配置。
    onPurge: () => void; // 卸载并清理配置。移除弹窗不用。
}) => {
    const dialogRef = useRef<HTMLDialogElement>(null); // 原生对话框。
    const confirmRef = useRef<HTMLButtonElement>(null); // 预选的确认按钮。
    const uninstall = request.kind === "uninstall"; // 卸载与移除共用这一套键盘规则。
    const install = request.kind === "install" ? request.plan : undefined; // 覆盖或降级确认。

    useEffect(() => {
        const dialog = dialogRef.current; // 当前弹窗。
        const confirmButton = confirmRef.current; // 卸载或移除按钮。
        if (!dialog || !confirmButton) return;
        if (!dialog.open) dialog.showModal();
        confirmButton.focus();
        const trigger = request.returnFocus; // 关闭后要回到的按钮。
        return () => {
            if (dialog.open) dialog.close();
            if (trigger.isConnected) trigger.focus();
        };
    }, [request.returnFocus]);

    /** Tab 只在弹窗内的按钮之间循环。 */
    const handleKeyDown = (event: KeyboardEvent<HTMLDialogElement>) => {
        if (pending && (event.key === "Escape" || event.key === "Tab" || event.key === "Enter" || event.key === " ")) {
            event.preventDefault();
            return;
        }
        if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
            return;
        }
        if ((event.key === "Enter" || event.key === " ") && document.activeElement === confirmRef.current) {
            event.preventDefault();
            onConfirm();
            return;
        }
        if (event.key !== "Tab") return;
        const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; // 当前可聚焦的按钮。
        const first = items[0];
        const last = items[items.length - 1];
        if (!first || !last) return;
        const active = document.activeElement; // 现在的焦点。
        if (event.shiftKey && (active === first || !event.currentTarget.contains(active))) {
            event.preventDefault();
            last.focus();
        } else if (!event.shiftKey && active === last) {
            event.preventDefault();
            first.focus();
        }
    };

    return (
        <dialog
            ref={dialogRef}
            className={`plugin-settings__dialog plugin-settings__dialog--${request.kind}`}
            aria-label={install ? "确认安装" : uninstall ? "确认卸载" : "确认移除"}
            onCancel={(event) => {
                event.preventDefault();
                if (!pending) onCancel();
            }}
            onKeyDown={handleKeyDown}
        >
            <header>
                <h2>
                    {install
                        ? install.conflict?.downgrade
                            ? "卸载并安装"
                            : "覆盖安装"
                        : request.kind === "uninstall"
                          ? `卸载「${request.plugin.name}」？`
                          : request.kind === "remove"
                            ? `移除「${request.instance.componentId}」？`
                            : "确认安装"}
                </h2>
            </header>
            <div className="plugin-settings__dialog-body">
                {install?.conflict ? (
                    <>
                        <p className="plugin-settings__dialog-name">{install.name}</p>
                        <p>{installConflictIdentity(install.author, install.pluginId)}</p>
                        <p>{installVersionLine(install.conflict.installedVersion, install.conflict.incomingVersion)}</p>
                        {install.conflict.downgrade && <p>确认后会先移除该插件在桌面上的格子。</p>}
                    </>
                ) : uninstall ? (
                    <>
                        <p>卸载会同时移除该插件在桌面上的所有格子，并清理对应的布局设置。</p>
                        <p>插件配置文件会保留，重新安装后原有设置自动恢复。</p>
                    </>
                ) : (
                    <p>只从桌面移除这个格子，不删除插件文件，也不删除插件设置。</p>
                )}
            </div>
            <footer className="plugin-settings__dialog-actions">
                <button type="button" disabled={pending} onClick={onCancel}>
                    取消
                </button>
                {uninstall && (
                    <button type="button" className="plugin-settings__confirm" disabled={pending} onClick={onPurge}>
                        卸载并清理配置
                    </button>
                )}
                <button ref={confirmRef} type="button" className="plugin-settings__confirm" disabled={pending} onClick={onConfirm}>
                    {install ? installConfirmLabel(Boolean(install.conflict?.downgrade)) : uninstall ? "卸载" : "移除"}
                </button>
            </footer>
        </dialog>
    );
};

/** 一条设置条目。停用时只能看见标题。 */
const PluginEntry = ({
    pluginId,
    entry,
    open,
    locked,
    onToggle,
}: {
    pluginId: string;
    entry: PluginSettingsEntry;
    open: boolean; // 当前是否展开。
    locked: boolean; // 插件停用，不允许展开。
    onToggle: () => void;
}) => (
    <div className={`plugin-settings__entry${locked ? " plugin-settings__entry--locked" : ""}`}>
        <button className="plugin-settings__entry-head" type="button" aria-expanded={open && !locked} disabled={locked} onClick={onToggle}>
            {entry.name}
            <span
                className={`plugin-settings__chevron${open && !locked ? " plugin-settings__chevron--open" : ""}`}
                aria-hidden="true"
                dangerouslySetInnerHTML={{ __html: chevronIcon }}
            />
        </button>
        {open && !locked && (
            <div className="plugin-settings__entry-body">
                {typeof entry.render === "function" ? <PluginEntryMount pluginId={pluginId} entry={entry} /> : <p>该插件未提供设置内容。</p>}
            </div>
        )}
    </div>
);

/** 把作者内容挂进宿主容器，折叠或停用时卸掉。 */
const PluginEntryMount = ({ pluginId, entry }: { pluginId: string; entry: PluginSettingsEntry }) => {
    const [settings, setSettings] = useState<unknown>(null); // 最近一次读到的插件设置。
    const [host, setHost] = useState<HTMLDivElement | null>(null); // 作者容器。

    useEffect(() => {
        let alive = true; // 退订后忽略迟到读取。
        const read = () => {
            void getPluginSettings(pluginId)
                .then((value) => alive && setSettings(value ?? null))
                .catch(() => alive && setSettings(null));
        };
        read();
        const subscription = listenPluginSettings(
            (changedId) => changedId === pluginId && read(),
            () => undefined,
        );
        return () => {
            alive = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [pluginId]);

    useEffect(() => {
        if (!host) return;
        host.replaceChildren();
        let cleanup: void | (() => void);
        try {
            cleanup = entry.render(host, {
                pluginId,
                instanceId: "",
                collapsed: false,
                settings,
                saveSettings: (value) => savePluginSettings(pluginId, value),
            });
        } catch (error) {
            console.error(`插件设置条目渲染失败：${pluginId}`, error);
            host.replaceChildren();
            const fallback = document.createElement("p"); // 宿主兜底，不把作者异常留成空白。
            fallback.textContent = "设置内容渲染失败。";
            host.append(fallback);
        }
        return () => {
            if (typeof cleanup === "function") cleanup();
        };
    }, [host, entry, pluginId, settings]);

    return <div className="plugin-settings__author" ref={setHost} />;
};

/** 只有清单声明可创建时才显示入口。 */
export const showCreatePlugin = (creatable: boolean) => creatable === true;

/** 停用时不能创建格子。 */
export const createPluginBlocked = (pluginDisabled: boolean) => pluginDisabled;

/** 已有格子数。工作区还没到时按 0 计。 */
export const pluginTileCount = (components: readonly { componentId: string }[] | undefined, pluginId: string) =>
    components?.filter((item) => item.componentId === pluginId).length ?? 0;

/** 上限。0 是不限。满了给出原因。 */
export const pluginTileRoom = (maxTiles: number, used: number) => {
    if (!maxTiles || maxTiles < 0) return { full: false, remaining: undefined as number | undefined, reason: undefined as string | undefined };
    const remaining = Math.max(0, maxTiles - used); // 还能再加几个。
    const full = remaining === 0;
    return { full, remaining, reason: full ? `已达上限（${maxTiles} 个）` : undefined };
};

/** 加号按钮的悬停说明。不限时不报个数。 */
export const addToDesktopHint = (maxTiles: number, used: number) => {
    const room = pluginTileRoom(maxTiles, used);
    if (room.full) return room.reason ?? "已达上限";
    if (room.remaining === undefined) return "添加到桌面";
    return `还可以添加 ${room.remaining} 个格子`;
};

const addIcon = plusIcon.replace("<svg ", '<svg width="18" height="18" '); // 现成的加号图标，按钮不再写长文案。
const repoIcon = linkIcon.replace("<svg ", '<svg width="18" height="18" '); // 仓库入口，和信息行 18px 行高一致。
const removeIcon = trashIcon.replace("<svg ", '<svg width="18" height="18" '); // 现成的垃圾桶，表示卸载。

/** 标题下第一行：有作者才拼 @，没有就不留空符号。 */
export const pluginIdentity = (author: string | undefined, id: string) => (author ? `${author}@${id}` : id);

/** 头部启停按钮。停用时写启动，启用时写停用。 */
export const pluginPowerLabel = (pluginDisabled: boolean) => (pluginDisabled ? "启动插件" : "停用插件");

/** 简介超出一行才给出展开。收起后仍可再展开。 */
const PluginAbout = ({ text }: { text: string }) => {
    const textRef = useRef<HTMLSpanElement>(null); // 用来量是否超出一行。
    const [open, setOpen] = useState(false); // 展开后显示全文。
    const [overflow, setOverflow] = useState(false); // 收起时是否放不下。
    useLayoutEffect(() => {
        const node = textRef.current;
        if (!node || open) return;
        setOverflow(node.scrollWidth > node.clientWidth + 1);
    }, [text, open]);
    return (
        <p className="plugin-settings__about">
            <span ref={textRef} className={open ? "plugin-settings__about-text plugin-settings__about-text--open" : "plugin-settings__about-text"}>
                {text}
            </span>
            {overflow || open ? (
                <button className="plugin-settings__about-toggle" type="button" onClick={() => setOpen((current) => !current)}>
                    {open ? "收起" : "展开"}
                </button>
            ) : null}
        </p>
    );
};

/** 插件行右键菜单。四组用分隔线隔开。不可创建时不出现「添加到桌面」。 */
export const pluginContextMenu = (
    plugin: { creatable: boolean; maxTiles: number },
    actions: {
        used: number;
        pluginDisabled: boolean;
        onOpenSettings: () => void;
        onOpenDirectory: () => void;
        onCreate: () => void;
        busy?: boolean;
        onToggle: () => void;
        onUninstall: () => void;
    },
): DesktopTileMenuItem[] => {
    const room = pluginTileRoom(plugin.maxTiles, actions.used); // 上限是否已经用完。
    const items: DesktopTileMenuItem[] = [
        { label: "打开插件设置", onSelect: actions.onOpenSettings },
        { label: "打开插件目录", onSelect: actions.onOpenDirectory },
    ];
    if (plugin.creatable) {
        const blocked = room.full || actions.pluginDisabled || actions.busy === true; // 满了、停用或正在创建都不能再加。
        items.push({
            label: "添加到桌面",
            separatorBefore: true,
            disabled: blocked,
            description: room.full ? room.reason : actions.pluginDisabled ? "启用后可创建" : undefined,
            onSelect: actions.onCreate,
        });
    }
    items.push({
        label: actions.pluginDisabled ? "启用插件" : "停用插件",
        separatorBefore: true,
        onSelect: actions.onToggle,
    });
    items.push({ label: "卸载插件", danger: true, separatorBefore: true, onSelect: actions.onUninstall });
    return items;
};

/** 只要带 plugin. 前缀的清单，按显示名排序。 */
export const pluginRows = (manifests: readonly ComponentManifest[]): PluginRow[] =>
    manifests
        .filter((manifest) => manifest.id.startsWith("plugin."))
        .map((manifest) => ({
            id: manifest.id,
            name: manifest.name,
            creatable: manifest.creatable === true,
            maxTiles: manifest.maxTiles ?? 0,
            version: manifest.version,
            description: manifest.description,
            author: manifest.author,
            repository: manifest.repository,
            script: manifest.script,
        }))
        .sort((left, right) => left.name.localeCompare(right.name, "zh"));

/** 一条安装任务。只活在这次打开的设置页里。 */
interface InstallTask {
    id: string; // 计划标识。
    name: string; // 插件名。
    status: "running" | "done" | "failed" | "cancelled"; // 当前结果。
    stage: string; // 后端阶段。
    percent: number; // 0 到 100。
    detail?: string; // 失败原因。
}

const PLUGIN_DIRECTORY_DEFAULT = "%APPDATA%\\TIAmo\\plugins"; // 空偏好在界面上的默认目录。
const INSTALL_TASK_LIMIT = 20; // 内存里保留的任务条数。

/** 待保存路径。空串显示默认目录。 */
export const pluginDirectoryShown = (draft: string | undefined, saved: string | undefined) => {
    const value = draft !== undefined ? draft : (saved ?? ""); // 有草稿时以草稿为准。
    return value || PLUGIN_DIRECTORY_DEFAULT;
};

/** 只有草稿和已保存值不同才允许保存。 */
export const pluginDirectoryDirty = (draft: string | undefined, saved: string | undefined) => draft !== undefined && draft !== (saved ?? "");

/** 覆盖确认里的作者和标识。没有作者时只留 @标识。 */
export const installConflictIdentity = (author: string | undefined, pluginId: string | undefined) =>
    author ? `${author}@${pluginId ?? ""}` : `@${pluginId ?? ""}`;

/** 覆盖确认里的版本。降级不改这一行，仍用下面的说明。 */
export const installVersionLine = (installed: string, incoming: string) => `版本：${installed} -> ${incoming}`;

/** 升级和同版本覆盖，降级要先卸载。 */
export const installConfirmLabel = (downgrade: boolean) => (downgrade ? "卸载并安装" : "覆盖安装");

/** 进度阶段的界面文案。 */
export const installStageLabel = (stage: string) =>
    (({ validate: "校验", extract: "解压", copy: "复制", done: "完成", failed: "失败", cancelled: "已取消" }) as Record<string, string>)[stage] ?? stage;

/** 写入或更新一条任务，并丢掉超出的最旧记录。 */
export const rememberInstallTask = (tasks: readonly InstallTask[], next: InstallTask) => {
    const without = tasks.filter((task) => task.id !== next.id); // 同一计划只留最新状态。
    return [...without, next].slice(-INSTALL_TASK_LIMIT);
};

/** 进行中的总进度。N 是当前这一件，M 含已完成。 */
export const installBoard = (tasks: readonly InstallTask[]) => {
    const running = tasks.filter((task) => task.status === "running"); // 还在安装的行。
    const done = tasks.filter((task) => task.status === "done").length; // 已经完成的行。
    const total = Math.max(running.length + done, running.length); // 分母至少覆盖进行中的件数。
    const percent = running.length ? Math.round(running.reduce((sum, task) => sum + task.percent, 0) / running.length) : 0;
    return { running: running.length, text: running.length ? `正在安装 ${Math.min(done + 1, total)}/${total}` : "", percent };
};

/** 下方放不下且上方更宽时向上翻。内容比两边都高就内部滚动。 */
export const taskMenuPlacement = (menu: { top: number; bottom: number; height: number }, frame: { top: number; bottom: number }) => {
    const spaceAbove = menu.top - frame.top; // 菜单顶到内容区顶。
    const spaceBelow = frame.bottom - menu.top; // 菜单顶到内容区底。
    return { flipUp: menu.bottom > frame.bottom && spaceAbove > spaceBelow, scroll: menu.height > Math.max(spaceAbove, spaceBelow) };
};

/** 已注册的条目。没有注册对象时返回 undefined，有注册但没有条目时返回空数组。 */
export const entriesFor = (pluginId: string) => {
    const api = readPluginApi(pluginId); // 首份外壳对象。
    if (!api) return undefined;
    return api.settingsEntries ?? [];
};

/** 重新加载清掉旧失败，再导入一次插件模块。 */
const handleReload = (
    pluginId: string,
    script: string,
    setReloaded: (update: (current: Readonly<Record<string, string>>) => Readonly<Record<string, string>>) => void,
    setLoadedIds: (update: (current: ReadonlySet<string>) => ReadonlySet<string>) => void,
    setShellEpoch: (update: (value: number) => number) => void,
) => {
    setReloaded((current) => ({ ...current, [pluginId]: "" }));
    void loadPluginScript(pluginId, script, true).then((failure) => {
        if (failure) {
            setReloaded((current) => ({ ...current, [pluginId]: failure }));
            return;
        }
        setLoadedIds((current) => new Set(current).add(pluginId));
        setReloaded((current) => ({ ...current, [pluginId]: "ok" }));
        setShellEpoch((value) => value + 1);
    });
};

/** 脚本失败说明。重新加载成功后忽略模块里留下的旧失败。 */
const scriptFailure = (pluginId: string, reloaded: Readonly<Record<string, string>>) => {
    const again = reloaded[pluginId]; // 这一页重新加载的结果。
    if (again === "ok") return undefined;
    if (again) return again;
    return pluginScriptFailure(pluginId);
};

/** 安装成功后的提示。装完就能用，不再要求重启。 */
/** 通知第一行是插件名，第二行是结果。宿主只收一段正文。 */
export const installNotice = (name: string, body: string) => `${name}\n${body}`;

export const installSuccessMessage = () => "安装成功";

/** 安装失败只带后端原因，不再附加重启或默认停用说明。 */
export const installFailureMessage = (reason: string) => (reason.startsWith("安装失败") ? reason : `安装失败：${reason}`);

/** 一级行的条目数说明。 */
export const entrySummary = (pluginId: string, pluginDisabled: boolean, loaded: boolean, ignoreFailure = false) => {
    if (pluginDisabled) return "已停用";
    const failure = ignoreFailure ? undefined : pluginScriptFailure(pluginId); // 加载失败不伪装成零条目。
    if (failure === "插件模块没有默认导出" || failure === "插件导出不是对象") return failure;
    if (failure) return "设置条目加载失败";
    const shape = pluginSettingsShape(pluginId); // 不是数组时不说成没导出。
    if (shape) return shape;
    const entries = entriesFor(pluginId);
    if (!entries && !loaded) return "正在加载插件设置…";
    const count = entries?.length ?? 0; // 脚本已执行但没有条目。
    return count === 0 ? "插件未提供设置条目" : `${count} 个设置条目`;
};
