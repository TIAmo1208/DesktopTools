import { useState } from "react";
import type { ApplicationPreferences } from "../../contracts/settings";
import { SettingsSwitch } from "./SettingsSwitch";
import "./FeatureSettings.css";

/** 功能格子开关与二级配置，隐藏不销毁工作区实例。 */
export const FeatureSettings = ({
    preferences,
    disabled,
    debugBuild,
    onPreferences,
}: {
    preferences?: ApplicationPreferences; // 已保存的格子偏好。
    disabled: boolean; // 提交中的互斥状态。
    debugBuild: boolean; // Rust 构建提供的调试边界。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交用户意图。
}) => {
    const [detail, setDetail] = useState<"clock" | "debug">(); // 当前二级菜单。
    const unavailable = disabled || !preferences; // 加载前不允许修改。
    return (
        <section className="feature-settings">
            {detail && (
                <button className="feature-settings__back" type="button" title="返回功能格子" aria-label="返回功能格子" onClick={() => setDetail(undefined)}>
                    {"\uE72B"}
                </button>
            )}
            <h2>{detail === "clock" ? "本地时间" : detail === "debug" ? "Debug 测试" : "功能格子"}</h2>
            {!detail ? (
                <>
                    {(
                        [
                            { id: "clock", key: "clockEnabled", label: "本地时间" },
                            { id: "music", key: "musicEnabled", label: "音乐控制" },
                            ...(debugBuild ? [{ id: "debug", key: "debugEnabled", label: "Debug 测试" }] : []),
                        ] as const
                    ).map((feature) => (
                        <div className="feature-settings__row feature-settings__row--link" key={feature.id}>
                            {feature.id === "music" ? (
                                <span>{feature.label}</span>
                            ) : (
                                <button
                                    className="feature-settings__open"
                                    type="button"
                                    title={`${feature.label}设置`}
                                    aria-label={`${feature.label}设置`}
                                    onClick={() => setDetail(feature.id as "clock" | "debug")}
                                >
                                    {feature.label}
                                </button>
                            )}
                            <SettingsSwitch
                                label={feature.label}
                                disabled={unavailable}
                                checked={Boolean(preferences?.[feature.key as "clockEnabled" | "musicEnabled" | "debugEnabled"])}
                                onChange={(checked) => onPreferences({ [feature.key]: checked })}
                            />
                            {feature.id !== "music" && (
                                <span className="feature-settings__chevron" aria-hidden="true">
                                    {"\uE76C"}
                                </span>
                            )}
                        </div>
                    ))}
                    <div className="feature-settings__row">
                        <span>边缘自动对齐</span>
                        <SettingsSwitch
                            label="边缘自动对齐"
                            checked={preferences?.componentSnap ?? true}
                            disabled={unavailable}
                            onChange={(checked) => onPreferences({ componentSnap: checked })}
                        />
                    </div>
                </>
            ) : detail === "clock" ? (
                <>
                    <label className="feature-settings__row">
                        时间格式
                        <select
                            disabled={unavailable}
                            value={preferences?.clockHour12 ? "12" : "24"}
                            onChange={(event) => onPreferences({ clockHour12: event.currentTarget.value === "12" })}
                        >
                            <option value="24">24 小时制</option>
                            <option value="12">12 小时制</option>
                        </select>
                    </label>
                    <div className="feature-settings__row">
                        <span>显示秒数</span>
                        <SettingsSwitch
                            label="显示秒数"
                            disabled={unavailable}
                            checked={preferences?.clockShowSeconds ?? true}
                            onChange={(checked) => onPreferences({ clockShowSeconds: checked })}
                        />
                    </div>
                </>
            ) : (
                debugBuild && (
                    <div className="feature-settings__row">
                        <span>显示详细宿主状态</span>
                        <SettingsSwitch
                            label="显示详细宿主状态"
                            disabled={unavailable}
                            checked={preferences?.debugShowDetails ?? true}
                            onChange={(checked) => onPreferences({ debugShowDetails: checked })}
                        />
                    </div>
                )
            )}
        </section>
    );
};
