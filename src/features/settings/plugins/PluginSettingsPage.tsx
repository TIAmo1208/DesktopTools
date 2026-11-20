import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../../bridge/components";
import { requestLoopback } from "../../../bridge/loopback";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../../bridge/pluginSettings";
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
    readPluginApi,
    resolvedPluginDirectory,
    subscribePluginShell,
    uninstallPlugin,
    pluginScriptFailure,
    pluginScriptLoaded,
    pluginSettingsShape,
    type PluginSettingsEntry,
} from "../../../bridge/pluginScripts";
import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../../bridge/workspace";
import type { ComponentInstanceSnapshot, ComponentManifest, ComponentTileType } from "../../../contracts/component";
import type { PluginInstallPlan } from "../../../contracts/plugin";
import type { ApplicationPreferences } from "../../../contracts/settings";
import { invokeRuntime } from "../../../bridge/runtime";
import { publishMessage } from "../../notifications/messageNotification";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import backIcon from "../../../../resources/weather-back.svg?raw";
import chevronIcon from "../../../../resources/weather-chevron-right.svg?raw";
import trashIcon from "../../../../resources/weather-trash.svg?raw";
import plusIcon from "../../../../resources/collect-new.svg?raw";
import linkIcon from "../../../../resources/plugin-repository.svg?raw";
import infoIcon from "../../../../resources/info.svg?raw";
import { SettingsSwitch, SettingsAbout } from "../ui";
import { PluginTaskMenu } from "./PluginTaskMenu";
import {
    entriesFor,
    installConfirmLabel,
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
import { cancelledInstallTask, installBoard, installStageLabel } from "./pluginInstallTasks";
import { usePluginSettings } from "./usePluginSettings";
import "./PluginSettingsPage.css";

export { entriesFor, installConfirmLabel, installFailureMessage, installNotice, installSuccessMessage, pluginDirectoryShown, pluginRows, requestPluginScript, taskMenuPlacement };


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
    const {
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
    } = usePluginSettings({ preferences, onPreferences });

    // 从二级页返回列表后把焦点放回进入前的插件行；停留在二级页时不动，行已删除时不强制。
    const detailOpen = Boolean(selected);
    const returnFocusSelector = useRef<string | undefined>(undefined);
    useEffect(() => {
        if (!returnFocusSelector.current || detailOpen) return;
        const target = document.querySelector<HTMLButtonElement>(returnFocusSelector.current);
        returnFocusSelector.current = undefined;
        target?.focus();
    }, [detailOpen]);
    const menuPlugin = plugins?.find((plugin) => plugin.id === menu?.pluginId); // 右键时仍在列表里的插件。
    const menuUsed = menuPlugin ? pluginTileCount(components, menuPlugin.id) : 0; // 这个插件已经在桌面上的格子。
    const menuItems: DesktopTileMenuItem[] = menuPlugin
        ? pluginContextMenu({ ...menuPlugin, componentId: menuPlugin.id }, {
              used: menuUsed,
              components, // 按类型分别计数要用到每个实例的 state。
              pluginDisabled: disabledIds.includes(menuPlugin.id),
              onOpenSettings: () => {
                  returnFocusSelector.current = `[aria-label="${menuPlugin.name}设置"]`;
                  setSelectedId(menuPlugin.id);
              },
              onOpenDirectory: () =>
                  void openPluginDirectory(menuPlugin.id).catch((error: unknown) =>
                      publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }),
                  ),
              onCreate: (tileType) => handleCreate(menuPlugin.id, tileType),
              busy: creating,
              onToggle: () => handleEnabled(menuPlugin.id, disabledIds.includes(menuPlugin.id)),
              onUninstall: () => menu && handleUninstallRequest(menuPlugin, menu.anchor),
          })
        : [];
    const overlays = (
        <>
            {menu && menuItems.length > 0 && <DesktopTileMenu items={menuItems} left={menu.left} top={menu.top} onClose={() => setMenu(undefined)} />}
            {typeMenu && (
                <DesktopTileMenu
                    items={pluginTileTypeMenu(
                        typeMenu.tileTypes,
                        creating,
                        (tileType) => {
                            setTypeMenu(undefined);
                            handleCreate(typeMenu.pluginId, tileType);
                        },
                        pluginTypeStatus(components, typeMenu.pluginId, typeMenu.tileTypes, typeMenu.maxTiles), // 每类各自的额度。
                    )}
                    left={typeMenu.left}
                    top={typeMenu.top}
                    onClose={() => setTypeMenu(undefined)}
                />
            )}
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
        const scriptlessTheme = selected.role === "theme" && !selected.script; // 没有入口脚本的主题不加载，也不展示加载失败。
        const failure = scriptlessTheme ? undefined : scriptFailure(selected.id, reloaded); // 脚本失败说明；重新加载成功后不再沿用旧失败。
        const pending = !scriptlessTheme && !pluginDisabled && entries === undefined && !failure && !pluginScriptLoaded(selected.id); // 只有还没加载完才显示加载态。
        const usedTiles = pluginTileCount(components, selected.id); // 已在桌面上的格子数。
        const desktopInstanceId = components?.find((item) => item.componentId === selected.id)?.instanceId ?? ""; // 设置条目主动拉取用第一个桌面实例。
        const loopbackCredential = pluginLoopbackCredential(selected.id); // 插件声明的共享 token 配对入口。
        const typeStatus = pluginTypeStatus(components, selected.id, selected.tileTypes, selected.maxTiles); // 每个类型各自的额度。
        const tileRoom = pluginTileRoom(selected.maxTiles, usedTiles); // 没有声明类型时按整体额度判断。
        const allFull = typeStatus ? [...typeStatus.values()].every((room) => room.full) : tileRoom.full; // 声明了类型时全部类型都满才禁用。
        const tightest = pluginTightestRemaining(typeStatus); // 加号悬停按最紧的一类报数。
        return (
            <>
                <section className="plugin-settings plugin-settings--detail">
                    <div className="plugin-settings__detail-head">
                        <button className="plugin-settings__back" type="button" aria-label="返回插件" onClick={() => setSelectedId(undefined)} data-button="icon">
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
                             data-button="icon">
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: removeIcon }} />
                            </button>
                            <button
                                className="settings__button settings__icon-button"
                                type="button"
                                disabled={!showCreatePlugin(selected.creatable) || createPluginBlocked(pluginDisabled) || creating || allFull}
                                title={
                                    !showCreatePlugin(selected.creatable)
                                        ? "添加到桌面"
                                        : createPluginBlocked(pluginDisabled)
                                          ? "启用后可创建"
                                          : pluginTileTypesExpand(selected.tileTypes)
                                            ? "添加到桌面"
                                            : addToDesktopHint(selected.maxTiles, usedTiles, tightest)
                                }
                                aria-label="添加到桌面"
                                aria-haspopup={pluginTileTypesExpand(selected.tileTypes) ? "menu" : undefined}
                                onClick={(event) => {
                                    if (!pluginTileTypesExpand(selected.tileTypes)) {
                                        handleCreate(selected.id, pluginSingleTileType(selected.tileTypes)); // 只有一种类型时仍是加号按钮。
                                        return;
                                    }
                                    const rect = event.currentTarget.getBoundingClientRect(); // 下拉贴着按钮下沿。
                                    setTypeMenu({ pluginId: selected.id, tileTypes: selected.tileTypes, maxTiles: selected.maxTiles, left: rect.left, top: rect.bottom });
                                }}
                             data-button="icon">
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
                             data-button="secondary">
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: repoIcon }} />
                            </button>
                        ) : null}
                    </p>
                    {selected.description ? <SettingsAbout text={selected.description} /> : null}
                    {loopbackCredential ? <PluginLoopbackPairing pluginId={selected.id} metadata={loopbackCredential} disabled={disabled || pluginDisabled} /> : null}
                    {pending ? (
                        <p className="plugin-settings__status" role="status" aria-busy="true">
                            正在加载插件设置…
                        </p>
                    ) : failure ? (
                        <div className="plugin-settings__error" role="alert">
                            <p>{failure}</p>
                            <button
                                type="button"
                                onClick={() => handleReload(selected.id, selected.script, selected.role, setReloaded, setLoadedIds, setShellEpoch)}
                            >
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
                                    version={selected.version}
                                    instanceId={desktopInstanceId}
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
                    <button className="plugin-settings__add-main" type="button" disabled={disabled} onClick={() => setAddOpen(true)} data-button="ghost">
                        ＋ 添加插件
                    </button>
                    <button
                        className="plugin-settings__add-more"
                        type="button"
                        aria-label="任务进度"
                        aria-expanded={taskMenu}
                        onClick={() => setTaskMenu((open) => !open)}
                     data-button="ghost">
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
                            setTasks((current) => cancelledInstallTask(current, task));
                            void cancelPluginInstall(taskId).catch((error: unknown) =>
                                publishMessage({ type: "error", content: commandErrorMessage(error), source: "plugin-settings" }),
                            );
                        }}
                    />
                </div>
                <div className="plugin-settings__notice" role="note">
                    <span className="plugin-settings__notice-icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: infoIcon }} />
                    <p>请确认插件来源可信。插件在本机以你的权限运行，来源不明的插件可能损坏程序，或读取和泄露你的数据。</p>
                </div>
                <div className="plugin-settings__body">
                    <section className="settings__group" aria-labelledby="plugin-directory-title">
                        <h2 className="settings__group-title" id="plugin-directory-title">插件目录</h2>
                    <form
                        className="settings__row settings__directory"
                        onSubmit={(event) => {
                            event.preventDefault();
                            if (!pluginDirectoryDirty(directoryDraft, preferences?.pluginDirectory)) return;
                            onPreferences({ pluginDirectory: directoryDraft ?? "" });
                            setDirectoryDraft(undefined);
                        }}
                    >
                        <span className="settings__row-title" title={shownDirectory}>
                            插件目录（下次启动生效）
                            <span className="settings__directory-path">{shownDirectory}</span>
                        </span>
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            title="复制插件目录"
                            aria-label="复制插件目录"
                            disabled={disabled || !preferences}
                            onClick={handleCopyDirectory}
                         data-button="icon">
                            <span className="settings__directory-icon settings__directory-icon--copy" aria-hidden="true" />
                        </button>
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            title="选择插件目录"
                            aria-label="选择插件目录"
                            disabled={disabled || !preferences}
                            onClick={handleChooseDirectory}
                         data-button="icon">
                            <span className="settings__directory-icon settings__directory-icon--folder" aria-hidden="true" />
                        </button>
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            title="恢复默认目录"
                            aria-label="恢复默认目录"
                            disabled={disabled || !preferences}
                            onClick={() => setDirectoryDraft("")}
                         data-button="icon">
                            <span className="settings__directory-icon settings__directory-icon--reset" aria-hidden="true" />
                        </button>
                        <button
                            className="settings__button"
                            type="submit"
                            disabled={disabled || !preferences || !pluginDirectoryDirty(directoryDraft, preferences?.pluginDirectory)}
                         data-button="secondary">
                            保存
                        </button>
                    </form>
                    </section>
                    <section className="settings__group" aria-labelledby="plugin-installed-title">
                        <h2 className="settings__group-title" id="plugin-installed-title">已安装插件</h2>
                        {plugins === undefined ? null : plugins.length === 0 ? (
                            <p className="plugin-settings__empty">
                                尚未安装插件
                                <small>将插件放入插件目录后重新打开程序。</small>
                            </p>
                        ) : (
                            <ul className="plugin-settings__list settings__list" aria-labelledby="plugin-installed-title">
                            {plugins.map((plugin) => {
                                const pluginDisabled = disabledIds.includes(plugin.id); // 本行是否停用。
                                return (
                                    <li
                                        className={`plugin-settings__row plugin-settings__row--link${pluginDisabled ? " plugin-settings__row--disabled" : ""}${menu?.pluginId === plugin.id ? " plugin-settings__row--selected" : ""}`}
                                        key={plugin.id}
                                        onContextMenu={(event) => {
                                            event.preventDefault();
                                            setMenu({ pluginId: plugin.id, left: event.clientX, top: event.clientY, anchor: event.currentTarget });
                                        }}
                                    >
                                        <span className="plugin-settings__copy">
                                            {plugin.name}
                                            <small>
                                                {entrySummary(
                                                    plugin.id,
                                                    pluginDisabled,
                                                    (plugin.role === "theme" && !plugin.script) ||
                                                        loadedIds.has(plugin.id) ||
                                                        pluginScriptLoaded(plugin.id) ||
                                                        reloaded[plugin.id] === "ok",
                                                    reloaded[plugin.id] === "ok",
                                                )}
                                            </small>
                                        </span>
                                        <button
                                            className="plugin-settings__uninstall"
                                            type="button"
                                            aria-label={`卸载${plugin.name}`}
                                            title="卸载"
                                            onClick={(event) => handleUninstallRequest(plugin, event.currentTarget)}
                                         data-button="icon">
                                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: trashIcon }} />
                                        </button>
                                        <SettingsSwitch
                                            label={`启用${plugin.name}`}
                                            checked={!pluginDisabled}
                                            disabled={disabled || !preferences}
                                            onChange={(checked) => handleEnabled(plugin.id, checked)}
                                        />
                                        <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                                        <button
                                            className="plugin-settings__hit"
                                            type="button"
                                            data-button="row-hit"
                                            aria-label={`${plugin.name}设置`}
                                            onClick={() => {
                                                returnFocusSelector.current = `[aria-label="${plugin.name}设置"]`;
                                                setSelectedId(plugin.id);
                                            }}
                                        />
                                    </li>
                                );
                            })}
                            </ul>
                        )}
                    </section>
                    {plugins !== undefined && stale.length > 0 && (
                        <section className="plugin-settings__stale settings__group" aria-labelledby="plugin-stale-title">
                            <h2 className="settings__group-title" id="plugin-stale-title">失效插件</h2>
                            <p>插件因外部原因不可用，例如清单被外部删除或脚本加载失败。</p>
                            <ul className="plugin-settings__list settings__list" aria-labelledby="plugin-stale-title">
                            {stale.map((instance) => (
                                <li className="plugin-settings__row" key={instance.instanceId}>
                                    <span className="plugin-settings__copy">
                                        {instance.componentId}
                                        <small>清单已不存在</small>
                                    </span>
                                    <button
                                        className="plugin-settings__remove"
                                        type="button"
                                        onClick={(event) => handleRemoveRequest(instance, event.currentTarget)}
                                     data-button="danger">
                                        移除
                                    </button>
                                </li>
                            ))}
                            </ul>
                        </section>
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
                <button type="button" disabled={busy} onClick={onCancel}>
                    取消
                </button>
                <button type="button" disabled={busy} onClick={() => onPick("folder")}>
                    选择文件夹
                </button>
                <button type="button" disabled={busy} onClick={() => onPick("archive")}>
                    选择压缩包
                </button>
                <button type="button" disabled={busy || picked.length === 0} onClick={onStart}>
                    开始安装
                </button>
            </footer>
        </dialog>
    );
};

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
                    <button type="button" className="plugin-settings__confirm" disabled={pending} onClick={onPurge} data-button="danger">
                        卸载并清理配置
                    </button>
                )}
                <button ref={confirmRef} type="button" className="plugin-settings__confirm" disabled={pending} onClick={onConfirm} data-button={install ? "primary" : "danger"}>
                    {install ? installConfirmLabel(Boolean(install.conflict?.downgrade)) : uninstall ? "卸载" : "移除"}
                </button>
            </footer>
        </dialog>
    );
};

