import { useEffect, useState } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import { loadPluginScript, pluginScriptLoaded, readPluginApi, subscribePluginShell, type PluginSettingsEntry } from "../../bridge/pluginScripts";
import type { ComponentManifest } from "../../contracts/component";
import type { ApplicationPreferences } from "../../contracts/settings";
import backIcon from "../../../resources/weather-back.svg?raw";
import trashIcon from "../../../resources/weather-trash.svg?raw";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import { SettingsSwitch } from "./SettingsSwitch";
import { islandCardAvailability, moveIslandCard, sortIslandCardManifests, updateIslandCardOrder } from "./islandSettings";
import "./IslandSettingsPage.css";

/** 设置里的岛一级页与卡片二级页。 */
export const IslandSettings = ({
    preferences,
    disabled,
    onPreferences,
}: {
    preferences?: ApplicationPreferences;
    disabled: boolean;
    onPreferences: (patch: Partial<ApplicationPreferences>) => void;
}) => {
    const [manifests, setManifests] = useState<ComponentManifest[]>(); // 当前可用清单。
    const [detailId, setDetailId] = useState<string>(); // 当前卡片二级页。
    const [menu, setMenu] = useState<{ cardId: string; left: number; top: number }>(); // 公共自绘右键菜单锚点。
    const [shellEpoch, setShellEpoch] = useState(0); // 插件外壳到达后刷新设置条目。
    const unavailable = disabled || !preferences;
    const order = preferences?.islandCardOrder ?? [];
    const cards = sortIslandCardManifests(manifests ?? [], order);
    const detail = cards.find((card) => card.id === detailId);

    useEffect(() => {
        let alive = true;
        const apply = (snapshot: { manifests: ComponentManifest[] }) => alive && setManifests(snapshot.manifests);
        const subscription = listenComponentRuntimeSnapshots(apply, () => undefined);
        void getComponentRuntimeSnapshot().then(apply).catch(() => alive && setManifests([]));
        const stopShell = subscribePluginShell(() => setShellEpoch((value) => value + 1));
        return () => {
            alive = false;
            stopShell();
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);

    useEffect(() => {
        if (!detail?.id.startsWith("plugin.") || pluginScriptLoaded(detail.id) || !detail.script) return;
        void loadPluginScript(detail.id, detail.script).then(() => setShellEpoch((value) => value + 1));
    }, [detail]);

    const saveOrder = (next: string[]) => onPreferences({ islandCardOrder: next });
    const toggle = (cardId: string, enabled: boolean) => saveOrder(updateIslandCardOrder(order, cardId, enabled));
    const move = (cardId: string, direction: -1 | 1) => saveOrder(moveIslandCard(order, cardId, direction));
    const availability = detail ? islandCardAvailability(order, detail.id) : undefined;
    const menuCard = cards.find((card) => card.id === menu?.cardId);
    const menuState = menuCard ? islandCardAvailability(order, menuCard.id) : undefined;
    const menuItems: DesktopTileMenuItem[] = menuCard
        ? [
              { label: "打开卡片设置", onSelect: () => setDetailId(menuCard.id) },
              { label: "上移", disabled: !menuState?.canMoveUp, onSelect: () => move(menuCard.id, -1) },
              { label: "下移", disabled: !menuState?.canMoveDown, onSelect: () => move(menuCard.id, 1) },
              { label: "停用此卡片", separatorBefore: true, danger: true, disabled: !menuState?.enabled, onSelect: () => toggle(menuCard.id, false) },
          ]
        : [];

    if (detail) {
        void shellEpoch; // 外壳注册时触发本次读取。
        const entries = readPluginApi(detail.id)?.settingsEntries ?? [];
        return (
            <section className="island-settings island-settings--detail">
                <div className="island-settings__heading">
                    <button className="island-settings__back" type="button" aria-label="返回浮岛" title="返回浮岛" onClick={() => setDetailId(undefined)} data-button="icon">
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: backIcon }} />
                    </button>
                    <h2>{detail.name}</h2>
                    <div className="island-settings__commands">
                        <button
                            className="settings__button settings__icon-button"
                            type="button"
                            aria-label="从浮岛移除"
                            title="从浮岛移除"
                            disabled={unavailable || !availability?.enabled}
                            onClick={() => toggle(detail.id, false)}
                            data-button="icon"
                        >
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: trashIcon }} />
                        </button>
                        <SettingsSwitch label={`${detail.name}上浮岛`} checked={availability?.enabled ?? false} disabled={unavailable} onChange={(checked) => toggle(detail.id, checked)} />
                    </div>
                </div>
                <div className="island-settings__identity">
                    <strong>{detail.id}</strong>
                    <span>{detail.description || "此卡片未提供描述"}</span>
                </div>
                {entries.length ? (
                    <div className="island-settings__entries">
                        {entries.map((entry) => <IslandPluginEntry key={entry.id} pluginId={detail.id} entry={entry} />)}
                    </div>
                ) : (
                    <p className="island-settings__empty">插件未提供设置条目</p>
                )}
            </section>
        );
    }

    return (
        <section className="island-settings">
            <div className="settings__rows">
                <div className="settings__row">
                    <label className="settings__row-title" htmlFor="island-enabled">显示浮岛</label>
                    <SettingsSwitch id="island-enabled" label="显示浮岛" checked={preferences?.islandEnabled ?? false} disabled={unavailable} onChange={(checked) => onPreferences({ islandEnabled: checked })} />
                </div>
                <div className="settings__row">
                    <span className="settings__row-title">浮岛形态</span>
                    <div className="island-settings__segmented" role="tablist" aria-label="浮岛形态">
                        {(
                            [
                                ["capsule", "胶囊"],
                                ["notch", "刘海"],
                            ] as const
                        ).map(([value, label]) => (
                            <button
                                key={value}
                                type="button"
                                role="tab"
                                aria-selected={(preferences?.islandForm ?? "capsule") === value}
                                className={`island-settings__segment${(preferences?.islandForm ?? "capsule") === value ? " is-active" : ""}`}
                                disabled={unavailable}
                                onClick={() => onPreferences({ islandForm: value })}
                                data-button="primary"
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                </div>
                <div className="settings__row">
                    <span className="settings__row-title">显示位置</span>
                    <div className="island-settings__segmented" role="tablist" aria-label="显示位置">
                        {(
                            [
                                ["top", "上"],
                                ["bottom", "下"],
                                ["left", "左"],
                                ["right", "右"],
                            ] as const
                        ).map(([value, label]) => (
                            <button
                                key={value}
                                type="button"
                                role="tab"
                                aria-selected={(preferences?.islandEdge || "top") === value}
                                className={`island-settings__segment${(preferences?.islandEdge || "top") === value ? " is-active" : ""}`}
                                disabled={unavailable}
                                onClick={() => onPreferences({ islandEdge: value, islandOffset: 0 })}
                                data-button="primary"
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                </div>
                <div className="settings__row">
                    <label className="settings__row-title" htmlFor="island-hide-fullscreen">全屏时隐藏</label>
                    <SettingsSwitch id="island-hide-fullscreen" label="全屏时隐藏" checked={preferences?.islandHideOnFullscreen ?? true} disabled={unavailable} onChange={(checked) => onPreferences({ islandHideOnFullscreen: checked })} />
                </div>
                <label className="settings__row island-settings__radius">
                    <span className="settings__row-title">浮岛圆角（展开卡）</span>
                    <input
                        type="number"
                        min={0}
                        max={16}
                        step={1}
                        value={preferences?.islandRadius ?? 12}
                        disabled={unavailable}
                        onChange={(event) => {
                            const value = Number(event.currentTarget.value);
                            if (Number.isInteger(value) && value >= 0 && value <= 16) onPreferences({ islandRadius: value });
                        }}
                    />
                </label>
            </div>
            <h2 className="island-settings__label">卡片</h2>
            {!manifests ? (
                <p className="island-settings__empty" role="status">正在读取浮岛卡…</p>
            ) : cards.length === 0 ? (
                <p className="island-settings__empty">暂无可用浮岛卡</p>
            ) : (
                <div className="island-settings__list">
                    {cards.map((card) => {
                        const state = islandCardAvailability(order, card.id);
                        return (
                            <div
                                className="island-settings__row"
                                key={card.id}
                                onContextMenu={(event) => {
                                    event.preventDefault();
                                    setMenu({ cardId: card.id, left: event.clientX, top: event.clientY });
                                }}
                            >
                                <span className="island-settings__copy">
                                    <strong>{card.name}</strong>
                                    <small>{card.description || card.id}</small>
                                </span>
                                <div className="island-settings__controls">
                                    <SettingsSwitch label={`${card.name}上浮岛`} checked={state.enabled} disabled={unavailable} onChange={(checked) => toggle(card.id, checked)} />
                                    <button type="button" aria-label={`${card.name}上移`} title="上移" disabled={unavailable || !state.canMoveUp} onClick={() => move(card.id, -1)} data-button="icon">↑</button>
                                    <button type="button" aria-label={`${card.name}下移`} title="下移" disabled={unavailable || !state.canMoveDown} onClick={() => move(card.id, 1)} data-button="icon">↓</button>
                                </div>
                                <span className="island-settings__chevron" aria-hidden="true">›</span>
                                <button className="island-settings__hit" type="button" aria-label={`打开${card.name}卡片设置`} onClick={() => setDetailId(card.id)} data-button="row-hit" />
                            </div>
                        );
                    })}
                </div>
            )}
            {menu && menuItems.length > 0 && <DesktopTileMenu items={menuItems} left={menu.left} top={menu.top} onClose={() => setMenu(undefined)} />}
        </section>
    );
};

/** 插件自有设置条目直接复用既有作者渲染契约。 */
const IslandPluginEntry = ({ pluginId, entry }: { pluginId: string; entry: PluginSettingsEntry }) => {
    const [host, setHost] = useState<HTMLDivElement | null>(null);
    const [settings, setSettings] = useState<unknown>(null);

    useEffect(() => {
        let alive = true;
        const read = () => void getPluginSettings(pluginId).then((value) => alive && setSettings(value ?? null)).catch(() => alive && setSettings(null));
        read();
        const subscription = listenPluginSettings((changedId) => changedId === pluginId && read(), () => undefined);
        return () => {
            alive = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [pluginId]);

    useEffect(() => {
        if (!host || typeof entry.render !== "function") return;
        host.replaceChildren();
        const cleanup = entry.render(host, { pluginId, instanceId: "", tileType: "", collapsed: false, settings, saveSettings: (value) => savePluginSettings(pluginId, value) });
        return () => { if (typeof cleanup === "function") cleanup(); };
    }, [entry, host, pluginId, settings]);

    return (
        <div className="island-settings__entry">
            <h3>{entry.name}</h3>
            <div ref={setHost} />
        </div>
    );
};
