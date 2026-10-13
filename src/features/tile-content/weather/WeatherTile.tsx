import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
    getWeatherCity,
    getWeatherForecast,
    listenWeatherConfiguration,
    listenWeatherForecast,
    searchWeatherCities,
    setWeatherCity,
} from "../../../bridge/weather";
import { openCollectLink } from "../../../bridge/collect";
import type { ComponentInstanceSnapshot } from "../../../contracts/component";
import type { WeatherCity } from "../../../contracts/weather";
import { usePreferences } from "../../settings/PreferencesProvider";
import { ComponentTile, type ComponentTileProps } from "../../components/ComponentTile";
import type { DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import "./WeatherTile.css";
import editorLinkIcon from "../../../../resources/editor-link.svg?raw";
import { publishMessage } from "../../notifications/messageNotification";

type Forecast = {
    city: WeatherCity;
    current?: { temperature?: { value?: number }; condition?: { text?: string; code?: string } };
    hourly?: { hours?: { forecastTime?: string; temperature?: { value?: number }; condition?: { text?: string; code?: string } }[] };
    daily?: {
        days?: {
            forecastStartTime?: string;
            forecastEndTime?: string; // 使用供应商日界线，避免把夏令时日期当成固定 24 小时。
            temperatureMax?: { value?: number };
            temperatureMin?: { value?: number };
            daytime?: { condition?: { text?: string; code?: string } };
        }[];
    };
    attributions?: string[];
    fetchedAt?: number;
    moduleFetchedAt?: Record<string, number>; // 每个模块的实际获取时间。
    moduleErrors?: Record<string, string>;
};
const temperature = (value?: number) => (value == null ? "--" : `${Math.round(value)}°`);
const dateFormatter = (city: WeatherCity, options: Intl.DateTimeFormatOptions) =>
    new Intl.DateTimeFormat("zh-CN", { timeZone: city.tz || undefined, ...options });
const formatHour = (value: string | undefined, city: WeatherCity) =>
    value ? dateFormatter(city, { hour: "2-digit", minute: "2-digit" }).format(new Date(value)) : "--:--";
const formatDay = (value: string | undefined, city: WeatherCity) => (value ? dateFormatter(city, { weekday: "short" }).format(new Date(value)) : "--");
const isToday = (value: string | undefined, city: WeatherCity) => {
    const formatter = dateFormatter(city, { year: "numeric", month: "2-digit", day: "2-digit" }); // 按目标城市的日历判断，不以数组下标推断今天。
    return value !== undefined && Number.isFinite(Date.parse(value)) && formatter.format(new Date(value)) === formatter.format(new Date());
};
type ModuleState = "loading" | "ready" | "unavailable";
type WeatherMode = "summary" | "hourly" | "daily" | "full";

// 内容下限同时供自动判断、菜单预设及固定模式缩放使用。
const weatherSizes = {
    summary: { width: 168, height: 68, label: "当天摘要" },
    hourly: { width: 380, height: 120, label: "小时预报" },
    daily: { width: 200, height: 300, label: "7 天预报" },
    full: { width: 430, height: 320, label: "完整预报" },
};

// 以当前内容盒按优先级匹配，不依赖上次显示模式。
const selectWeatherMode = (width: number, height: number, fontScale: number) => {
    if (width >= weatherSizes.full.width * fontScale && height >= weatherSizes.full.height * fontScale) return "full";
    else if (width >= weatherSizes.daily.width * fontScale && height >= weatherSizes.daily.height * fontScale) return "daily";
    else if (width >= weatherSizes.hourly.width * fontScale && height >= weatherSizes.hourly.height * fontScale) return "hourly";
    else return "summary";
};

const ModuleNotice = ({ state }: { state: ModuleState }) => <p className="weather-tile__module-notice">{state === "loading" ? "正在获取" : "暂不可用"}</p>;

const WeatherGlyph = ({ code }: { code?: string }) => (
    <svg className="weather-tile__glyph" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
        {code && Number(code) >= 300 && <path d="M7 17h9a3 3 0 0 0 0-6 4.5 4.5 0 0 0-8.5-1A3 3 0 0 0 7 17Z" />}
    </svg>
);

const CurrentModule = ({ forecast, state }: { forecast?: Forecast; state: ModuleState }) => {
    const current = forecast?.current;
    const today = forecast?.daily?.days?.find((item) => isToday(item.forecastStartTime, forecast.city));
    return (
        <section className="weather-tile__module weather-tile__current" aria-label="当前天气">
            <div className="weather-tile__current-main">
                <WeatherGlyph code={current?.condition?.code} />
                <strong>{temperature(current?.temperature?.value)}</strong>
                <span>{current?.condition?.text ?? (state === "loading" ? "正在获取天气" : "暂不可用")}</span>
            </div>
            {state === "ready" ? (
                <small>
                    今天 {temperature(today?.temperatureMin?.value)}–{temperature(today?.temperatureMax?.value)}
                </small>
            ) : (
                <ModuleNotice state={state} />
            )}
        </section>
    );
};

const HourlyModule = ({ forecast, state }: { forecast?: Forecast; state: ModuleState }) => {
    const city = forecast?.city;
    const hours = forecast?.hourly?.hours ?? [];
    const [page, setPage] = useState(0);
    const pageSize = 5;
    const pages = Math.max(1, Math.ceil(hours.length / pageSize));
    useEffect(() => setPage((value) => Math.min(value, pages - 1)), [pages]);
    return (
        <section className="weather-tile__module weather-tile__hourly-module" aria-label="未来 24 小时">
            <header>
                <h3>小时预报</h3>
                <div>
                    <button type="button" aria-label="上一组小时预报" title="上一组" disabled={page === 0} onClick={() => setPage((value) => value - 1)}>
                        ‹
                    </button>
                    <button type="button" aria-label="下一组小时预报" title="下一组" disabled={page >= pages - 1} onClick={() => setPage((value) => value + 1)}>
                        ›
                    </button>
                </div>
            </header>
            <div className="weather-tile__hourly">
                {hours.slice(page * pageSize, page * pageSize + pageSize).map((item, index) => (
                    <div className="weather-tile__hour" key={`${item.forecastTime ?? "hour"}-${index}`}>
                        <small>{city ? formatHour(item.forecastTime, city) : "--:--"}</small>
                        <WeatherGlyph code={item.condition?.code} />
                        <span>{temperature(item.temperature?.value)}</span>
                    </div>
                ))}
            </div>
            {!hours.length && <ModuleNotice state={state} />}
            {hours.length > 0 && forecast?.moduleErrors?.hourly && <p className="weather-tile__module-notice">{forecast.moduleErrors.hourly}</p>}
        </section>
    );
};

const DailyModule = ({ forecast, state }: { forecast?: Forecast; state: ModuleState }) => {
    const city = forecast?.city;
    return (
        <section className="weather-tile__module weather-tile__daily-module" aria-label="未来 7 天">
            <h3>7 天预报</h3>
            <div className="weather-tile__daily">
                {(forecast?.daily?.days ?? []).slice(0, 7).map((item, index) => (
                    <div className="weather-tile__day" key={`${item.forecastStartTime ?? "day"}-${index}`}>
                        <span>{city ? (isToday(item.forecastStartTime, city) ? "今天" : formatDay(item.forecastStartTime, city)) : "--"}</span>
                        <WeatherGlyph code={item.daytime?.condition?.code} />
                        <small>{item.daytime?.condition?.text ?? "--"}</small>
                        <strong>
                            {temperature(item.temperatureMin?.value)} / {temperature(item.temperatureMax?.value)}
                        </strong>
                    </div>
                ))}
            </div>
            {!forecast?.daily?.days?.length && <ModuleNotice state={state} />}
            {!!forecast?.daily?.days?.length && forecast.moduleErrors?.daily && <p className="weather-tile__module-notice">{forecast.moduleErrors.daily}</p>}
        </section>
    );
};

/** 当前天气格子的会话城市选择器，不写入 GeoAPI 结果或工作区布局。 */
const CityPicker = ({ onSelect, onClose }: { onSelect: (city: WeatherCity) => Promise<void>; onClose: () => void }) => {
    const [query, setQuery] = useState("");
    const [cities, setCities] = useState<WeatherCity[]>([]);
    const [failure, setFailure] = useState<string>();
    const [busy, setBusy] = useState(false);
    const [saving, setSaving] = useState(false);
    const session = useRef({ active: false, revision: 0, busy: false, ready: false }); // 命令式回调读取最新会话，不依赖渲染时捕获的 busy。
    useEffect(() => {
        session.current.active = true;
        let active = true; // 区分 StrictMode 重新挂载前后异步完成的订阅。
        let stop: (() => void) | undefined; // 异步订阅在卸载后完成时立即退订。
        ++session.current.revision;
        void listenWeatherConfiguration(() => {
            if (!active) return;
            ++session.current.revision;
            session.current.busy = false;
            setBusy(false);
            setCities([]);
            setFailure("天气配置已改变，请重新搜索城市");
        })
            .then((unlisten) => {
                if (!active) return unlisten();
                stop = unlisten;
                session.current.ready = true;
            })
            .catch(() => {
                if (active) setFailure("天气配置通知订阅失败");
            });
        return () => {
            active = false;
            session.current.active = false;
            session.current.ready = false;
            session.current.busy = false;
            ++session.current.revision;
            stop?.();
        };
    }, []);
    const handleSearch = async () => {
        if (!query.trim() || !session.current.active || !session.current.ready || session.current.busy || saving) return;
        const revision = ++session.current.revision; // 关闭、配置变化或新查询后拒绝旧结果。
        session.current.busy = true;
        setBusy(true);
        setCities([]);
        setFailure(undefined);
        try {
            const results = await searchWeatherCities(query.trim());
            if (session.current.active && session.current.revision === revision) setCities(results);
        } catch (error) {
            if (!session.current.active || session.current.revision !== revision) return;
            setCities([]);
            setFailure(error instanceof Error ? error.message : "城市搜索失败");
        } finally {
            if (session.current.active && session.current.revision === revision) {
                session.current.busy = false;
                setBusy(false);
            }
        }
    };
    return (
        <div className="weather-tile__picker-backdrop" role="presentation" onPointerDown={(event) => event.target === event.currentTarget && onClose()}>
            <section className="weather-tile__picker" role="dialog" aria-modal="true" aria-label="选择城市">
                <h2>选择城市</h2>
                <form
                    onSubmit={(event) => {
                        event.preventDefault();
                        void handleSearch();
                    }}
                >
                    <input
                        aria-label="搜索城市"
                        value={query}
                        maxLength={200}
                        disabled={busy || saving}
                        onChange={(event) => setQuery(event.currentTarget.value)}
                        autoFocus
                    />
                    <button type="submit" disabled={!query.trim() || busy || saving}>
                        {busy ? "搜索中" : "搜索"}
                    </button>
                </form>
                <div className="weather-tile__cities" aria-live="polite">
                    {cities.map((city) => (
                        <button
                            type="button"
                            key={city.id}
                            disabled={saving || busy}
                            onClick={() => {
                                setSaving(true);
                                void onSelect(city)
                                    .catch((error) => setFailure(error instanceof Error ? error.message : "城市保存失败"))
                                    .finally(() => setSaving(false));
                            }}
                        >
                            <strong>{city.name}</strong>
                            <small>{[city.adm2, city.adm1, city.country].filter(Boolean).join("，")}</small>
                        </button>
                    ))}
                    {!busy && !cities.length && <small>{failure ?? "输入城市名称后搜索"}</small>}
                </div>
                {failure && (
                    <p className="weather-tile__error" role="alert">
                        {failure}
                    </p>
                )}
                <footer>
                    <button type="button" onClick={onClose}>
                        取消
                    </button>
                </footer>
            </section>
        </div>
    );
};

const WeatherContent = ({
    instance,
    city,
    visible,
    onChooseCity,
    fixedMode,
    onModeChange,
}: {
    instance: ComponentInstanceSnapshot;
    city?: WeatherCity;
    visible: boolean;
    onChooseCity: () => void;
    fixedMode?: WeatherMode; // 固定时不执行尺寸模式判断。
    onModeChange: (mode: WeatherMode) => void; // 菜单使用内容实际呈现的模式。
}) => {
    const summaryRef = useRef<HTMLDivElement>(null);
    const [forecast, setForecast] = useState<Forecast>();
    const [failure, setFailure] = useState<string>();
    const [motionActive, setMotionActive] = useState(() => document.visibilityState === "visible");
    const [expired, setExpired] = useState(false); // 来源栏在模块有效期到达时更新，不依赖其他界面操作。
    const [contentSize, setContentSize] = useState({ width: 0, height: 0 }); // 公共视口扣除内边距后的实际内容盒。
    const { animationsEnabled, preferences } = usePreferences();
    const modulesRef = useRef<HTMLDivElement>(null); // 模块的布局与动画都由本内容组件拥有。
    const moduleAnimations = useRef(new Map<HTMLElement, Animation>()); // 稳定画面不保留动画资源。
    const [paused, setPaused] = useState(() => window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false); // 复用宿主的逐屏暂停状态。
    useEffect(() => {
        const handleRefreshState = () => setPaused(window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false);
        window.addEventListener("desktop-refresh-changed", handleRefreshState);
        handleRefreshState();
        return () => window.removeEventListener("desktop-refresh-changed", handleRefreshState);
    }, []);
    useEffect(() => {
        setForecast(undefined);
        setFailure(undefined);
    }, [city?.id]);
    useEffect(() => {
        if (!city) return;
        setMotionActive(document.visibilityState === "visible");
        let active = true; // 暂停、折叠或更换城市后拒绝迟到结果。
        let revision = 0; // 页面隐藏后旧读取不得覆盖恢复时的新结果。
        let stopActivity: (() => void) | undefined; // 只为活动内容登记后端调度。
        let activityRevision = 0; // 异步登记迟到时立即撤销。
        let activityPending = false;
        const loadForecast = () => {
            if (!active || !visible || paused || document.visibilityState === "hidden") return;
            if (!stopActivity && !activityPending) {
                activityPending = true;
                const registration = ++activityRevision;
                void listenWeatherForecast(instance.instanceId, instance.projectionMonitorId ?? instance.monitorId, city, (value) => {
                    if (!active || registration !== activityRevision || document.visibilityState === "hidden") return;
                    const next = value as Forecast;
                    if (next?.city?.id !== city.id) return;
                    ++revision;
                    setForecast(next);
                    setFailure(undefined);
                })
                    .then((stop) => {
                        if (!active || registration !== activityRevision) return stop();
                        activityPending = false;
                        stopActivity = stop;
                    })
                    .catch(() => {
                        if (active && registration === activityRevision) {
                            activityPending = false;
                            setFailure("天气活动登记失败");
                        }
                    });
            }
            const requestRevision = ++revision;
            void getWeatherForecast(city)
                .then((value) => active && requestRevision === revision && (setForecast(value as Forecast), setFailure(undefined)))
                .catch((error: unknown) => active && requestRevision === revision && setFailure(error instanceof Error ? error.message : "天气暂不可用"));
        };
        const handleVisibilityChange = () => {
            ++revision;
            if (document.visibilityState === "hidden") {
                ++activityRevision;
                activityPending = false;
                stopActivity?.();
                stopActivity = undefined;
            }
            setMotionActive(document.visibilityState === "visible");
            if (document.visibilityState === "visible") loadForecast();
        };
        document.addEventListener("visibilitychange", handleVisibilityChange);
        let stop: (() => void) | undefined; // 异步订阅完成前卸载也必须释放监听。
        void listenWeatherConfiguration(() => {
            if (!active) return;
            ++revision;
            setForecast(undefined);
            setFailure(undefined);
            loadForecast();
        })
            .then((unlisten) => {
                if (!active) return unlisten();
                stop = unlisten;
                loadForecast();
            })
            .catch(() => active && setFailure("天气配置通知订阅失败"));
        return () => {
            active = false;
            stop?.();
            ++activityRevision;
            stopActivity?.();
            document.removeEventListener("visibilitychange", handleVisibilityChange);
        };
    }, [city?.id, visible, paused, instance.instanceId, instance.monitorId, instance.projectionMonitorId]);
    useEffect(() => {
        const currentFetchedAt = forecast?.moduleFetchedAt?.current; // 不从整包时间推断当前天气年龄。
        if (!forecast?.current || !visible || paused) return;
        let active = true; // 拒绝卸载或预报替换后的迟到回调。
        let timer: ReturnType<typeof setTimeout> | undefined; // 只为下一次到期设置一次性计时。
        const handleExpiry = () => {
            clearTimeout(timer);
            if (!active || document.visibilityState === "hidden") return;
            const now = Date.now(); // 从隐藏恢复时重新计算，避免沿用暂停前剩余时间。
            const remaining =
                typeof currentFetchedAt === "number" && Number.isFinite(currentFetchedAt) && currentFetchedAt <= now
                    ? currentFetchedAt + 24 * 60 * 60 * 1000 - now
                    : 0;
            if (remaining > 0) {
                timer = setTimeout(handleExpiry, remaining);
                return;
            }
            setForecast((latest) =>
                latest?.current && latest.moduleFetchedAt?.current === currentFetchedAt
                    ? {
                          ...latest,
                          current: undefined,
                          moduleErrors: { ...latest.moduleErrors, current: latest.moduleErrors?.current ?? "当前天气已超过展示期限，暂不可用" },
                      }
                    : latest,
            );
        };
        document.addEventListener("visibilitychange", handleExpiry);
        handleExpiry();
        return () => {
            active = false;
            clearTimeout(timer);
            document.removeEventListener("visibilitychange", handleExpiry);
        };
    }, [forecast?.current, forecast?.moduleFetchedAt?.current, visible, paused]);
    useEffect(() => {
        if (!forecast || !visible || paused) return;
        let active = true; // 新预报或城市替换后拒绝旧到期回调。
        let timer: ReturnType<typeof setTimeout> | undefined; // 仅等待最近条目的结束时间。
        const handleForecastWindow = () => {
            clearTimeout(timer);
            if (!active || document.visibilityState === "hidden") return;
            const now = Date.now(); // 恢复活动后重新判断，不沿用隐藏前的剩余时间。
            const hours = forecast.hourly?.hours; // 保持缺少模块与空模块的区别。
            const days = forecast.daily?.days;
            const hourEnd = (item: NonNullable<NonNullable<Forecast["hourly"]>["hours"]>[number]) => Date.parse(item.forecastTime ?? "") + 60 * 60 * 1000;
            const dayEnd = (item: NonNullable<NonNullable<Forecast["daily"]>["days"]>[number]) => Date.parse(item.forecastEndTime ?? "");
            const remainingHours = hours?.filter((item) => hourEnd(item) > now);
            const remainingDays = days?.filter((item) => dayEnd(item) > now);
            if (remainingHours?.length !== hours?.length || remainingDays?.length !== days?.length) {
                setForecast((latest) =>
                    latest === forecast
                        ? {
                              ...latest,
                              hourly: hours ? { ...latest.hourly, hours: remainingHours } : latest.hourly,
                              daily: days ? { ...latest.daily, days: remainingDays } : latest.daily,
                              moduleErrors: {
                                  ...latest.moduleErrors,
                                  ...(hours && remainingHours?.length !== hours.length
                                      ? { hourly: latest.moduleErrors?.hourly ?? "小时预报窗口不足，部分时段暂不可用" }
                                      : {}),
                                  ...(days && remainingDays?.length !== days.length
                                      ? { daily: latest.moduleErrors?.daily ?? "日预报窗口不足，部分日期暂不可用" }
                                      : {}),
                              },
                          }
                        : latest,
                );
            }
            const nextExpiry = Math.min(...(remainingHours?.map(hourEnd) ?? []), ...(remainingDays?.map(dayEnd) ?? [])); // 没有有效条目时不保留计时器。
            if (Number.isFinite(nextExpiry)) timer = setTimeout(handleForecastWindow, Math.min(nextExpiry - now, 2_147_483_647));
        };
        document.addEventListener("visibilitychange", handleForecastWindow);
        handleForecastWindow();
        return () => {
            active = false;
            clearTimeout(timer);
            document.removeEventListener("visibilitychange", handleForecastWindow);
        };
    }, [forecast, visible, paused]);
    useEffect(() => {
        const element = summaryRef.current?.closest<HTMLElement>(".official-component__content"); // 标题显隐和拖动缩放均由公共视口决定可用空间。
        if (!element || typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver((entries) => {
            const bounds = entries.find((entry) => entry.target === element)?.contentRect; // contentRect 不包含视口 padding 和边框。
            if (!bounds) return;
            setContentSize((current) =>
                current.width === bounds.width && current.height === bounds.height ? current : { width: bounds.width, height: bounds.height },
            );
        });
        observer.observe(element);
        return () => observer.disconnect();
    }, [instance.instanceId]);
    const contentWidth = Math.max(0, contentSize.width);
    const contentHeight = Math.max(0, contentSize.height);
    const fontScale = (preferences?.fontSize ?? 13) / 13;
    const mode = fixedMode ?? selectWeatherMode(contentWidth, contentHeight, fontScale);
    useEffect(() => onModeChange(mode), [mode, onModeChange]);
    const animateModules = animationsEnabled && motionActive && visible && !paused; // 沿用有效动画与宿主活动门槛。
    useLayoutEffect(() => {
        const container = modulesRef.current;
        if (!container) return;
        const initialized = container.dataset.mode !== undefined;
        const origin = container.getBoundingClientRect();
        const elements = Array.from(container.children) as HTMLElement[]; // 当前、小时、日模块始终保留同一 DOM 身份。
        const before = elements.map((element) => ({
            rect: element.getBoundingClientRect(),
            shown: !element.hidden,
            opacity: getComputedStyle(element).opacity,
        })); // 取消前读取正在呈现的中间帧。
        moduleAnimations.current.forEach((animation) => animation.cancel());
        moduleAnimations.current.clear();
        container.dataset.mode = mode;
        const selected = [true, mode === "hourly" || mode === "full", mode === "daily" || mode === "full"];
        elements.forEach((element, index) => {
            const shown = selected[index];
            if (index === 2 && shown) element.dataset.horizontal = String(mode === "full"); // 退出过程中保留旧日预报排布。
            element.inert = !shown;
            element.setAttribute("aria-hidden", String(!shown));
            if (!shown && element.contains(document.activeElement)) (document.activeElement as HTMLElement | null)?.blur();
            element.style.position = "";
            element.style.left = "";
            element.style.top = "";
            element.style.width = "";
            element.hidden = !shown;
            if (!shown && before[index].shown && initialized && animateModules) {
                element.hidden = false;
                Object.assign(element.style, {
                    position: "absolute",
                    left: `${before[index].rect.left - origin.left}px`,
                    top: `${before[index].rect.top - origin.top}px`,
                    width: `${before[index].rect.width}px`,
                });
            }
        });
        if (!initialized || !animateModules) return;
        elements.forEach((element, index) => {
            if (element.hidden) return;
            const target = element.getBoundingClientRect();
            const old = before[index];
            const shown = selected[index];
            const dx = old.shown ? old.rect.left - target.left : 0;
            const dy = old.shown ? old.rect.top - target.top : 4;
            if (shown && old.shown && dx === 0 && dy === 0 && Number(old.opacity) === 1) return;
            const animation = element.animate(
                [
                    { transform: `translate(${dx}px, ${dy}px)`, opacity: old.shown ? old.opacity : "0" },
                    { transform: shown ? "none" : "translateY(4px)", opacity: shown ? "1" : "0" },
                ],
                { duration: 180, easing: "ease-out" },
            ); // 只平移和淡入淡出，不缩放文字与图标。
            moduleAnimations.current.set(element, animation);
            animation.onfinish = () => {
                if (moduleAnimations.current.get(element) !== animation) return;
                if (!shown) element.hidden = true;
                moduleAnimations.current.delete(element);
                animation.cancel();
            };
        });
    }, [mode, animateModules, city?.id]);
    useEffect(
        () => () => {
            moduleAnimations.current.forEach((animation) => animation.cancel());
            moduleAnimations.current.clear();
        },
        [],
    );
    useEffect(() => {
        if (!visible || paused) return;
        let active = true; // 预报替换或卸载后拒绝旧计时回调。
        let timer: ReturnType<typeof setTimeout> | undefined; // 仅等待最早模块到期点，不轮询。
        const handleStatusExpiry = () => {
            clearTimeout(timer);
            if (!active || document.visibilityState === "hidden") return;
            const now = Date.now(); // 隐藏恢复后重新计算有效期。
            const deadlines = (["current", "hourly", "daily"] as const)
                .filter((module) => forecast?.[module])
                .map((module) => {
                    const timestamp = forecast?.moduleFetchedAt?.[module]; // 缺失或未来获取时间不能证明数据有效。
                    return typeof timestamp === "number" && Number.isFinite(timestamp) && timestamp <= now
                        ? timestamp + { current: 30, hourly: 60, daily: 180 }[module] * 60 * 1000
                        : now;
                });
            const deadline = Math.min(...deadlines); // 无数据时不显示缓存过期标记。
            setExpired(deadline <= now);
            if (Number.isFinite(deadline) && deadline > now) timer = setTimeout(handleStatusExpiry, deadline - now);
        };
        document.addEventListener("visibilitychange", handleStatusExpiry);
        handleStatusExpiry();
        return () => {
            active = false;
            clearTimeout(timer);
            document.removeEventListener("visibilitychange", handleStatusExpiry);
        };
    }, [forecast, visible, paused]);
    const className = `weather-tile__summary is-${mode}`;
    if (!city)
        return (
            <div ref={summaryRef} className={className}>
                <div className="weather-tile__empty">
                    <strong>未选择城市或地区</strong>
                    <button type="button" onClick={onChooseCity}>
                        选择城市
                    </button>
                </div>
            </div>
        );
    const currentState: ModuleState = forecast?.current ? "ready" : forecast?.moduleErrors?.current || failure ? "unavailable" : "loading";
    const hourlyState: ModuleState = forecast?.hourly?.hours?.length ? "ready" : forecast?.moduleErrors?.hourly || failure ? "unavailable" : "loading";
    const dailyState: ModuleState = forecast?.daily?.days?.length ? "ready" : forecast?.moduleErrors?.daily || failure ? "unavailable" : "loading";
    const status = expired
        ? "已过期"
        : forecast?.fetchedAt
          ? `${dateFormatter(city, { hour: "2-digit", minute: "2-digit" }).format(forecast.fetchedAt)} 更新`
          : "正在获取天气";
    return (
        <div ref={summaryRef} className={className}>
            <div ref={modulesRef} className="weather-tile__modules">
                <CurrentModule forecast={forecast} state={currentState} />
                <HourlyModule forecast={forecast} state={hourlyState} />
                <DailyModule forecast={forecast} state={dailyState} />
            </div>
            <footer className="weather-tile__footer">
                <small
                    title={
                        forecast?.fetchedAt
                            ? `获取时间：${dateFormatter(city, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", timeZoneName: "short" }).format(forecast.fetchedAt)}`
                            : undefined
                    }
                >
                    和风 · {status}
                </small>
                {failure && <small className="weather-tile__stale">{failure}</small>}
            </footer>
        </div>
    );
};

export const WeatherTile = (props: ComponentTileProps) => {
    const [city, setCity] = useState<WeatherCity>();
    const [pickerOpen, setPickerOpen] = useState(false);
    const { preferences } = usePreferences();
    const [displayMode, setDisplayMode] = useState<WeatherMode>("summary");
    const busy = useRef(false); // 防止连续菜单操作交错提交。
    const state = props.instance.state && typeof props.instance.state === "object" ? (props.instance.state as Record<string, unknown>) : {};
    const savedMode = state.weatherMode;
    const fixedMode =
        state.weatherModeFixed === true ? (savedMode === "hourly" || savedMode === "daily" || savedMode === "full" ? savedMode : "summary") : undefined;
    const fontScale = (preferences?.fontSize ?? 13) / 13;
    const headerHeight = !props.appearance || props.appearance.titleMode === "fixed" ? 28 : 0;
    const sizeFor = (mode: WeatherMode) => ({
        width: Math.ceil(weatherSizes[mode].width * fontScale + 24),
        height: Math.ceil(weatherSizes[mode].height * fontScale + 24 + headerHeight),
    });
    const handleMode = async (mode: WeatherMode, fixed: boolean, resize: boolean) => {
        if (busy.current) return;
        busy.current = true;
        try {
            const minimum = sizeFor(mode); // 尺寸先提交成功再固定，空间不足时不写入新模式。
            const bounds = {
                ...props.instance.bounds,
                width: resize ? minimum.width : Math.max(props.instance.bounds.width, minimum.width),
                height: resize ? minimum.height : Math.max(props.instance.bounds.height, minimum.height),
            };
            if ((resize || fixed) && (bounds.width !== props.instance.bounds.width || bounds.height !== props.instance.bounds.height))
                await props.onCommand({ type: "update-component-bounds", instanceId: props.instance.instanceId, bounds });
            await props.onCommand({
                type: "update-component-state",
                instanceId: props.instance.instanceId,
                stateVersion: props.instance.stateVersion,
                state: { ...state, weatherModeFixed: fixed, weatherMode: mode },
            });
        } catch (error) {
            publishMessage({ type: "error", content: error instanceof Error ? error.message : "天气显示模式保存失败", source: "desktop-tile" });
        } finally {
            busy.current = false;
        }
    };
    const menuItems = useMemo<DesktopTileMenuItem[]>(
        () => [
            { label: "查看详情", disabled: !city, onSelect: () => city && void openCollectLink(city.fxLink).catch(() => undefined) },
            { label: city ? "更换城市" : "选择城市", onSelect: () => setPickerOpen(true) },
            {
                label: "固定显示模式",
                separatorBefore: true,
                checked: fixedMode !== undefined,
                onSelect: () => void handleMode(fixedMode ?? displayMode, fixedMode === undefined, false),
            },
            {
                label: "切换显示模式",
                children: (Object.keys(weatherSizes) as WeatherMode[]).map((mode) => ({
                    label: weatherSizes[mode].label,
                    radio: true,
                    checked: (fixedMode ?? displayMode) === mode,
                    onSelect: () => void handleMode(mode, fixedMode !== undefined, true),
                })),
            },
        ],
        [city, props, fixedMode, displayMode, fontScale, headerHeight],
    );
    useEffect(() => {
        let active = true;
        void getWeatherCity()
            .then((saved) => active && setCity(saved ?? undefined))
            .catch(() => active && setCity(undefined));
        return () => {
            active = false;
        };
    }, []);
    return (
        <ComponentTile
            {...props}
            title={city ? `${city.name} · ${city.adm2 ?? city.adm1 ?? city.country}` : "天气"}
            minimum={sizeFor(fixedMode ?? "summary")}
            menuItems={menuItems}
            commands={() => (
                <button
                    type="button"
                    title="查看详情"
                    aria-label="查看详情"
                    disabled={!city}
                    onClick={() => city && openCollectLink(city.fxLink).catch(() => undefined)}
                >
                    <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: editorLinkIcon }} />
                </button>
            )}
            scrollable={false}
        >
            {(collapsed) => (
                <>
                    <WeatherContent
                        instance={props.instance}
                        city={city}
                        visible={!collapsed}
                        onChooseCity={() => setPickerOpen(true)}
                        fixedMode={fixedMode}
                        onModeChange={setDisplayMode}
                    />
                    {pickerOpen && (
                        <CityPicker
                            onSelect={async (nextCity) => {
                                const saved = await setWeatherCity(nextCity);
                                setCity(saved);
                                setPickerOpen(false);
                            }}
                            onClose={() => setPickerOpen(false)}
                        />
                    )}
                </>
            )}
        </ComponentTile>
    );
};
