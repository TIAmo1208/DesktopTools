import { useEffect, useRef, useState } from "react";

import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../../bridge/components";
import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../../bridge/pluginSettings";
import { loadPluginScript, pluginScriptLoaded, readPluginApi, subscribePluginShell, type PluginSettingsEntry } from "../../../bridge/pluginScripts";
import type { ComponentManifest } from "../../../contracts/component";
import { islandAnchorVisible, type ApplicationPreferences, type IslandAnchorKey } from "../../../contracts/settings";
import backIcon from "../../../../resources/weather-back.svg?raw";
import chevronIcon from "../../../../resources/weather-chevron-right.svg?raw";
import trashIcon from "../../../../resources/weather-trash.svg?raw";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import { SettingsAbout } from "../ui";
import { SettingsSwitch } from "../ui";
import { islandCardAvailability, moveIslandCard, sortIslandCardManifests, updateIslandCardOrder } from "./islandSettings";
import { IslandWeatherSettings } from "./IslandWeatherSettings";
import "./IslandSettingsPage.css";

/** 现成的垃圾桶图标缩到页头命令尺寸，与插件卸载按钮同一视觉。 */
const removeIcon = trashIcon.replace("<svg ", '<svg width="18" height="18" ');

/** 锚点显示二级页的三枚开关。胶囊与 hover 共用，标签即无障碍名。 */
const ANCHOR_ROWS = [
    ["music", "音乐上浮岛显示"],
    ["message", "消息通知上浮岛显示"],
    ["plugin", "插件卡上浮岛显示"],
] as const satisfies readonly (readonly [IslandAnchorKey, string])[];