/** 一条设置条目。停用时只能看见标题。 */
/** 插件脚本可声明的 loopback 凭据元数据；没有声明时不显示配对入口。 */
export interface PluginLoopbackCredential {
    target: string; // Windows Credential Manager 目标。
    headerName?: string; // 注入与校验使用的头名。
    title?: string; // 面板标题。
    hint?: string; // 使用说明。
}

/** 从已注册外壳读取凭据元数据；格式不合法时不显示入口。 */
export const pluginLoopbackCredential = (pluginId: string): PluginLoopbackCredential | undefined => {
    const metadata = (readPluginApi(pluginId) as unknown as { loopbackCredential?: unknown } | undefined)?.loopbackCredential;
    if (typeof metadata !== "object" || metadata === null) return undefined;
    const record = metadata as Record<string, unknown>;
    const target = typeof record.target === "string" ? record.target.trim() : "";
    if (!target) return undefined;
    const text = (value: unknown) => (typeof value === "string" && value.trim() ? value.trim() : undefined);
    return { target, headerName: text(record.headerName), title: text(record.title), hint: text(record.hint) };
};

/**
 * 配对变化后把版本写进插件设置，借 plugin-settings-changed 通知桌面插件重试注册。
 * 不新增宿主命令；写入失败由保存/清除流程明确报错。
 */
