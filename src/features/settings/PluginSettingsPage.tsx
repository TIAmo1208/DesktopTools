import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import { invokeRuntime } from "../../bridge/runtime";
import {
    openPluginDirectory,
    listenPluginScripts,
    loadPluginScript,
    pluginScriptFailure,
    publishPluginShell,
    readPluginApi,
    subscribePluginShell,
    uninstallPlugin,
    type PluginSettingsEntry,
} from "../../bridge/pluginScripts";
import { applyWorkspaceCommand, getWorkspaceSnapshot, listenWorkspaceSnapshots } from "../../bridge/workspace";
import type { ComponentInstanceSnapshot, ComponentManifest } from "../../contracts/component";
import type { ApplicationPreferences } from "../../contracts/settings";
import { publishMessage } from "../notifications/messageNotification";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import backIcon from "../../../resources/weather-back.svg?raw";
import chevronIcon from "../../../resources/weather-chevron-right.svg?raw";
import trashIcon from "../../../resources/weather-trash.svg?raw";
import { SettingsSwitch } from "./SettingsSwitch";
import "./PluginSettingsPage.css";

/** 设置页里的一个插件清单。 */
interface PluginRow {
    id: string; // 清单标识。
    name: string; // 清单显示名。
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
    const plugins = manifests === undefined ? undefined : pluginRows(manifests).filter((plugin) => !removedIds.has(plugin.id)); // 仍显示的插件行。
    const selected = plugins?.find((plugin) => plugin.id === selectedId); // 仍在清单里的当前插件。
    const stale = manifests && components ? components.filter((item) => !manifests.some((manifest) => manifest.id === item.componentId)) : []; // 清单里已经没有的实例。
    const disabledIds = preferences?.disabledPlugins ?? []; // 停用名单。

