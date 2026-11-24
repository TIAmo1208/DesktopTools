import type { IslandContentUnit } from "./islandContent.ts";
import { WEATHER_SOURCES, type WeatherCity, type WeatherSource, type WeatherSourceStatus } from "../../contracts/weather.ts";
import type { WeatherOutlookFacts } from "./islandWeatherOutlook.ts";

/** 浮岛天气卡从同一预报快照读取当前、小时和日预报。 */
export interface IslandWeatherForecast extends WeatherOutlookFacts {
    source?: WeatherSource; // 事件载荷来源，用于区分同名城市。
    city?: { id?: string; name?: string; tz?: string }; // 当前预报位置。
    daily?: {
        days?: {
            forecastStartTime?: string;
            temperatureMax?: { value?: number };
            temperatureMin?: { value?: number };
            daytime?: { condition?: { text?: string; code?: string } };
        }[];
    };
    moduleFetchedAt?: Record<string, number>; // 当前天气的获取时刻。
    moduleErrors?: Record<string, string>; // 有失败的预报模块不渲染对应区块。
}

/** 浮岛天气绑定结果；缺失态只给一句可执行说明，不在浮岛内放设置入口。 */
export type IslandWeatherBinding =
    | { state: "ready"; source: WeatherSource; city: WeatherCity }
    | { state: "no-sources" | "no-source" | "source-removed" | "no-city" | "city-removed"; message: string };

/** 以设置中的来源列表为唯一事实，判定浮岛天气卡当前能否取数。 */
export const islandWeatherBinding = (sources: readonly WeatherSourceStatus[], source: "" | WeatherSource, cityId: string): IslandWeatherBinding => {
    if (!sources.length) return { state: "no-sources", message: "未配置天气 API" };
    if (!source) return { state: "no-source", message: "请选择天气 API" };
    const status = sources.find((item) => item.source === source);
    if (!status) return { state: "source-removed", message: "天气 API 已删除" };
    if (!cityId) return { state: "no-city", message: "请选择城市" };
    const city = status.cities.find((item) => item.id === cityId);
    if (!city) return { state: "city-removed", message: "城市已移除" };
    return { state: "ready", source, city };
};

/** 来源显示名只从共享来源表读取，不在浮岛另写一份名字。 */
export const islandWeatherSourceName = (source: WeatherSource) => WEATHER_SOURCES[source].name;

const temperature = (value?: number) => (value == null ? "--" : `${Math.round(value)}°`);

const dayKey = (value: string | undefined, tz: string | undefined) => {
    if (!value || !Number.isFinite(Date.parse(value))) return undefined;
    try {
        return new Intl.DateTimeFormat("zh-CN", { timeZone: tz || undefined, year: "numeric", month: "2-digit", day: "2-digit" }).format(new Date(value));
    } catch {
        return undefined;
    }
};

/** 今天取城市时区内的同一天，不以下标推断供应商日界线。 */
export const islandWeatherToday = (forecast: IslandWeatherForecast | undefined, city: WeatherCity, nowMs = Date.now()) => {
    const today = dayKey(new Date(nowMs).toISOString(), city.tz);
    return forecast?.daily?.days?.find((day) => dayKey(day.forecastStartTime, city.tz) === today);
};

/** 浮岛只显示当天摘要，当前值与最高最低温缺失时保留占位而不是整卡空白。 */
export const islandWeatherSummary = (forecast: IslandWeatherForecast | undefined, city: WeatherCity, nowMs = Date.now()) => {
    const today = islandWeatherToday(forecast, city, nowMs);
    const current = forecast?.current;
    return {
        temperature: temperature(current?.temperature?.value),
        condition: current?.condition?.text?.trim() || (forecast ? "暂无天气" : "正在获取"),
        high: temperature(today?.temperatureMax?.value),
        low: temperature(today?.temperatureMin?.value),
        code: current?.condition?.code,
    };
};

/** 小时、日预报只展示有日期、天况和有限温度的未来条目，不为缺失模块补值。 */
export const islandWeatherCardForecast = (forecast: IslandWeatherForecast | undefined, city: WeatherCity, nowMs = Date.now()) => {
    if (!forecast) return { hours: [], days: [] };
    const timeZone = city.tz || undefined;
    let clock: Intl.DateTimeFormat;
    let date: Intl.DateTimeFormat;
    let calendar: Intl.DateTimeFormat;
    try {
        clock = new Intl.DateTimeFormat("zh-CN", { timeZone, hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
        date = new Intl.DateTimeFormat("zh-CN", { timeZone, month: "numeric", day: "numeric" });
        calendar = new Intl.DateTimeFormat("sv-SE", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" });
    } catch {
        return { hours: [], days: [] };
    }
    const hours = [];
    const days = [];
    for (const hour of forecast.moduleErrors?.hourly ? [] : forecast.hourly?.hours ?? []) {
        const start = Date.parse(hour.forecastTime ?? "");
        const value = hour.temperature?.value;
        const condition = hour.condition?.text?.trim();
        const code = hour.condition?.code;
        if (!Number.isFinite(start) || start <= nowMs || !Number.isFinite(value) || !condition || !code?.trim()) continue;
        hours.push({ start, dateTime: new Date(start).toISOString(), time: clock.format(start), temperature: temperature(value), condition, code });
    }
    const today = calendar.format(nowMs);
    for (const day of forecast.moduleErrors?.daily ? [] : forecast.daily?.days ?? []) {
        const start = Date.parse(day.forecastStartTime ?? "");
        const high = day.temperatureMax?.value;
        const low = day.temperatureMin?.value;
        const condition = day.daytime?.condition?.text?.trim();
        if (!Number.isFinite(start) || !Number.isFinite(high) || !Number.isFinite(low) || !condition) continue;
        const dateTime = calendar.format(start);
        if (dateTime < today) continue;
        days.push({ start, dateTime, date: date.format(start), condition, high: temperature(high), low: temperature(low) });
    }
    return {
        hours: hours.sort((a, b) => a.start - b.start).slice(0, 6),
        days: days.sort((a, b) => a.start - b.start).slice(0, 7),
    };
};

/** 预报绑定切换时不要把旧城市的天气送到新锚点。温度属于主信息文案，不是状态数值附加项。 */
export const islandWeatherProjection = (binding: IslandWeatherBinding | undefined, forecast: IslandWeatherForecast | undefined, failure?: string, nowMs = Date.now()): {
    unit: IslandContentUnit;
    summary?: ReturnType<typeof islandWeatherSummary>;
    detail?: string;
} => {
    if (!binding) return { unit: { status: failure ? "failed" : "indeterminate", primary: failure ?? "正在读取天气" } };
    if (binding.state !== "ready") return { unit: { status: failure ? "failed" : "disconnected", primary: failure ?? binding.message } };
    const current = forecast?.source === binding.source && forecast.city?.id === binding.city.id ? forecast : undefined;
    const summary = islandWeatherSummary(current, binding.city, nowMs);
    const detail = current ? `${binding.city.name} · 今天 ${summary.low}–${summary.high}` : binding.city.name;
    return { unit: {
        status: failure ? "failed" : current ? "pending" : "indeterminate",
        primary: failure ?? (current ? `${summary.temperature} ${summary.condition}` : "正在获取天气"),
    }, summary, detail };
};