export const bumpPairingVersion = async (pluginId: string): Promise<void> => {
    const current = await getPluginSettings(pluginId).catch(() => null);
    const base = current && typeof current === "object" && !Array.isArray(current) ? (current as Record<string, unknown>) : {};
    const previous = Number(base.pairingVersion);
    await savePluginSettings(pluginId, { ...base, pairingVersion: Math.max(Date.now(), Number.isFinite(previous) ? previous + 1 : 0) });
};

/** 共享 token 配对：只写入凭据管理器，已保存的 token 不回显。 */
const PluginLoopbackPairing = ({ pluginId, metadata, disabled }: { pluginId: string; metadata: PluginLoopbackCredential; disabled: boolean }) => {
    const [secret, setSecret] = useState(""); // 只有本次输入，保存后立即清空。
    const [busy, setBusy] = useState(false); // 保存或清除进行中。
    const [feedback, setFeedback] = useState<{ tone: "ok" | "error"; text: string }>(); // 面板内的可见反馈。
    const [confirmClear, setConfirmClear] = useState(false); // 清除凭据不可恢复，先要一次二次确认。
    const inputId = `loopback-token-${metadata.target.replace(/[^a-zA-Z0-9_-]/g, "-")}`; // 目标里可能有斜杠。
    const run = (task: Promise<unknown>, success: string) => {
        setBusy(true);
        setFeedback(undefined);
        void task
            .then(() => bumpPairingVersion(pluginId))
            .then(() => {
                setSecret("");
                setFeedback({ tone: "ok", text: success });
                publishMessage({ type: "success", content: success, source: "plugin-settings" });
            })
            .catch((error: unknown) => {
                const text = `操作失败：${commandErrorMessage(error)}`;
                setFeedback({ tone: "error", text });
                publishMessage({ type: "error", content: text, source: "plugin-settings" });
            })
            .finally(() => setBusy(false));
    };
    const handleSave = () => {
        const value = secret.trim();
        if (!value) {
            setFeedback({ tone: "error", text: "请先粘贴共享 token" });
            return;
        }
        run(invokeRuntime("set_loopback_credential", { target: metadata.target, secret: value }), "已保存到 Windows 凭据管理器，页面不会回显 token");
    };
    const handleClear = () => run(invokeRuntime("clear_loopback_credential", { target: metadata.target }), "已清除保存的共享 token");
    return (
        <div className="plugin-settings__pairing">
            <h3>{metadata.title ?? "共享 token"}</h3>
            {metadata.hint ? <p className="plugin-settings__pairing-hint">{metadata.hint}</p> : null}
            <div className="plugin-settings__pairing-form">
                <label className="plugin-settings__pairing-label" htmlFor={inputId}>
                    共享 token
                </label>
                <input
                    id={inputId}
                    className="plugin-settings__pairing-input"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="粘贴 token"
                    value={secret}
                    disabled={disabled || busy}
                    onChange={(event) => setSecret(event.target.value)}
                    onKeyDown={(event) => {
                        if (event.key === "Enter") handleSave();
                    }}
                />
                <button className="settings__button" type="button" disabled={disabled || busy} onClick={handleSave} data-button="secondary">
                    保存
                </button>
                {confirmClear ? (
                    <>
                        <button
                            className="settings__button plugin-settings__pairing-clear"
                            type="button"
                            disabled={disabled || busy}
                            onClick={() => {
                                setConfirmClear(false);
                                handleClear();
                            }}
                            data-button="danger"
                        >
                            确认清除
                        </button>
                        <button className="settings__button" type="button" disabled={disabled || busy} onClick={() => setConfirmClear(false)} data-button="secondary">
                            取消
                        </button>
                    </>
                ) : (
                    <button
                        className="settings__button plugin-settings__pairing-clear"
                        type="button"
                        disabled={disabled || busy}
                        onClick={() => setConfirmClear(true)}
                        data-button="danger"
                    >
                        清除
                    </button>
                )}
            </div>
            {feedback ? (
                <p className={`plugin-settings__pairing-feedback plugin-settings__pairing-feedback--${feedback.tone}`} role={feedback.tone === "error" ? "alert" : "status"}>
                    {feedback.text}
                </p>
            ) : null}
        </div>
    );
};

