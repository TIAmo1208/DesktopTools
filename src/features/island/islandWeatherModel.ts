import { WEATHER_SOURCES, type WeatherCity, type WeatherSource, type WeatherSourceStatus } from "../../contracts/weather.ts";

/** 浮岛天气卡读取的最小预报字段；完整模块仍由桌面天气格子呈现。 */
export interface IslandWeatherForecast {
    source?: WeatherSource; // 事件载荷来源，用于区分同名城市。
    city?: { id?: string; name?: string; tz?: string }; // 当前预报位置。
    current?: { temperature?: { value?: number }; condition?: { text?: string; code?: string } };
    daily?: {
        days?: {
            forecastStartTime?: string;
            temperatureMax?: { value?: number };
            temperatureMin?: { value?: number };
            daytime?: { condition?: { text?: string; code?: string } };
        }[];
    };
    moduleFetchedAt?: Record<string, number>; // 当前天气的获取时刻。
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
export const islandWeatherToday = (forecast: IslandWeatherForecast | undefined, city: WeatherCity) => {
    const today = dayKey(new Date().toISOString(), city.tz);
    return forecast?.daily?.days?.find((day) => dayKey(day.forecastStartTime, city.tz) === today);
};

/** 浮岛只显示当天摘要，当前值与最高最低温缺失时保留占位而不是整卡空白。 */
export const islandWeatherSummary = (forecast: IslandWeatherForecast | undefined, city: WeatherCity) => {
    const today = islandWeatherToday(forecast, city);
    const current = forecast?.current;
    return {
        temperature: temperature(current?.temperature?.value),
        condition: current?.condition?.text?.trim() || (forecast ? "暂无天气" : "正在获取"),
        high: temperature(today?.temperatureMax?.value),
        low: temperature(today?.temperatureMin?.value),
        code: current?.condition?.code,
    };
};
