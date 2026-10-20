import { useEffect, useReducer, useRef, useState } from "react";
import closeIcon from "../../../resources/weather-close.svg?raw";
import checkIcon from "../../../resources/weather-check.svg?raw";
import searchIcon from "../../../resources/weather-search.svg?raw";
import type { WeatherCheck, WeatherCity } from "../../contracts/weather";
import "./WeatherCityDialog.css";

/** 弹窗形态决定主命令文案；两种形态的搜索、选择、验证与确认流程一致。 */
export type WeatherCityDialogType = "select" | "add";

/** 搜索会话状态；结果与草稿只服务本次弹窗，代次用于拒绝迟到结果。 */
interface WeatherCityState {
    query: string; // 已输入但尚未提交的搜索内容。
    results: WeatherCity[]; // 最近一次成功搜索的选项。
    selection?: WeatherCity; // 本次弹窗内改选、尚未确认的城市。
}

type WeatherCityAction =
    { type: "query"; value: string } | { type: "searching" } | { type: "results"; cities: WeatherCity[] } | { type: "select"; city: WeatherCity };

/** 搜索会话归约：提交搜索清空旧选项，选择只记录草稿，不触发外部副作用。 */
export const weatherCityReducer = (state: WeatherCityState, action: WeatherCityAction): WeatherCityState => {
    if (action.type === "query") return { ...state, query: action.value };
    if (action.type === "searching") return { ...state, results: [], selection: undefined };
    if (action.type === "results") return { ...state, results: action.cities };
    return { ...state, selection: action.city };
};

/** 由调用方传入的当前选中城市与验证结果，决定三段状态与主命令可用性。 */
export interface WeatherCityDialogState {
    city?: WeatherCity; // 已选择但尚未确认的城市。
    checks: WeatherCheck[]; // 当前城市的验证结果，空数组表示尚未验证。
    status: string; // 本次动作的状态或失败说明。
    failure: boolean; // 状态文本是否使用错误色。
}

/** 图标资源以原始 SVG 保存，尺寸与颜色由所在按键的样式决定。 */
const Glyph = ({ source }: { source: string }) => <span className="weather-city__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: source }} />;