const PluginEntry = ({
    pluginId,
    entry,
    version,
    instanceId,
    open,
    locked,
    onToggle,
}: {
    pluginId: string;
    entry: PluginSettingsEntry;
    version?: string; // 清单版本，设置条目可只读展示。
    instanceId: string; // 第一个桌面实例；主动拉取与实例上下文使用。
    open: boolean; // 当前是否展开。
    locked: boolean; // 插件停用，不允许展开。
    onToggle: () => void;
}) => (
    <div className={`plugin-settings__entry${locked ? " plugin-settings__entry--locked" : ""}`}>
        <button className="plugin-settings__entry-head" type="button" aria-expanded={open && !locked} disabled={locked} onClick={onToggle} data-button="row-hit">
            {entry.name}
            <span
                className={`plugin-settings__chevron${open && !locked ? " plugin-settings__chevron--open" : ""}`}
                aria-hidden="true"
                dangerouslySetInnerHTML={{ __html: chevronIcon }}
            />
        </button>
        {open && !locked && (
            <div className="plugin-settings__entry-body">
                {typeof entry.render === "function" ? (
                    <PluginEntryMount pluginId={pluginId} entry={entry} version={version} instanceId={instanceId} />
                ) : (
                    <p>该插件未提供设置内容。</p>
                )}
            </div>
        )}
    </div>
);

