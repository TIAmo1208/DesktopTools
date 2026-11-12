import { useEffect, useState } from "react";
import { type ApplicationPreferences, type ApplicationTheme, type DesktopAppearance } from "../../contracts/settings";
import "./AppearanceSettings.css";
import { SettingsSwitch } from "./SettingsSwitch";
import { SettingsInfo } from "./SettingsInfo";
import { getComponentRuntimeSnapshot, listenComponentRuntimeSnapshots } from "../../bridge/components";
import type { ComponentManifest, ComponentRuntimeSnapshot } from "../../contracts/component";
import { settingsErrorMessage } from "../../bridge/settings";
import { publishMessage } from "../notifications/messageNotification";
import { appearanceThemeOptions } from "./appearanceThemeOptions";
import { themeFontOptions } from "./themeFonts";

/** 外观控件仅提交已完成编辑的配置，不在输入过程中持久化半成品。 */
interface AppearanceSettingsProps {
    preferences?: ApplicationPreferences; // 后端已保存的真实外观。
    disabled: boolean; // 当前是否正在保存。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交单项外观修改。
    onThemeSelection: (manifest: ComponentManifest | null) => void; // 经父页串行命令保存主题。
}

// 常用强调色作为直观色块，不以颜色名称代替预览。
const ACCENTS = ["#2563eb", "#0078d4", "#087f5b", "#b42318", "#9f42b5", "#986400"];
// 色块的提示与可访问名称用颜色名，不把 hex 当名称。
const ACCENT_NAMES: Record<string, string> = {
    "#2563eb": "蓝",
    "#0078d4": "天蓝",
    "#087f5b": "绿",
    "#b42318": "红",
    "#9f42b5": "紫",
    "#986400": "金",
};