/** 城市弹窗：搜索、选择、验证、确认的共享实现；纯受控，不导入桥接层，也不直接发起命令。 */
export const WeatherCityDialog = ({
    title,
    type,
    state,
    busy,
    modal = false,
    onSearch,
    onSelect,
    onValidate,
    onConfirm,
    onClose,
}: {
    title: string; // 由调用方给出的弹窗标题。
    type: WeatherCityDialogType; // 弹窗形态，决定主命令文案。
    state: WeatherCityDialogState; // 由调用方持有的选择、验证与状态。
    busy: boolean; // 调用方是否正在执行某个城市动作。
    modal?: boolean; // 是否使用顶层模态：设置页的弹窗叠在同样模态的配置弹窗之上，必须进同一层。
    onSearch: (query: string) => Promise<WeatherCity[]>; // 按提交触发的位置搜索。
    onSelect: (city?: WeatherCity) => void; // 本次弹窗内改选的城市；新搜索开始时清空。
    onValidate: (city: WeatherCity) => Promise<void>; // 对传入的城市发起能力验证。
    onConfirm: (city: WeatherCity) => Promise<void>; // 确认传入的城市，由调用方写入并关闭弹窗。
    onClose: () => void; // 取消关闭，不改变调用方已确认的值。
}) => {
    const dialog = useRef<HTMLDialogElement>(null); // 模态模式下的顶层弹窗元素。
    const query = useRef<HTMLInputElement>(null); // 打开后聚焦的搜索输入。
    const busyRef = useRef(busy); // 关闭键读取最新忙碌状态，不因忙碌变化重建监听。
    busyRef.current = busy;
    const generation = useRef(0); // 搜索会话身份，递增即作废在途结果。
    const active = useRef(true); // 卸载后拒绝迟到结果。
    const [session, dispatch] = useReducer(weatherCityReducer, { query: "", results: [] });
    const latest = session.selection ?? state.city; // 主命令要确认的城市：本次改选优先，否则是调用方当前值。
    const verified = state.checks.length > 0 && state.checks.every((check) => check.failure === null); // 只决定验证按键文案，不作为写入前提。

    useEffect(() => {
        active.current = true;
        const element = modal ? dialog.current : null;
        if (element && !element.open) element.showModal(); // 模态模式进入顶层，叠在配置弹窗之上。
        query.current?.focus(); // 两种模式都不做焦点陷阱，只把焦点落到搜索输入。
        /** 顶层模态的关闭键由 dialog 自己接收，页面层只处理浮层模式。 */
        const handleKeyDown = (event: KeyboardEvent) => {
            if (!modal && event.key === "Escape" && !busyRef.current) onClose();
        };
        window.addEventListener("keydown", handleKeyDown);
        return () => {
            active.current = false;
            generation.current++;
            window.removeEventListener("keydown", handleKeyDown);
            if (element?.open) element.close();
        };
    }, [modal]);

    /** 提交搜索：清空旧选项与旧选择，忙碌期间拒绝重复提交，迟到结果不覆盖最新一次。 */
    const handleSearch = async () => {
        if (busy || !session.query.trim()) return;
        const current = ++generation.current; // 本次搜索的前端身份。
        dispatch({ type: "searching" });
        onSelect(undefined);
        try {
            const cities = await onSearch(session.query);
            if (current === generation.current && active.current) dispatch({ type: "results", cities });
        } catch {
            if (current === generation.current && active.current) dispatch({ type: "results", cities: [] }); // 失败原因由调用方在同一状态行给出。
        }
    };

    /** 改选城市只记录草稿并同步给调用方，由调用方作废上一次验证结果。 */
    const handleSelect = (city: WeatherCity) => {
        if (busy) return;
        generation.current++;
        dispatch({ type: "select", city });
        onSelect(city);
    };

    /** 弹窗内容只写一份，两种承载方式（顶层模态 / 页面内浮层）共用。 */
    const content = (
        <>
            <header className="weather-city__head">
                <h2>{title}</h2>
                <button type="button" className="weather-city__close" aria-label="关闭" title="关闭" onClick={onClose}>
                    <Glyph source={closeIcon} />
                </button>
            </header>
            <div className="weather-city__body">
                <form
                    className="weather-city__search"
                    onSubmit={(event) => {
                        event.preventDefault();
                        void handleSearch();
                    }}
                >
                    <input
                        ref={query}
                        aria-label="搜索城市"
                        value={session.query}
                        maxLength={100}
                        disabled={busy}
                        onChange={(event) => dispatch({ type: "query", value: event.currentTarget.value })}
                    />
                    <button type="submit" className="weather-city__icon" title="搜索城市" aria-label="搜索城市" disabled={!session.query.trim() || busy}>
                        <Glyph source={searchIcon} />
                    </button>
                </form>
                <div className="weather-city__results">
                    <div className="weather-city__scroll">
                        <div className="weather-city__cities">
                            {session.results.map((result) => (
                                <button
                                    type="button"
                                    key={result.id}
                                    className={result.id === latest?.id ? "is-selected" : undefined}
                                    disabled={busy}
                                    onClick={() => handleSelect(result)}
                                >
                                    <span>{result.name}</span>
                                    <small>{[result.adm2, result.adm1, result.country].filter(Boolean).join("，")}</small>
                                </button>
                            ))}
                        </div>
                        {state.checks.length === 0 ? null : (
                            <ul className="weather-city__checks">
                                {state.checks.map((check) => (
                                    <li key={check.label}>
                                        <span className={`weather-city__result-icon${check.failure === null ? " is-passed" : ""}`} aria-hidden="true">
                                            {check.failure === null ? <Glyph source={checkIcon} /> : null}
                                        </span>
                                        <span>{check.label}</span>
                                        <span className={check.failure ? "weather-city__error" : "weather-city__passed"}>
                                            {check.failure?.message ?? "通过"}
                                        </span>
                                    </li>
                                ))}
                            </ul>
                        )}
                    </div>
                </div>
            </div>
            <footer className="weather-city__actions">
                <span className={`weather-city__status${state.failure ? " weather-city__error" : ""}`} role="status">
                    {state.status}
                </span>
                <button type="button" disabled={!latest || busy} onClick={() => latest && void onValidate(latest)}>
                    {verified ? "重新验证" : "验证配置"}
                </button>
                <button type="button" className="weather-city__primary" disabled={!latest || busy} onClick={() => latest && void onConfirm(latest)}>
                    {type === "add" ? "添加" : "确认"}
                </button>
                <button type="button" disabled={busy} onClick={onClose}>
                    取消
                </button>
            </footer>
        </>
    );

    if (modal)
        return (
            <dialog
                ref={dialog}
                className="weather-city"
                aria-label={title}
                onCancel={(event) => {
                    event.preventDefault();
                    if (!busy) onClose(); // 顶层模态下 Escape 由 dialog 接收。
                }}
            >
                {content}
            </dialog>
        );

    return (
        <div
            className="weather-city__overlay"
            onPointerDown={(event) => {
                if (event.target === event.currentTarget && !busy) onClose(); // 点击浮层以外收起，不拦截桌面其他区域。
            }}
        >
            <section className="weather-city" role="dialog" aria-modal="false" aria-label={title}>
                {content}
            </section>
        </div>
    );
};