/** 把作者内容挂进宿主容器，折叠或停用时卸掉。 */
const PluginEntryMount = ({ pluginId, entry, version, instanceId }: { pluginId: string; entry: PluginSettingsEntry; version?: string; instanceId: string }) => {
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
                instanceId,
                tileType: "", // 设置条目不对应实例，没有类型。
                collapsed: false,
                pluginVersion: version,
                settings,
                saveSettings: (value) => savePluginSettings(pluginId, value),
                requestLoopback: instanceId ? (channelId, method, path) => requestLoopback(instanceId, channelId, method, path) : undefined,
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
    }, [host, entry, pluginId, settings, version, instanceId]);

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

/** 创建入口展开后的类型列表。显示名是菜单文案，键交给创建命令。每项按自己那一类判断是否已满。 */
export const pluginTileTypeMenu = (tileTypes: readonly ComponentTileType[], blocked: boolean, onCreate: (tileType: string) => void, typeStatus?: PluginTypeStatus): DesktopTileMenuItem[] =>
    tileTypes.map((tileType) => {
        const room = typeStatus?.get(tileType.key); // 该类还能不能再加。
        const full = room?.full === true; // 只有这一项满了。
        return { label: tileType.name, disabled: blocked || full, description: full && !blocked ? room?.reason : undefined, onSelect: () => onCreate(tileType.key) };
    });