/** 主题色、圆角和字体的完整可编辑设置行。 */
export const AppearanceSettings = ({ preferences, disabled, onPreferences, onThemeSelection }: AppearanceSettingsProps) => {
    const [manifests, setManifests] = useState<ComponentManifest[]>(); // 已确认的最新运行时清单。
    useEffect(() => {
        let active = true; // 销毁后不更新页面状态。
        let revision = -1; // 丢弃被事件超越的启动读取。
        const handleSnapshot = (snapshot: ComponentRuntimeSnapshot) => {
            if (!active || snapshot.revision < revision) return;
            revision = snapshot.revision;
            setManifests(snapshot.manifests);
        };
        const handleFailure = (error: unknown) => {
            if (active) publishMessage({ type: "error", content: settingsErrorMessage(error), source: "appearance-themes" });
        };
        const subscription = listenComponentRuntimeSnapshots(handleSnapshot, handleFailure); // 先订阅再读取，沿用已有 bridge。
        void subscription
            .then(() => getComponentRuntimeSnapshot())
            .then(handleSnapshot)
            .catch(handleFailure);
        return () => {
            active = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, []);
    const themeOptions = appearanceThemeOptions(manifests, preferences?.disabledPlugins ?? []); // 每次权威偏好变化重新过滤。
    const selectedTheme = themeOptions.find((option) => option.id === preferences?.themePluginId) ?? themeOptions[0]; // 无可用主题时显示默认，完整名称用于原生提示。
    const [radius, setRadius] = useState("6"); // 尚未提交的圆角输入。
    const [fontSize, setFontSize] = useState("13"); // 尚未提交的字号输入。
    useEffect(() => {
        setRadius(String(preferences?.cornerRadius ?? 6));
        setFontSize(String(preferences?.fontSize ?? 13));
    }, [preferences, disabled]);
    const unavailable = disabled || !preferences; // 加载或保存期间阻止新提交。
    const themeActive = Boolean(preferences?.themePluginId); // 主题生效时这几项外观由主题写入。
    const fromTheme = themeActive ? <span className="appearance-settings__from-theme">由主题设置</span> : null; // 行标签上的来源提示。

    return (
        <fieldset className="appearance-settings" disabled={unavailable} aria-label="外观偏好">
            <section className="appearance-settings__group" aria-labelledby="global-theme-title">
                <div className="appearance-settings__row">
                    <span className="appearance-settings__label appearance-settings__label--inline">
                        <label htmlFor="interface-theme-plugin">外观主题包</label>
                        <SettingsInfo
                            label="外观主题包"
                            description="主题包对整个应用生效：桌面网格、设置界面与自绘菜单。可设置字体、字号、圆角与两组强调色。切回默认会还原原有外观。"
                        />
                    </span>
                    <select
                        id="interface-theme-plugin"
                        className="appearance-settings__theme"
                        value={selectedTheme.id}
                        title={selectedTheme.name}
                        onChange={(event) => {
                            const id = event.currentTarget.value; // 空值表示恢复内置主题。
                            const manifest = manifests?.find((item) => item.id === id && item.role === "theme");
                            if (!id) onThemeSelection(null);
                            else if (manifest) onThemeSelection(manifest);
                        }}
                    >
                        {themeOptions.map((option) => (
                            <option key={option.id} value={option.id} title={option.name}>
                                {option.name}
                            </option>
                        ))}
                    </select>
                </div>
            </section>
            {(
                [
                    ["desktop", "桌面网格主题"],
                    ["interface", "界面与菜单主题"],
                ] as const
            ).map(([scope, title]) => {
                const appearance = scope === "desktop" ? preferences?.desktopAppearance : preferences; // 当前分组的已保存外观。
                const short = scope === "desktop" ? "桌面网格" : "界面菜单"; // 行标签上的作用域前缀，滚动后仍能分辨。
                /** 仅更新当前分组，保留另一组及已保存的自定义颜色。 */
                const handleAppearance = (patch: Partial<DesktopAppearance>) => {
                    if (!preferences) return;
                    onPreferences(scope === "desktop" ? { desktopAppearance: { ...preferences.desktopAppearance, ...patch } } : patch);
                };
                return (
                    <section className="appearance-settings__group" key={scope} aria-labelledby={`${scope}-appearance-title`}>
                        <h2 className="appearance-settings__heading" id={`${scope}-appearance-title`}>
                            {title}
                        </h2>
                        <div className="appearance-settings__row">
                            <label className="appearance-settings__label" htmlFor={`${scope}-theme`}>
                                {short}：深浅模式
                            </label>
                            <select
                                id={`${scope}-theme`}
                                className="appearance-settings__theme"
                                value={appearance?.theme ?? "system"}
                                onChange={(event) => handleAppearance({ theme: event.currentTarget.value as ApplicationTheme })}
                            >
                                <option value="system">跟随系统</option>
                                <option value="light">浅色</option>
                                <option value="dark">深色</option>
                            </select>
                        </div>
                        <div className="appearance-settings__row">
                            <span className="appearance-settings__label">{short}：使用 Windows 主题色</span>
                            <SettingsSwitch
                                label={`${title}：使用 Windows 主题色`}
                                checked={appearance?.useWindowsAccentColor ?? false}
                                disabled={unavailable}
                                onChange={(checked) => handleAppearance({ useWindowsAccentColor: checked })}
                            />
                        </div>
                        <div className="appearance-settings__row">
                            <span className="appearance-settings__label">{short}：强调色</span>
                            <div className="appearance-settings__colors">
                                {ACCENTS.map((color) => (
                                    <button
                                        key={color}
                                        className="appearance-settings__swatch"
                                        type="button"
                                        disabled={appearance?.useWindowsAccentColor}
                                        style={{ backgroundColor: color }}
                                        title={ACCENT_NAMES[color] ?? color}
                                        aria-label={`${short}：强调色 ${ACCENT_NAMES[color] ?? color}`}
                                        aria-pressed={appearance?.accentColor.toLowerCase() === color}
                                        onClick={() => handleAppearance({ accentColor: color })}
                                        data-button="icon"
                                    />
                                ))}
                                <input
                                    className="appearance-settings__color"
                                    type="color"
                                    disabled={appearance?.useWindowsAccentColor}
                                    aria-label={`${short}：自定义强调色`}
                                    title="自定义强调色"
                                    value={appearance?.accentColor ?? "#2563eb"}
                                    onChange={(event) => handleAppearance({ accentColor: event.currentTarget.value })}
                                />
                            </div>
                        </div>
                    </section>
                );
            })}
            {themeActive && (
                <div className="appearance-settings__theme-status" role="status">
                    <p>当前外观由「{selectedTheme.name}」决定：强调色、圆角、字体与字号由主题写入，主题生效期间的手工修改会被下一次切换覆盖。</p>
                    <button
                        type="button"
                        className="appearance-settings__restore"
                        onClick={() => onThemeSelection(null)}
                        disabled={unavailable}
                        data-button="ghost"
                    >
                        恢复我原来的外观
                    </button>
                </div>
            )}
            <section className="appearance-settings__group" aria-labelledby="shared-appearance-title">
                <h2 className="appearance-settings__heading" id="shared-appearance-title">
                    通用
                </h2>
                <div className="appearance-settings__row">
                    <label className="appearance-settings__label" htmlFor="appearance-radius">
                        圆角{fromTheme}
                    </label>{" "}
                    <input
                        id="appearance-radius"
                        className="appearance-settings__number"
                        type="number"
                        min={0}
                        max={12}
                        step={1}
                        value={radius}
                        onChange={(event) => setRadius(event.currentTarget.value)}
                        onBlur={(event) => {
                            const value = event.currentTarget.valueAsNumber; // 当前已结束编辑的圆角。
                            if (Number.isInteger(value) && value >= 0 && value <= 12) {
                                if (value !== preferences?.cornerRadius) onPreferences({ cornerRadius: value });
                            } else setRadius(String(preferences?.cornerRadius ?? 6));
                        }}
                        onKeyDown={(event) => {
                            if (event.key === "Enter") event.currentTarget.blur();
                        }}
                    />
                    <span>px</span>
                </div>
                <div className="appearance-settings__row">
                    <label className="appearance-settings__label" htmlFor="appearance-font">
                        字体{fromTheme}
                    </label>
                    <select
                        id="appearance-font"
                        className="appearance-settings__font"
                        value={preferences?.fontFamily ?? "Segoe UI Variable"}
                        onChange={(event) => onPreferences({ fontFamily: event.currentTarget.value })}
                    >
                        {themeFontOptions(manifests).map((font) => (
                            <option key={font} value={font}>
                                {font}
                            </option>
                        ))}
                    </select>
                </div>
                <div className="appearance-settings__row">
                    <label className="appearance-settings__label" htmlFor="appearance-font-size">
                        字体大小{fromTheme}
                    </label>
                    <input
                        id="appearance-font-size"
                        className="appearance-settings__number"
                        type="number"
                        min={12}
                        max={18}
                        step={1}
                        value={fontSize}
                        onChange={(event) => setFontSize(event.currentTarget.value)}
                        onBlur={(event) => {
                            const value = event.currentTarget.valueAsNumber; // 当前已结束编辑的界面字号。
                            if (Number.isInteger(value) && value >= 12 && value <= 18) {
                                if (value !== preferences?.fontSize) onPreferences({ fontSize: value });
                            } else setFontSize(String(preferences?.fontSize ?? 13));
                        }}
                        onKeyDown={(event) => {
                            if (event.key === "Enter") event.currentTarget.blur();
                        }}
                    />
                    <span>px</span>
                </div>
            </section>
        </fieldset>
    );
};
