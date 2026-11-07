import { useEffect, useState } from "react";

import "../../styles/global.css";
import applicationIcon from "../../../resources/application.png";
import chevronIcon from "../../../resources/weather-chevron-right.svg?raw";
import infoIcon from "../../../resources/info.svg?raw";
import trashIcon from "../../../resources/weather-trash.svg?raw";
import { SettingsSwitch } from "./SettingsSwitch";
import "./SettingsSamples.css";
import "./SettingsPage.css";
import "./SettingsSwitch.css";
import "./PluginSettingsPage.css";
import "./AppearanceSettings.css";
import "./WeatherSettings.css";

/** 导航项只用于展示页面骨架，不接产品路由。 */
const NAV_ITEMS = ["常规", "外观", "功能格子", "插件", "性能", "诊断与维护", "关于"];

/**
 * 设置界面样张：用真实设置页类名、令牌和 SettingsSwitch 拼出各类行与状态。
 * 仅供浏览器对照，不接入设置页逻辑或宿主桥。
 */
export const SettingsSamples = () => {
    const [theme, setTheme] = useState<"light" | "dark">(() => {
        const requested = new URLSearchParams(window.location.search).get("theme");
        if (requested === "light" || requested === "dark") return requested;
        return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
    });
    const [switchOn, setSwitchOn] = useState(true);
    const [switchOff, setSwitchOff] = useState(false);

    useEffect(() => {
        document.documentElement.dataset.theme = theme;
    }, [theme]);

    return (
        <div className="settings">
            <header className="settings__titlebar">
                <div className="settings__window-controls" aria-hidden="true">
                    <button type="button" tabIndex={-1}>{"\uE921"}</button>
                    <button type="button" tabIndex={-1}>{"\uE922"}</button>
                    <button type="button" tabIndex={-1}>{"\uE8BB"}</button>
                </div>
            </header>
            <aside className="settings__sidebar">
                <div className="settings__brand">
                    <img src={applicationIcon} alt="" />
                    <span>Desktop Tools</span>
                </div>
                <nav className="settings__navigation" aria-label="设置分类">
                    {NAV_ITEMS.map((label, index) => (
                        <button
                            key={label}
                            className="settings__navigation-item"
                            type="button"
                            aria-current={index === 0 ? "page" : undefined}
                        >
                            <span className="settings__icon" aria-hidden="true">
                                {index === 0 ? "\uE80F" : "\uE8B7"}
                            </span>
                            <span className="settings__navigation-label">{label}</span>
                        </button>
                    ))}
                </nav>
            </aside>
            <main className="settings__content">
                <div className="settings__heading">
                    <button className="settings__back" type="button" title="返回" aria-label="返回" data-button="icon">
                        {"\uE72B"}
                    </button>
                    <h1>设置界面样张</h1>
                </div>

                <div className="settings-samples__toolbar">
                    <span className="settings-samples__toolbar-label">主题</span>
                    <button
                        className="settings__button"
                        type="button"
                        data-button={theme === "light" ? "primary" : "secondary"}
                        aria-pressed={theme === "light"}
                        onClick={() => setTheme("light")}
                    >
                        浅色
                    </button>
                    <button
                        className="settings__button"
                        type="button"
                        data-button={theme === "dark" ? "primary" : "secondary"}
                        aria-pressed={theme === "dark"}
                        onClick={() => setTheme("dark")}
                    >
                        深色
                    </button>
                    <span className="settings-samples__toolbar-hint">
                        悬停、键盘焦点为真实交互态：把指针移入控件或按 Tab；禁用、错误态常显。
                    </span>
                </div>

                <section className="settings-samples__section" aria-labelledby="samples-rows">
                    <h2 id="samples-rows" className="settings-samples__heading">设置行</h2>
                    <div className="settings__rows">
                        <div className="settings__row">
                            <label className="settings__row-title" htmlFor="sample-switch">开关行（默认 / 已开启）</label>
                            <SettingsSwitch
                                id="sample-switch"
                                label="开关行示例"
                                checked={switchOn}
                                disabled={false}
                                onChange={setSwitchOn}
                            />
                        </div>
                        <div className="settings__row">
                            <label className="settings__row-title" htmlFor="sample-switch-empty">开关行（关闭）</label>
                            <SettingsSwitch
                                id="sample-switch-empty"
                                label="关闭开关示例"
                                checked={switchOff}
                                disabled={false}
                                onChange={setSwitchOff}
                            />
                        </div>
                        <div className="settings__row">
                            <label className="settings__row-title" htmlFor="sample-switch-disabled">开关行（禁用）</label>
                            <SettingsSwitch
                                id="sample-switch-disabled"
                                label="禁用开关示例"
                                checked
                                disabled
                                onChange={() => undefined}
                            />
                        </div>
                        <div className="settings__row settings__row--link">
                            <span className="settings__row-title">入口行（整行可点击）</span>
                            <span className="settings__value">打开二级设置</span>
                            <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                        </div>
                        <div className="plugin-settings__row plugin-settings__row--link">
                            <span className="plugin-settings__copy">
                                AstrBot 状态
                                <small>插件条目行 · 已启用</small>
                            </span>
                            <button className="plugin-settings__uninstall" type="button" aria-label="卸载示例插件" title="卸载" data-button="icon">
                                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: trashIcon }} />
                            </button>
                            <SettingsSwitch label="启用示例插件" checked disabled={false} onChange={() => undefined} />
                            <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                            <button className="plugin-settings__hit" type="button" data-button="row-hit" aria-label="插件设置" />
                        </div>
                        <div className="plugin-settings__row plugin-settings__row--link">
                            <span className="plugin-settings__copy">
                                天气
                                <small>插件条目行 · 已停用</small>
                            </span>
                            <SettingsSwitch label="启用示例停用插件" checked={false} disabled onChange={() => undefined} />
                            <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                            <button className="plugin-settings__hit" type="button" data-button="row-hit" aria-label="停用插件设置" />
                        </div>
                    </div>
                </section>

                <section className="settings-samples__section" aria-labelledby="samples-inputs">
                    <h2 id="samples-inputs" className="settings-samples__heading">输入控件</h2>
                    <div className="weather-settings">
                        <div className="settings__rows">
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-text">文本输入行</label>
                                <input id="sample-text" type="text" defaultValue="https://example.com/api" />
                            </div>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-text-focus">文本输入行（焦点）</label>
                                <input id="sample-text-focus" type="text" defaultValue="按 Tab 聚焦" />
                            </div>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-text-disabled">文本输入行（禁用）</label>
                                <input id="sample-text-disabled" type="text" defaultValue="不可编辑" disabled />
                            </div>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-text-error">文本输入行（错误）</label>
                                <input id="sample-text-error" type="text" defaultValue="无效地址" aria-invalid="true" aria-describedby="sample-text-error-message" />
                            </div>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-select">下拉选择行</label>
                                <select id="sample-select" className="settings__select" defaultValue="dark">
                                    <option value="light">浅色</option>
                                    <option value="dark">深色</option>
                                    <option value="system">跟随系统</option>
                                </select>
                            </div>
                            <div className="settings__row">
                                <label className="settings__row-title" htmlFor="sample-select-disabled">下拉选择行（禁用）</label>
                                <select id="sample-select-disabled" className="settings__select" disabled defaultValue="dark">
                                    <option value="dark">深色</option>
                                </select>
                            </div>
                            <div className="settings__row settings-samples__row--textarea">
                                <label className="settings__row-title" htmlFor="sample-textarea">多行输入行</label>
                                <div className="weather-settings__secret-input">
                                    <textarea id="sample-textarea" defaultValue="第一行说明&#10;第二行说明" />
                                </div>
                            </div>
                            <div className="settings__row settings__directory">
                                <span className="settings__row-title" title="C:\\Users\\TIAmo\\AppData\\Roaming\\TIAmo\\DesktopTools">
                                    目录行（配置目录）
                                    <span className="settings__directory-path">C:\\Users\\TIAmo\\AppData\\Roaming\\TIAmo\\DesktopTools</span>
                                </span>
                                <button className="settings__button settings__icon-button" type="button" title="复制目录" aria-label="复制目录" data-button="icon">
                                    <span className="settings__directory-icon settings__directory-icon--copy" aria-hidden="true" />
                                </button>
                                <button className="settings__button settings__icon-button" type="button" title="选择目录" aria-label="选择目录" data-button="icon">
                                    <span className="settings__directory-icon settings__directory-icon--folder" aria-hidden="true" />
                                </button>
                            </div>
                        </div>
                    </div>
                </section>

                <section className="settings-samples__section" aria-labelledby="samples-plugin-entry">
                    <h2 id="samples-plugin-entry" className="settings-samples__heading">插件条目与分组</h2>
                    <div className="appearance-settings__group">
                        <h2 className="appearance-settings__heading">分组标题 · 外观</h2>
                    </div>
                    <div className="plugin-settings__entries">
                        <div className="plugin-settings__entry">
                            <button className="plugin-settings__entry-head" type="button" aria-expanded="true" data-button="row-hit">
                                连接与版本
                                <span className="plugin-settings__chevron plugin-settings__chevron--open" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                            </button>
                            <div className="plugin-settings__entry-body">
                                <div className="settings__rows">
                                    <div className="settings__row">
                                        <span className="settings__row-title">插件条目行（展开）</span>
                                        <span className="settings__value">4.5.0</span>
                                    </div>
                                </div>
                            </div>
                        </div>
                        <div className="plugin-settings__entry plugin-settings__entry--locked">
                            <button className="plugin-settings__entry-head" type="button" aria-expanded="false" disabled>
                                条目行（禁用 / 停用插件）
                                <span className="plugin-settings__chevron" aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                            </button>
                        </div>
                    </div>
                </section>

                <section className="settings-samples__section" aria-labelledby="samples-feedback">
                    <h2 id="samples-feedback" className="settings-samples__heading">说明、警示与命令</h2>
                    <div className="plugin-settings__notice" role="note">
                        <span className="plugin-settings__notice-icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: infoIcon }} />
                        <p>说明行：插件在本机以你的权限运行，请确认来源可信。</p>
                    </div>
                    <div className="plugin-settings__notice" role="alert">
                        <span className="plugin-settings__notice-icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: infoIcon }} />
                        <p>警示行：修改目录后需要重启程序；未保存的草稿会保留在当前窗口。</p>
                    </div>
                    <div className="settings__error" role="alert">
                        <span id="sample-text-error-message">错误状态：目录不可用，请检查路径权限。</span>
                        <button className="settings__button" type="button" data-button="secondary">重试</button>
                    </div>
                    <div className="settings__rows">
                        <div className="settings__row">
                            <span className="settings__row-title">按钮命令行</span>
                            <button className="settings__button" type="button" data-button="primary">主命令</button>
                            <button className="settings__button" type="button" data-button="secondary">次命令</button>
                            <button className="settings__button" type="button" data-button="ghost">文本命令</button>
                            <button className="settings__button" type="button" disabled>禁用</button>
                        </div>
                        <div className="settings__row">
                            <span className="settings__row-title">危险行（删除 / 移除）</span>
                            <button className="plugin-settings__remove" type="button" data-button="danger">移除插件</button>
                        </div>
                        <p className="settings__status" role="status">状态行：正在读取设置…</p>
                    </div>
                </section>
            </main>
        </div>
    );
};
