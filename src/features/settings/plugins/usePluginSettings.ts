import { useEffect, useRef, useState } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../../bridge/components";
import {
    applyPluginInstall,
    cancelPluginInstall,
    choosePluginDirectory,
    commandErrorMessage,
    createPluginComponent,
    listenPluginInstallProgress,
    listenPluginScripts,
    loadPluginScript,
    openPluginDirectory,
    preparePluginInstallsFromArchive,
    preparePluginInstallsFromFolder,
    resolvedPluginDirectory,
    subscribePluginShell,
    uninstallPlugin,
} from "../../../bridge/pluginScripts";
import { invokeRuntime } from "../../../bridge/runtime";
import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../../bridge/workspace";
import type { ComponentInstanceSnapshot, ComponentManifest, ComponentTileType } from "../../../contracts/component";
import type { PluginInstallPlan } from "../../../contracts/plugin";
import type { ApplicationPreferences } from "../../../contracts/settings";
import { publishMessage } from "../../notifications/messageNotification";
import {
    installFailureMessage,
    installNotice,
    installSuccessMessage,
    pluginDirectoryShown,
    pluginRows,
    requestPluginScript,
    taskMenuPlacement,
    type PluginConfirmRequest,
    type PluginRow,
} from "./pluginSettingsModel";
import { failedInstallTask, progressedInstallTask, rememberInstallTask, type InstallTask } from "./pluginInstallTasks";

/** 插件页的领域状态与动作；组件只保留 JSX 与右键菜单拼装。 */
export interface PluginSettingsOptions {
    preferences?: ApplicationPreferences; // 当前偏好；未就绪时不能改停用名单。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 写 disabledPlugins。
}

export const usePluginSettings = ({ preferences, onPreferences }: PluginSettingsOptions) => {
    const [manifests, setManifests] = useState<ComponentManifest[]>(); // 未赋值表示清单还没到。
    const [components, setComponents] = useState<ComponentInstanceSnapshot[]>(); // 工作区实例；未到时不判断失效。
    const [removedIds, setRemovedIds] = useState<ReadonlySet<string>>(new Set()); // 本次已卸载、快照尚未刷新的插件。
    const [selectedId, setSelectedId] = useState<string>(); // 当前就地展开的插件；离开本页即丢掉。
    const [openEntryIds, setOpenEntryIds] = useState<ReadonlySet<string>>(new Set()); // 本插件内展开的条目，互不联动。
    const [, setShellEpoch] = useState(0); // 脚本注册或加载成功后重读条目。
    const [loadedIds, setLoadedIds] = useState<ReadonlySet<string>>(new Set()); // 设置窗口里已经执行成功的脚本。
    const [reloaded, setReloaded] = useState<Readonly<Record<string, string>>>({}); // 重新加载后的结果；ok 表示不再沿用旧失败。
    const [menu, setMenu] = useState<{ pluginId: string; left: number; top: number; anchor: HTMLElement }>(); // 右键菜单锚点。
    const [typeMenu, setTypeMenu] = useState<{ pluginId: string; tileTypes: ComponentTileType[]; maxTiles: number; left: number; top: number }>(); // 二级页加号展开的类型列表。
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
    const [resolvedDirectory, setResolvedDirectory] = useState<string>(); // 本次启动实际使用的目录。
    const shownDirectory = (directoryDraft ?? "") || resolvedDirectory || pluginDirectoryShown(undefined, preferences?.pluginDirectory); // 草稿优先，其次是已生效路径。
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
        let alive = true; // 卸载后忽略迟到结果。
        void resolvedPluginDirectory()
            .then((path) => alive && setResolvedDirectory(path))
            .catch(() => undefined);
        return () => {
            alive = false;
        };
    }, []);

    useEffect(() => {
        if (!plugins || !preferences) return;
        for (const plugin of plugins) {
            if (preferences.disabledPlugins.includes(plugin.id)) continue;
            requestPluginScript(plugin, (id, script) => void loadPluginScript(id, script));
        }
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
            setTasks((current) => progressedInstallTask(current, progress)); // 已结束的行不被迟到进度改回进行中。
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
                setTasks((current) =>
                    failedInstallTask(current, {
                        id: plan.planId,
                        name: plan.name,
                        cancelled: message === "安装已取消",
                        detail: message === "安装已取消" ? message : installFailureMessage(message),
                    }),
                );
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

    /** 复制当前插件目录；剪贴板不可用时提示手动复制。 */
    const handleCopyDirectory = () => {
        const path = shownDirectory; // 复制正在显示的这一份。
        const clipboard = navigator.clipboard; // 只有安全上下文才提供。
        const failed = () => publishMessage({ type: "error", content: "复制失败，请手动选择路径", source: "plugin-settings" });
        if (!clipboard) {
            failed();
            return;
        }
        void clipboard.writeText(path).then(() => publishMessage({ type: "success", content: "已复制插件目录", source: "plugin-settings" }), failed);
    };

    /** 在桌面创建一个该插件的格子。给了类型就写进实例 state。失败把错误说出来。 */
    const handleCreate = (pluginId: string, tileType?: string) => {
        if (creating) return;
        setCreating(true);
        void createPluginComponent(pluginId, tileType)
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

    return {
        components,
        setSelectedId,
        openEntryIds,
        setOpenEntryIds,
        setShellEpoch,
        loadedIds,
        setLoadedIds,
        reloaded,
        setReloaded,
        menu,
        setMenu,
        typeMenu,
        setTypeMenu,
        confirm,
        setConfirm,
        confirming,
        creating,
        installing,
        addOpen,
        setAddOpen,
        picked,
        setPicked,
        taskMenu,
        setTaskMenu,
        taskMenuUp,
        tasks,
        setTasks,
        directoryDraft,
        setDirectoryDraft,
        shownDirectory,
        plugins,
        selected,
        stale,
        disabledIds,
        handleEnabled,
        enqueueInstalls,
        pickInstallSource,
        handleChooseDirectory,
        handleCopyDirectory,
        handleCreate,
        handleOpenRepository,
        handleUninstallRequest,
        handleRemoveRequest,
        handleConfirm,
    };
};