/** 设置里的岛一级页、锚点显示二级页与卡片二级页。 */
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
    const [anchorPage, setAnchorPage] = useState(false); // 锚点显示二级页。与卡片详情互不叠放。
    const [menu, setMenu] = useState<{ cardId: string; left: number; top: number }>(); // 公共自绘右键菜单锚点。
    const [shellEpoch, setShellEpoch] = useState(0); // 插件外壳到达后刷新设置条目。
    const unavailable = disabled || !preferences;
    const order = preferences?.islandCardOrder ?? [];
    const cards = sortIslandCardManifests(manifests ?? [], order);
    const detail = cards.find((card) => card.id === detailId);
    // 返回一级页后把焦点放回进入二级页的入口；停留在二级页时不动，入口行已删除时不强制。
    const returnFocusSelector = useRef<string | undefined>(undefined);
    const recordReturnFocus = (selector: string) => {
        returnFocusSelector.current = selector;
    };
    useEffect(() => {
        if (!returnFocusSelector.current || anchorPage || detailId) return;
        const target = document.querySelector<HTMLButtonElement>(returnFocusSelector.current);
        returnFocusSelector.current = undefined;
        target?.focus();
    }, [anchorPage, detailId]);

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
    // 只改被拨动的键，其余已保存键原样保留；缺键仍由读取函数给默认值。
    const saveAnchor = (key: IslandAnchorKey, checked: boolean) =>
        onPreferences({ islandAnchorVisible: { ...(preferences?.islandAnchorVisible ?? {}), [key]: checked } });
    const toggle = (cardId: string, enabled: boolean) => saveOrder(updateIslandCardOrder(order, cardId, enabled));
    const move = (cardId: string, direction: -1 | 1) => saveOrder(moveIslandCard(order, cardId, direction));
    const availability = detail ? islandCardAvailability(order, detail.id) : undefined;
    const menuCard = cards.find((card) => card.id === menu?.cardId);
    const menuState = menuCard ? islandCardAvailability(order, menuCard.id) : undefined;
    const menuItems: DesktopTileMenuItem[] = menuCard
        ? [
              { label: "打开卡片设置", onSelect: () => { recordReturnFocus(`[aria-label="打开${menuCard.name}卡片设置"]`); setDetailId(menuCard.id); } },
              { label: "上移", disabled: !menuState?.canMoveUp, onSelect: () => move(menuCard.id, -1) },
              { label: "下移", disabled: !menuState?.canMoveDown, onSelect: () => move(menuCard.id, 1) },
              { label: "停用此卡片", separatorBefore: true, danger: true, disabled: !menuState?.enabled, onSelect: () => toggle(menuCard.id, false) },
          ]
        : [];

    if (anchorPage) {
        return (
            <section className="island-settings island-settings--detail">
                <div className="island-settings__heading">
                    <button className="island-settings__back" type="button" aria-label="返回浮岛" title="返回浮岛" onClick={() => setAnchorPage(false)} data-button="icon">
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: backIcon }} />
                    </button>
                    <div className="island-settings__identity">
                        <h2>锚点显示</h2>
                    </div>
                </div>
                <div className="settings__rows">
                    {ANCHOR_ROWS.map(([key, label]) => (
                        <div className="settings__row" key={key}>
                            <label className="settings__row-title" htmlFor={`island-anchor-${key}`}>{label}</label>
                            <SettingsSwitch
                                id={`island-anchor-${key}`}
                                label={label}
                                checked={islandAnchorVisible(preferences, key)}
                                disabled={unavailable}
                                onChange={(checked) => saveAnchor(key, checked)}
                            />
                        </div>
                    ))}
                </div>
                <p className="island-settings__hint">控制该功能锚点在胶囊与 hover 同时出现或撤下；与下方「卡片」列表的上浮岛开关不是同一件事，展开卡仍按卡片开关。</p>
            </section>
        );
    }

    if (detail) {
        void shellEpoch; // 外壳注册时触发本次读取。
        const entries = readPluginApi(detail.id)?.settingsEntries ?? [];
        return (
            <section className="island-settings island-settings--detail">
                <div className="island-settings__heading">
                    <button className="island-settings__back" type="button" aria-label="返回浮岛" title="返回浮岛" onClick={() => setDetailId(undefined)} data-button="icon">
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: backIcon }} />
                    </button>
                    <div className="island-settings__identity">
                        <h2>{detail.name}</h2>
                        <p className="island-settings__meta">{availability?.enabled ? "已启用" : "已停用"}</p>
                    </div>
                    <div className="island-settings__commands">
                        {detail.id.startsWith("plugin.") ? (
                            <button
                                className="settings__button settings__icon-button island-settings__trash"
                                type="button"
                                title="卸载卡片"
                                aria-label="卸载卡片"
                                disabled={unavailable || !availability?.enabled}
                                onClick={() => toggle(detail.id, false)}
                                data-button="icon"
                            >
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: removeIcon }} />
                            </button>
                        ) : null}
                        <SettingsSwitch label={`${detail.name}上浮岛`} checked={availability?.enabled ?? false} disabled={unavailable} onChange={(checked) => toggle(detail.id, checked)} />
                    </div>
                </div>
                <SettingsAbout text={detail.description || ""} />
                <div className="settings__rows">
                    <div className="settings__row">
                        <label className="settings__row-title" htmlFor={`island-collapse-memory-${detail.id}`}>
                            记忆收起状态
                        </label>
                        <SettingsSwitch
                            id={`island-collapse-memory-${detail.id}`}
                            label="记忆收起状态"
                            checked={(preferences?.islandCardCollapseMemory ?? {})[detail.id] !== false}
                            disabled={unavailable || !preferences}
                            onChange={(checked) =>
                                onPreferences({ islandCardCollapseMemory: { ...(preferences?.islandCardCollapseMemory ?? {}), [detail.id]: checked } })
                            }
                        />
                    </div>
                    {detail.id === "official.collect-box" ? (
                        <>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="island-collect-collapsed-default">
                                    默认收起
                                </label>
                                <SettingsSwitch
                                    id="island-collect-collapsed-default"
                                    label="默认收起"
                                    checked={preferences?.islandCollectCollapsedDefault ?? true}
                                    disabled={unavailable || !preferences}
                                    onChange={(checked) => onPreferences({ islandCollectCollapsedDefault: checked })}
                                />
                            </div>
                            <div className="settings__row">
                                <span className="settings__row-title">收起时显示条数</span>
                                <div className="island-settings__segmented" role="tablist" aria-label="收起时显示条数">
                                    {[3, 5, 10].map((count) => (
                                        <button
                                            key={count}
                                            type="button"
                                            role="tab"
                                            aria-selected={(preferences?.islandCollectCollapsedCount ?? 3) === count}
                                            className={`island-settings__segment${(preferences?.islandCollectCollapsedCount ?? 3) === count ? " is-active" : ""}`}
                                            disabled={unavailable || !preferences}
                                            onClick={() => onPreferences({ islandCollectCollapsedCount: count })}
                                            data-button="primary"
                                        >
                                            {count}
                                        </button>
                                    ))}
                                </div>
                            </div>
                        </>
                    ) : null}
                </div>
                {detail.id === "official.music-control" ? (
                    <p className="island-settings__empty">歌词与频谱在「功能格子 → 音乐控制」中设置。</p>
                ) : null}
                {detail.id === "official.weather" ? <IslandWeatherSettings preferences={preferences} disabled={unavailable} onPreferences={onPreferences} /> : null}
                {entries.length ? (
                    <div className="island-settings__entries">
                        {entries.map((entry) => <IslandPluginEntry key={entry.id} pluginId={detail.id} entry={entry} />)}
                    </div>
                ) : detail.id === "official.music-control" || detail.id === "official.weather" ? null : detail.id === "official.collect-box" ? (
                    <p className="island-settings__empty">把文本、链接、图片或文件拖到浮岛上即可收入；展开后单击条目打开。</p>
                ) : (
                    <p className="island-settings__empty">插件未提供设置条目</p>
                )}
            </section>
        );
    }

    return (
        <section className="island-settings">
            <div className="settings__rows">
                <section className="settings__group" aria-labelledby="island-look-title">
                <h2 className="settings__group-title" id="island-look-title">浮岛外观</h2>
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
                </section>
                <section className="settings__group" aria-labelledby="island-behavior-title">
                    <h2 className="settings__group-title" id="island-behavior-title">行为</h2>
                    <div className="settings__row">
                        <label className="settings__row-title" htmlFor="island-hide-fullscreen">全屏时隐藏</label>
                        <SettingsSwitch id="island-hide-fullscreen" label="全屏时隐藏" checked={preferences?.islandHideOnFullscreen ?? true} disabled={unavailable} onChange={(checked) => onPreferences({ islandHideOnFullscreen: checked })} />
                    </div>
                </section>
                <section className="settings__group" aria-labelledby="island-anchor-entry-title">
                    <h2 className="settings__group-title" id="island-anchor-entry-title">锚点显示</h2>
                    <button className="settings__row settings__row--link" type="button" onClick={() => { recordReturnFocus('section[aria-labelledby="island-anchor-entry-title"] button[data-button="row-hit"]'); setAnchorPage(true); }} data-button="row-hit">
                        <span className="settings__row-title">锚点显示</span>
                        <span className="settings__icon settings__icon--svg" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                    </button>
                </section>
            </div>
            <h2 className="island-settings__label" id="island-card-title">卡片</h2>
            {!manifests ? (
                <p className="island-settings__empty" role="status">正在读取浮岛卡…</p>
            ) : cards.length === 0 ? (
                <p className="island-settings__empty">暂无可用浮岛卡</p>
            ) : (
                <ul className="island-settings__list settings__list" aria-labelledby="island-card-title">
                    {cards.map((card) => {
                        const state = islandCardAvailability(order, card.id);
                        return (
                            <li
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
                                <span className="island-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                                <button
                                    className="island-settings__hit"
                                    type="button"
                                    aria-label={`打开${card.name}卡片设置`}
                                    onClick={() => {
                                        recordReturnFocus(`[aria-label="打开${card.name}卡片设置"]`);
                                        setDetailId(card.id);
                                    }}
                                    data-button="row-hit"
                                />
                            </li>
                        );
                    })}
                </ul>
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