/** 类型键到该类剩余额度的映射；隐式单类型返回 undefined，调用方按整体额度处理。 */
export type PluginTypeStatus = Map<string, ReturnType<typeof pluginTileRoom>>;

/** 按实例的 state.tileType 统计每一类已用数量。缺失类型记到第一节（默认类型）。 */
export const pluginTypeUsage = (components: readonly { componentId: string; state?: unknown }[] | undefined, pluginId: string, tileTypes: readonly ComponentTileType[]) => {
    const used = new Map<string, number>(); // 类型键到已用数量。
    if (!tileTypes.length) return used;
    const fallback = tileTypes[0].key; // 旧实例按默认类型计。
    for (const item of components ?? []) {
        if (item.componentId !== pluginId) continue;
        const value = typeof item.state === "object" && item.state !== null ? (item.state as Record<string, unknown>).tileType : undefined;
        const key = typeof value === "string" && value ? value : fallback;
        used.set(key, (used.get(key) ?? 0) + 1);
    }
    return used;
};

/** 每一类各自的额度状态。隐式单类型返回 undefined。 */
export const pluginTypeStatus = (components: readonly { componentId: string; state?: unknown }[] | undefined, pluginId: string, tileTypes: readonly ComponentTileType[], maxTiles: number): PluginTypeStatus | undefined => {
    if (!tileTypes.length) return undefined;
    const used = pluginTypeUsage(components, pluginId, tileTypes);
    return new Map(tileTypes.map((tileType) => [tileType.key, pluginTileRoom(maxTiles, used.get(tileType.key) ?? 0)]));
};

