import { useEffect, useRef } from "react";

import { WeatherCityDialog } from "../../weather/WeatherCityDialog";
import { DesktopTileMenu, type DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import { SettingsCityList } from "./SettingsCityList";
import { SettingsInfo } from "../ui";
import chevronRightIcon from "../../../../resources/weather-chevron-right.svg?raw";
import closeIcon from "../../../../resources/weather-close.svg?raw";
import eyeIcon from "../../../../resources/weather-eye.svg?raw";
import eyeOffIcon from "../../../../resources/weather-eye-off.svg?raw";
import { WEATHER_SOURCES, type WeatherSource } from "../../../contracts/weather";
import { AVAILABILITY, WEATHER_FIELDS } from "./weatherSettingsFlow";
import { useWeatherSettings, type WeatherSettingsOptions } from "./useWeatherSettings";
import "./WeatherSettings.css";

/** 图标资源以原始 SVG 保存，尺寸与颜色由所在按键的样式决定。 */
const Glyph = ({ source }: { source: string }) => <span className="weather-settings__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: source }} />;

/** 天气来源列表、右键菜单与添加/编辑弹窗；领域状态与流程在 useWeatherSettings，组件只做绑定。 */
export const WeatherSettings = ({ leaveGuard, onStatus, onBusy, openAddKey }: WeatherSettingsOptions) => {
    const {
        sources,
        busy,
        cityBusy,
        status,
        setStatus,
        failure,
        setFailure,
        menu,
        setMenu,
        cityMenu,
        setCityMenu,
        form,
        removal,
        setRemoval,
        draft,
        setDraft,
        visible,
        setVisible,
        expandedLoading,
        city,
        setCity,
        checks,
        setChecks,
        picker,
        setPicker,
        expanded,
        formReady,
        formCities,
        confirmText,
        openPicker,
        openForm,
        closeForm,
        handleDraft,
        handleProbe,
        handleSearch,
        handleSelect,
        handleValidate,
        handleConfirm,
        handleSave,
        handleRemove,
        handleExpand,
        requestCityRemoval,
        openCityMenu,
        handleRemoveCity,
        handleCreateTile,
        switchSource,
        importPrivateKeyFile,
        cancelDiscard,
        discardAndLeave,
    } = useWeatherSettings({ leaveGuard, onStatus, onBusy, openAddKey });
    const formDialog = useRef<HTMLDialogElement>(null); // 添加与编辑弹窗元素。
    const confirmDialog = useRef<HTMLDialogElement>(null); // 确认框元素。
    const file = useRef<HTMLInputElement>(null); // 本地私钥导入入口。

    const renderCityMenu = () =>
        cityMenu ? (
            <DesktopTileMenu
                items={[{ label: "删除", danger: true, disabled: !!busy, onSelect: () => setRemoval({ kind: "city", source: cityMenu.source, city: cityMenu.city }) }]}
                left={cityMenu.left}
                top={cityMenu.top}
                onClose={() => setCityMenu(undefined)}
            />
        ) : null;

    useEffect(() => {
        for (const [element, open] of [
            [formDialog.current, !!form],
            [confirmDialog.current, !!removal],
        ] as const) {
            if (!element) continue;
            if (open && !element.open) element.showModal(); // 城市弹窗在编辑弹窗之上独立打开。
            if (!open && element.open) element.close();
        }
    }, [form, removal]);

    return (
        <section className="weather-settings">
            <h3>已配置的天气来源</h3>
            {sources.length ? (
                <ul className="weather-settings__list">
                    {sources.map((item) => (
                        <li key={item.source} className="weather-settings__item">
                            <div
                                className="weather-settings__item-row"
                                onContextMenu={(event) => {
                                    event.preventDefault();
                                    setMenu({
                                        source: item.source,
                                        left: Math.min(event.clientX, window.innerWidth - 240),
                                        top: Math.min(event.clientY, window.innerHeight - 132),
                                    });
                                }}
                            >
                                {/* 整行命中层：行只作容器，指针与键盘的进入动作都落在这一层。 */}
                                <button
                                    type="button"
                                    className="weather-settings__item-hit"
                                    title="显示详情"
                                    aria-label={`打开 ${WEATHER_SOURCES[item.source].name} 的天气来源设置`}
                                    onClick={() => openForm(item.source, false)}
                                    data-button="row-hit"
                                />
                                <span className="weather-settings__item-main">
                                    <span>{WEATHER_SOURCES[item.source].name}</span>
                                    <small>{item.address}</small>
                                </span>
                                <span className="weather-settings__mode">{WEATHER_SOURCES[item.source].mode}</span>
                                <span className={`weather-settings__state is-${item.availability}`}>
                                    <i aria-hidden="true" />
                                    {AVAILABILITY[item.availability]}
                                </span>
                                {(() => {
                                    const open = expanded === item.source; // 该项是否已展开城市列表。
                                    return (
                                        <button
                                            type="button"
                                            className={`weather-settings__expand${open ? " is-open" : ""}`}
                                            aria-expanded={open}
                                            aria-label={`${open ? "收起" : "展开"} ${WEATHER_SOURCES[item.source].name} 的城市列表`}
                                            title={`${open ? "收起" : "展开"}城市列表`}
                                            onClick={(event) => {
                                                event.stopPropagation(); // 展开按键只负责展开收起，不触发进入编辑。
                                                void handleExpand(item.source);
                                            }}
                                         data-button="icon">
                                            <Glyph source={chevronRightIcon} />
                                        </button>
                                    );
                                })()}
                                <span className="weather-settings__arrow" aria-hidden="true">
                                    <Glyph source={chevronRightIcon} />
                                </span>
                            </div>
                            {expanded === item.source && (
                                <div className="weather-settings__cities">
                                    {expandedLoading ? (
                                        <p className="weather-settings__status">正在读取城市列表…</p>
                                    ) : (
                                        <SettingsCityList
                                            cities={item.cities}
                                            busy={!!busy}
                                            onDelete={(target) => requestCityRemoval(item.source, target)}
                                            onMenu={(target, position) => openCityMenu(item.source, target, position)}
                                            onAdd={() => openPicker(item.source)}
                                        />
                                    )}
                                </div>
                            )}
                        </li>
                    ))}
                </ul>
            ) : (
                <p className="weather-settings__empty">尚未添加任何天气来源</p>
            )}
            {!form && (
                <p className={failure ? "weather-settings__error" : "weather-settings__status"} role="status">
                    {status}
                </p>
            )}
            {menu &&
                (() => {
                    const cities = sources.find((item) => item.source === menu.source)?.cities ?? []; // 该来源已添加的城市。
                    const items: DesktopTileMenuItem[] = [
                        { label: "显示详情", onSelect: () => openForm(menu.source, false) },
                        { label: "检测可用", disabled: !!busy, onSelect: () => void handleProbe(menu.source) },
                        { label: "添加城市", separatorBefore: true, disabled: !!busy, onSelect: () => openPicker(menu.source) },
                        {
                            label: "添加天气格子",
                            children: cities.length
                                ? cities.map((item) => ({ label: item.name, onSelect: () => handleCreateTile(menu.source, item) }))
                                : [{ label: "列表为空", heading: true }],
                        },
                        { label: "删除", separatorBefore: true, danger: true, onSelect: () => setRemoval({ kind: "source", source: menu.source }) },
                    ];
                    return <DesktopTileMenu items={items} left={menu.left} top={menu.top} onClose={() => setMenu(undefined)} />;
                })()}
            {!form && renderCityMenu()}
            {form && (
                <dialog
                    ref={formDialog}
                    className="weather-settings__dialog"
                    aria-label={form.adding ? "添加 API" : `编辑 ${WEATHER_SOURCES[form.source].name}`}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy !== "save") closeForm();
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>{form.adding ? "添加 API" : `编辑 ${WEATHER_SOURCES[form.source].name}`}</h2>
                        <button
                            type="button"
                            className="weather-settings__close"
                            aria-label="关闭"
                            title="关闭"
                            disabled={busy === "save"}
                            onClick={() => closeForm()}
                         data-button="icon">
                            <Glyph source={closeIcon} />
                        </button>
                    </header>
                    <div className="weather-settings__dialog-body">
                        {form.adding && (
                            <div className="weather-settings__row">
                                <span>供应商</span>
                                <div className="weather-settings__segmented" role="tablist" aria-label="供应商">
                                    {(Object.keys(WEATHER_SOURCES) as WeatherSource[]).map((item) => (
                                        <button
                                            key={item}
                                            type="button"
                                            role="tab"
                                            aria-selected={item === form.source}
                                            className={`weather-settings__segment${item === form.source ? " is-active" : ""}`}
                                            disabled={!!busy}
                                            onClick={() => switchSource(item)}
                                         data-button="primary">
                                            {WEATHER_SOURCES[item].name}
                                        </button>
                                    ))}
                                </div>
                            </div>
                        )}
                        {WEATHER_SOURCES[form.source].fields.map((key) =>
                            WEATHER_FIELDS[key].multiline ? (
                                <div className="weather-settings__row weather-settings__row--secret" key={key}>
                                    <span>
                                        <label htmlFor={`weather-${key}`}>{WEATHER_FIELDS[key].label}</label>
                                        <SettingsInfo label={WEATHER_FIELDS[key].label} description={WEATHER_FIELDS[key].description} />
                                    </span>
                                    <div className="weather-settings__secret">
                                        <div className="weather-settings__secret-input">
                                            <textarea
                                                id={`weather-${key}`}
                                                className={visible ? "" : "is-masked"}
                                                autoComplete="off"
                                                spellCheck={false}
                                                maxLength={WEATHER_FIELDS[key].maxLength}
                                                placeholder={form.adding ? "" : "已配置，输入新值代表替换"}
                                                disabled={!!busy}
                                                value={draft[key]}
                                                onPaste={(event) => {
                                                    event.preventDefault();
                                                    handleDraft(key, event.clipboardData.getData("text").slice(0, WEATHER_FIELDS[key].maxLength));
                                                }}
                                                onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                            />
                                            <button
                                                type="button"
                                                className="weather-settings__icon"
                                                aria-label={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                                title={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                                aria-pressed={visible}
                                                onClick={() => setVisible(!visible)}
                                             data-button="icon">
                                                {visible ? <Glyph source={eyeOffIcon} /> : <Glyph source={eyeIcon} />}
                                            </button>
                                        </div>
                                        <div className="weather-settings__actions">
                                            <button type="button" disabled={!!busy} onClick={() => file.current?.click()}>
                                                导入私钥
                                            </button>
                                            <small>{draft.privateKey ? "已填写草稿" : form.adding ? "" : "已保存的私钥不回读"}</small>
                                        </div>
                                        <input
                                            ref={file}
                                            type="file"
                                            hidden
                                            accept=".pem,.key"
                                            onChange={(event) => {
                                                const selected = event.currentTarget.files?.[0]; // 读取用户明确选择的本地文件。
                                                event.currentTarget.value = "";
                                                if (selected) importPrivateKeyFile(selected, key);
                                            }}
                                        />
                                    </div>
                                </div>
                            ) : (
                                <div className="weather-settings__row" key={key}>
                                    <span>
                                        <label htmlFor={`weather-${key}`}>{WEATHER_FIELDS[key].label}</label>
                                        <SettingsInfo label={WEATHER_FIELDS[key].label} description={WEATHER_FIELDS[key].description} />
                                    </span>
                                    {WEATHER_FIELDS[key].secret ? (
                                        <div className="weather-settings__secret-input">
                                            <input
                                                id={`weather-${key}`}
                                                type={visible ? "text" : "password"}
                                                autoComplete="new-password"
                                                spellCheck={false}
                                                maxLength={WEATHER_FIELDS[key].maxLength}
                                                placeholder={form.adding ? "" : "已配置，输入新值代表替换"}
                                                disabled={!!busy}
                                                value={draft[key]}
                                                onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                            />
                                            <button
                                                type="button"
                                                className="weather-settings__icon"
                                                aria-label={visible ? "隐藏草稿" : "显示草稿"}
                                                title={visible ? "隐藏草稿" : "显示草稿"}
                                                aria-pressed={visible}
                                                onClick={() => setVisible(!visible)}
                                             data-button="icon">
                                                {visible ? <Glyph source={eyeOffIcon} /> : <Glyph source={eyeIcon} />}
                                            </button>
                                        </div>
                                    ) : (
                                        <input
                                            id={`weather-${key}`}
                                            autoComplete="off"
                                            spellCheck={false}
                                            maxLength={WEATHER_FIELDS[key].maxLength}
                                            disabled={!!busy}
                                            value={draft[key]}
                                            onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                        />
                                    )}
                                </div>
                            ),
                        )}
                        {!form.adding && (
                            <>
                                <h3>城市列表</h3>
                                <SettingsCityList
                                    cities={formCities}
                                    busy={!!busy}
                                    onDelete={(target) => requestCityRemoval(form.source, target)}
                                    onMenu={(target, position) => openCityMenu(form.source, target, position)}
                                    onAdd={() => {
                                        setStatus("");
                                        openPicker(form.source); // 城市列表分组按来源命令添加，与来源展开区同一条后端路径。
                                    }}
                                />
                            </>
                        )}
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        <span className={`weather-settings__dialog-status${failure ? " weather-settings__error" : ""}`} role="status">
                            {status}
                        </span>
                        <button type="button" className="weather-settings__primary" disabled={!formReady || !!busy} onClick={() => void handleSave()} data-button="primary">
                            {busy === "save" ? "正在保存" : "保存"}
                        </button>
                        <button type="button" disabled={busy === "save"} onClick={() => closeForm()}>
                            取消
                        </button>
                    </footer>
                    {renderCityMenu()}
                </dialog>
            )}
            {picker && (
                <WeatherCityDialog
                    title="添加城市"
                    type="add"
                    modal // 设置窗口的配置弹窗是顶层模态，城市弹窗必须是同一层才叠得上去。
                    state={{ city, checks, status, failure }}
                    busy={cityBusy || busy === "save"}
                    onSearch={handleSearch}
                    onSelect={handleSelect}
                    onValidate={handleValidate}
                    onConfirm={handleConfirm}
                    onClose={() => {
                        setCity(undefined);
                        setChecks([]);
                        setStatus("");
                        setFailure(false);
                        setPicker(undefined);
                    }}
                />
            )}
            {removal && (
                <dialog
                    ref={confirmDialog}
                    className="weather-settings__dialog weather-settings__dialog--confirm"
                    aria-label={removal.kind === "discard" ? "配置尚未保存" : "确认删除"}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy === "remove") return;
                        setRemoval(undefined);
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>{removal.kind === "source" ? "删除天气来源？" : removal.kind === "city" ? "删除城市或地区？" : "配置尚未保存"}</h2>
                    </header>
                    <div className="weather-settings__dialog-body">
                        <p>{confirmText}</p>
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        {removal.kind === "discard" ? (
                            <>
                                <button
                                    type="button"
                                    autoFocus
                                    disabled={busy === "save"}
                                    onClick={cancelDiscard}
                                >
                                    取消关闭
                                </button>
                                <button type="button" className="weather-settings__primary" disabled={!formReady || !!busy} onClick={handleSave} data-button="primary">
                                    保存
                                </button>
                                <button
                                    type="button"
                                    disabled={busy === "save"}
                                    onClick={discardAndLeave}
                                >
                                    不保存
                                </button>
                            </>
                        ) : (
                            <>
                                <button type="button" autoFocus disabled={busy === "remove"} onClick={() => setRemoval(undefined)}>
                                    取消
                                </button>
                                <button
                                    type="button"
                                    className="weather-settings__danger"
                                    disabled={busy === "remove"}
                                    onClick={() => {
                                        if (removal.kind === "source") void handleRemove();
                                        else void handleRemoveCity(removal.source, removal.city);
                                    }}
                                 data-button="danger">
                                    {busy === "remove" ? "正在删除" : "删除"}
                                </button>
                            </>
                        )}
                    </footer>
                </dialog>
            )}
        </section>
    );
};
