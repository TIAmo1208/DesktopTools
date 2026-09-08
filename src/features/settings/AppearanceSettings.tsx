import { useEffect, useState } from "react";
import { APPLICATION_FONTS, type ApplicationPreferences } from "../../contracts/settings";
import "./AppearanceSettings.css";
import { SettingsSwitch } from "./SettingsSwitch";

/** 外观控件仅提交已完成编辑的配置，不在输入过程中持久化半成品。 */
interface AppearanceSettingsProps {
    preferences?: ApplicationPreferences; // 后端已保存的真实外观。
    disabled: boolean; // 当前是否正在保存。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交单项外观修改。
}

// 常用强调色作为直观色块，不以颜色名称代替预览。
const ACCENTS = ["#2563eb", "#0078d4", "#087f5b", "#b42318", "#9f42b5", "#986400"];

/** 主题色、圆角和字体的完整可编辑设置行。 */
export const AppearanceSettings = ({ preferences, disabled, onPreferences }: AppearanceSettingsProps) => {
    const [radius, setRadius] = useState("6"); // 尚未提交的圆角输入。
    const [fontSize, setFontSize] = useState("13"); // 尚未提交的字号输入。
    useEffect(() => {
        setRadius(String(preferences?.cornerRadius ?? 6));
        setFontSize(String(preferences?.fontSize ?? 13));
    }, [preferences, disabled]);
    const unavailable = disabled || !preferences; // 加载或保存期间阻止新提交。

    return (
        <fieldset className="appearance-settings" disabled={unavailable} aria-label="外观偏好">
            <div className="appearance-settings__row">
                <span className="appearance-settings__label">使用Windows主题色</span>
                <SettingsSwitch
                    label="使用Windows主题色"
                    checked={preferences?.useWindowsAccentColor ?? false}
                    disabled={unavailable}
                    onChange={(checked) => onPreferences({ useWindowsAccentColor: checked })}
                />
            </div>
            <div className="appearance-settings__row">
                <span className="appearance-settings__label">主题色</span>
                <div className="appearance-settings__colors">
                    {ACCENTS.map((color) => (
                        <button
                            key={color}
                            className="appearance-settings__swatch"
                            type="button"
                            disabled={preferences?.useWindowsAccentColor}
                            style={{ backgroundColor: color }}
                            title={color}
                            aria-label={`主题色 ${color}`}
                            aria-pressed={preferences?.accentColor.toLowerCase() === color}
                            onClick={() => onPreferences({ accentColor: color })}
                        />
                    ))}
                    <input
                        className="appearance-settings__color"
                        type="color"
                        disabled={preferences?.useWindowsAccentColor}
                        aria-label="自定义主题色"
                        title="自定义主题色"
                        value={preferences?.accentColor ?? "#2563eb"}
                        onChange={(event) => onPreferences({ accentColor: event.currentTarget.value })}
                    />
                </div>
            </div>
            <div className="appearance-settings__row">
                <label className="appearance-settings__label" htmlFor="appearance-radius">
                    圆角
                </label>
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
                    字体
                </label>
                <select
                    id="appearance-font"
                    className="appearance-settings__font"
                    value={preferences?.fontFamily ?? "Segoe UI Variable"}
                    onChange={(event) => onPreferences({ fontFamily: event.currentTarget.value })}
                >
                    {APPLICATION_FONTS.map((font) => (
                        <option key={font} value={font}>
                            {font}
                        </option>
                    ))}
                </select>
            </div>
            <div className="appearance-settings__row">
                <label className="appearance-settings__label" htmlFor="appearance-font-size">
                    字体大小
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
        </fieldset>
    );
};