/** 各类剩余额度里最少的一类；全部不限时为 undefined。用于加号按钮的悬停说明。 */
export const pluginTightestRemaining = (status: PluginTypeStatus | undefined) => {
    const remaining = [...(status?.values() ?? [])].map((room) => room.remaining).filter((value): value is number => value !== undefined);
    return remaining.length ? Math.min(...remaining) : undefined;
};

/** 加一个格子。给了类型就写进实例 state，没给就按隐式单类型创建。 */
export const createTileState = (tileType?: string) => (tileType ? { tileType } : {});

/** 创建入口是否展开成类型列表：多于一种类型才展开。 */
export const pluginTileTypesExpand = (tileTypes: readonly ComponentTileType[]) => tileTypes.length > 1;

/** 只有一种类型时直接创建它，让实例也带上类型，插件不必再按空串猜默认值。 */
export const pluginSingleTileType = (tileTypes: readonly ComponentTileType[]) => (tileTypes.length === 1 ? tileTypes[0].key : undefined);

/** 加号按钮悬停说明。不限时不报个数；声明了类型时报最紧那一类的剩余。 */
export const addToDesktopHint = (maxTiles: number, used: number, tightest: number | undefined = undefined) => {
    if (tightest !== undefined) return tightest === 0 ? `已达上限（${maxTiles} 个）` : `还可以添加 ${tightest} 个格子`; // 每类各自计数。
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


/**
 * 插件行右键菜单。四组用分隔线隔开。不可创建时不出现「添加到桌面」。
 * 多于一种格子类型时，「添加到桌面」变成二级菜单，逐项列出类型。
 */
export const pluginContextMenu = (
    plugin: { creatable: boolean; maxTiles: number; tileTypes?: readonly ComponentTileType[]; componentId?: string },
    actions: {
        used: number;
        components?: readonly { componentId: string; state?: unknown }[]; // 已加载的工作区实例，用来按类型分别计数。
        pluginDisabled: boolean;
        onOpenSettings: () => void;
        onOpenDirectory: () => void;
        onCreate: (tileType?: string) => void;
        busy?: boolean;
        onToggle: () => void;
        onUninstall: () => void;
    },
): DesktopTileMenuItem[] => {
    const room = pluginTileRoom(plugin.maxTiles, actions.used); // 没有声明类型时按整体额度判断。
    const tileTypes = plugin.tileTypes ?? []; // 缺省按隐式单类型处理。
    const typeStatus = pluginTypeStatus(actions.components, plugin.componentId ?? "", tileTypes, plugin.maxTiles); // 声明了类型时每类各自的额度。
    const singleRoom = tileTypes.length === 1 ? typeStatus?.get(tileTypes[0].key) : undefined; // 只有一种类型时按那一类判断。
    const full = singleRoom?.full ?? room.full; // 这个入口还能不能再加。
    const items: DesktopTileMenuItem[] = [
        { label: "打开插件设置", onSelect: actions.onOpenSettings },
        { label: "打开插件目录", onSelect: actions.onOpenDirectory },
    ];
    if (plugin.creatable) {
        const blocked = full || actions.pluginDisabled || actions.busy === true; // 满了、停用或正在创建都不能再加。
        const reason = full ? (singleRoom?.reason ?? room.reason) : actions.pluginDisabled ? "启用后可创建" : undefined; // 禁用原因。
        items.push(
            pluginTileTypesExpand(tileTypes)
                ? { label: "添加到桌面", separatorBefore: true, disabled: actions.pluginDisabled || actions.busy === true, description: actions.pluginDisabled ? "启用后可创建" : undefined, children: pluginTileTypeMenu(tileTypes, actions.pluginDisabled || actions.busy === true, (tileType) => actions.onCreate(tileType), typeStatus) }
                : { label: "添加到桌面", separatorBefore: true, disabled: blocked, description: reason, onSelect: () => actions.onCreate(pluginSingleTileType(tileTypes)) },
        );
    }
    items.push({
        label: actions.pluginDisabled ? "启用插件" : "停用插件",
        separatorBefore: true,
        onSelect: actions.onToggle,
    });
    items.push({ label: "卸载插件", danger: true, separatorBefore: true, onSelect: actions.onUninstall });
    return items;
};

/** 只有草稿和已保存值不同才允许保存。 */
export const pluginDirectoryDirty = (draft: string | undefined, saved: string | undefined) => draft !== undefined && draft !== (saved ?? "");

/** 覆盖确认里的作者和标识。没有作者时只留 @标识。 */
export const installConflictIdentity = (author: string | undefined, pluginId: string | undefined) =>
    author ? `${author}@${pluginId ?? ""}` : `@${pluginId ?? ""}`;

/** 覆盖确认里的版本。降级不改这一行，仍用下面的说明。 */
export const installVersionLine = (installed: string, incoming: string) => `版本：${installed} -> ${incoming}`;

/** 重新加载清掉旧失败，再导入一次插件模块。没有入口脚本的主题不提供这一步。 */
const handleReload = (
    pluginId: string,
    script: string,
    role: string | undefined,
    setReloaded: (update: (current: Readonly<Record<string, string>>) => Readonly<Record<string, string>>) => void,
    setLoadedIds: (update: (current: ReadonlySet<string>) => ReadonlySet<string>) => void,
    setShellEpoch: (update: (value: number) => number) => void,
) => {
    requestPluginScript(
        { id: pluginId, role, script },
        (id, next, reload) => {
            setReloaded((current) => ({ ...current, [pluginId]: "" }));
            void loadPluginScript(id, next, reload).then((failure) => {
                if (failure) {
                    setReloaded((current) => ({ ...current, [pluginId]: failure }));
                    return;
                }
                setLoadedIds((current) => new Set(current).add(pluginId));
                setReloaded((current) => ({ ...current, [pluginId]: "ok" }));
                setShellEpoch((value) => value + 1);
            });
        },
        true,
    );
};

/** 脚本失败说明。重新加载成功后忽略模块里留下的旧失败。 */
const scriptFailure = (pluginId: string, reloaded: Readonly<Record<string, string>>) => {
    const again = reloaded[pluginId]; // 这一页重新加载的结果。
    if (again === "ok") return undefined;
    if (again) return again;
    return pluginScriptFailure(pluginId);
};

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