    useEffect(() => {
        publishPluginShell();
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
        for (const plugin of plugins) if (!preferences.disabledPlugins.includes(plugin.id)) void loadPluginScript(plugin.id);
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

    /** 写入停用名单。启用后的加载由上面的效果在偏好落地后再发起。 */
    const handleEnabled = (pluginId: string, enabled: boolean) => {
        if (!preferences) return;
        const next = enabled ? preferences.disabledPlugins.filter((id) => id !== pluginId) : [...preferences.disabledPlugins, pluginId];
        onPreferences({ disabledPlugins: next });
        if (!enabled) setOpenEntryIds(new Set());
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

    /** 执行当前确认。失败留在弹窗里，成功后关掉并把焦点还给触发元素。 */
    const handleConfirm = async () => {
        if (!confirm || confirming) return;
        setConfirming(true);
        try {
            if (confirm.kind === "uninstall") {
                await uninstallPlugin(confirm.plugin.id);
                setRemovedIds((current) => new Set(current).add(confirm.plugin.id));
                if (selectedId === confirm.plugin.id) setSelectedId(undefined);
                publishMessage({ type: "success", content: "已卸载，重启后桌面上的格子会变为失效实例", source: "plugin-settings" });
            } else {
                await applyWorkspaceCommand(confirm.instance.monitorId, { type: "remove-component", instanceId: confirm.instance.instanceId });
            }
            setConfirm(undefined);
        } catch (error) {
            publishMessage({ type: "error", content: error instanceof Error ? error.message : "插件操作失败", source: "plugin-settings" });
        } finally {
            setConfirming(false);
        }
    };

    const menuPlugin = plugins?.find((plugin) => plugin.id === menu?.pluginId); // 右键时仍在列表里的插件。
    const menuItems: DesktopTileMenuItem[] = menuPlugin
        ? [
              {
                  label: "打开插件目录",
                  onSelect: () =>
                      void openPluginDirectory(menuPlugin.id).catch((error: unknown) =>
                          publishMessage({ type: "error", content: error instanceof Error ? error.message : "打开插件目录失败", source: "plugin-settings" }),
                      ),
              },
              {
                  label: disabledIds.includes(menuPlugin.id) ? "启用插件" : "停用插件",
                  onSelect: () => handleEnabled(menuPlugin.id, disabledIds.includes(menuPlugin.id)),
              },
              { label: "卸载插件", danger: true, separatorBefore: true, onSelect: () => menu && handleUninstallRequest(menuPlugin, menu.anchor) },
          ]
        : [];
    const overlays = (
        <>
            {menu && menuItems.length > 0 && <DesktopTileMenu items={menuItems} left={menu.left} top={menu.top} onClose={() => setMenu(undefined)} />}
            {confirm && (
                <PluginConfirmDialog
                    request={confirm}
                    pending={confirming}
                    onCancel={() => !confirming && setConfirm(undefined)}
                    onConfirm={() => void handleConfirm()}
                />
            )}
        </>
    );

    if (selected) {
        const pluginDisabled = disabledIds.includes(selected.id); // 当前插件是否停用。
        const entries = entriesFor(selected.id); // 已注册的设置条目；脚本未到时为 undefined。
        const failure = scriptFailure(selected.id, reloaded); // 脚本失败说明；重新加载成功后不再沿用旧失败。
        const pending = !pluginDisabled && entries === undefined && !failure && !loadedIds.has(selected.id); // 启用后还在等脚本。
        return (
            <>
                <section className="plugin-settings">
                    <button className="plugin-settings__back" type="button" aria-label="返回插件" onClick={() => setSelectedId(undefined)}>
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: backIcon }} />
                    </button>
                    <h2>{selected.name}</h2>
                    <div className="plugin-settings__row">
                        <span className="plugin-settings__copy">
                            启用插件
                            <small>停用后不加载插件脚本，桌面格子显示「插件已停用」。</small>
                        </span>
                        <SettingsSwitch
                            label={`启用${selected.name}`}
                            checked={!pluginDisabled}
                            disabled={disabled || !preferences}
                            onChange={(checked) => handleEnabled(selected.id, checked)}
                        />
                    </div>
                    {pending ? (
                        <p className="plugin-settings__status" role="status" aria-busy="true">
                            正在加载插件设置…
                        </p>
                    ) : failure ? (
                        <div className="plugin-settings__error" role="alert">
                            <p>{failure}</p>
                            <button type="button" onClick={() => handleReload(selected.id, setReloaded, setLoadedIds, setShellEpoch)}>
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
                    ) : (
                        <p className="plugin-settings__empty">已安装但没有设置条目</p>
                    )}
                </section>
                {overlays}
            </>
        );
    }

    return (
        <>
            <section className="plugin-settings">
                <p className="plugin-settings__lead">插件按行列出。停用后不加载插件脚本，插件行仍然保留。</p>
                {plugins === undefined ? null : plugins.length === 0 ? (
                    <p className="plugin-settings__empty">
                        尚未安装插件
                        <small>将插件放入插件目录后重新打开程序。</small>
                    </p>
                ) : (
                    <div className="plugin-settings__list">
                        <h2 className="plugin-settings__label">已安装插件</h2>
                        {plugins.map((plugin) => {
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
                                                    loadedIds.has(plugin.id) || reloaded[plugin.id] === "ok",
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
                        })}
                    </div>
                )}
                {plugins !== undefined && stale.length > 0 && (
                    <div className="plugin-settings__stale">
                        <h2>失效插件</h2>
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
            </section>
            {overlays}
        </>
    );
};

/** 卸载或移除共用的确认请求。 */
type PluginConfirmRequest =
    { kind: "uninstall"; plugin: PluginRow; returnFocus: HTMLElement } | { kind: "remove"; instance: ComponentInstanceSnapshot; returnFocus: HTMLElement };

/** 模态确认。焦点显式落在确认按钮，不依赖对话框的默认聚焦。 */
const PluginConfirmDialog = ({
    request,
    pending,
    onCancel,
    onConfirm,
}: {
    request: PluginConfirmRequest;
    pending: boolean; // 命令进行中，两个按钮都不可用。
    onCancel: () => void;
    onConfirm: () => void;
}) => {
    const dialogRef = useRef<HTMLDialogElement>(null); // 原生对话框。
    const confirmRef = useRef<HTMLButtonElement>(null); // 预选的确认按钮。
    const uninstall = request.kind === "uninstall"; // 卸载与移除共用这一套键盘规则。
    const files = uninstall ? pluginFiles(request.plugin.id) : undefined; // 将删除的清单和脚本。

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
            aria-label={uninstall ? "确认卸载" : "确认移除"}
            onCancel={(event) => {
                event.preventDefault();
                if (!pending) onCancel();
            }}
            onKeyDown={handleKeyDown}
        >
            <header>
                <h2>{uninstall ? `卸载「${request.plugin.name}」？` : `移除「${request.instance.componentId}」？`}</h2>
            </header>
            <div className="plugin-settings__dialog-body">
                {uninstall && files ? (
                    <>
                        <ul className="plugin-settings__files">
                            <li>{files.manifest}</li>
                            <li>{files.script}</li>
                        </ul>
                        <p>只删除插件目录中的这两个文件。</p>
                        <p>{files.settings} 会保留，重装后设置会恢复。</p>
                        <p>重启后，桌面上的格子才会变为失效实例，可在「失效插件」节移除。</p>
                    </>
                ) : (
                    <p>只从桌面移除这个格子，不删除插件文件，也不删除插件设置。</p>
                )}
            </div>
            <footer className="plugin-settings__dialog-actions">
                <button type="button" disabled={pending} onClick={onCancel}>
                    取消
                </button>
                <button ref={confirmRef} type="button" className="plugin-settings__confirm" disabled={pending} onClick={onConfirm}>
                    {uninstall ? "卸载" : "移除"}
                </button>
            </footer>
        </dialog>
    );
};

/** 由插件标识得到将删除的文件和保留的设置文件。 */
const pluginFiles = (pluginId: string) => {
    const name = pluginId.startsWith("plugin.") ? pluginId.slice("plugin.".length) : pluginId; // 清单前缀后面的名称。
    return { manifest: `${pluginId}.json`, script: `${pluginId}.js`, settings: `plugins-${name}.json` };
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

/** 只要带 plugin. 前缀的清单，按显示名排序。 */
export const pluginRows = (manifests: readonly ComponentManifest[]): PluginRow[] =>
    manifests
        .filter((manifest) => manifest.id.startsWith("plugin."))
        .map((manifest) => ({ id: manifest.id, name: manifest.name }))
        .sort((left, right) => left.name.localeCompare(right.name, "zh"));

/** 已注册的条目。没有注册对象时返回 undefined，有注册但没有条目时返回空数组。 */
export const entriesFor = (pluginId: string) => {
    const api = readPluginApi(pluginId); // 首份外壳对象。
    if (!api) return undefined;
    return api.settingsEntries ?? [];
};

/** 重新加载清掉这一页记住的旧失败，再执行同一次脚本命令。 */
const handleReload = (
    pluginId: string,
    setReloaded: (update: (current: Readonly<Record<string, string>>) => Readonly<Record<string, string>>) => void,
    setLoadedIds: (update: (current: ReadonlySet<string>) => ReadonlySet<string>) => void,
    setShellEpoch: (update: (value: number) => number) => void,
) => {
    setReloaded((current) => ({ ...current, [pluginId]: "" }));
    void invokeRuntime("load_plugin_script", { pluginId })
        .then(() => {
            setLoadedIds((current) => new Set(current).add(pluginId));
            setReloaded((current) => ({ ...current, [pluginId]: "ok" }));
            setShellEpoch((value) => value + 1);
        })
        .catch((error: unknown) => setReloaded((current) => ({ ...current, [pluginId]: error instanceof Error ? error.message : "插件脚本加载失败" })));
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
    if (failure) return "设置条目加载失败";
    const entries = entriesFor(pluginId);
    if (!entries && !loaded) return "正在加载插件设置…";
    const count = entries?.length ?? 0; // 脚本已执行但没有条目。
    return count === 0 ? "没有设置条目" : `${count} 个设置条目`;
};
