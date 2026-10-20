import {
    parseWeatherSettingsSnapshot,
    type WeatherCity,
    type WeatherSettingsCommand,
    type WeatherSettingsSnapshot,
    type WeatherSource,
    type WeatherSourceStatus,
    type WeatherTileConfig,
} from "../contracts/weather";
import { invokeRuntime, listenRuntime } from "./runtime";

/** 配置失效通知不携带凭据或缓存内容。 */
export const listenWeatherConfiguration = (onChange: () => void) => listenRuntime("weather-configuration-changed", onChange);

/** 先接收更新再登记活动实例；相同来源与城市由 Rust 共享调度。 */
export const listenWeatherForecast = async (instanceId: string, monitorId: string, config: WeatherTileConfig, onForecast: (value: unknown) => void) => {
    const stop = await listenRuntime("weather-forecast-changed", onForecast);
    try {
        const subscription = await invokeRuntime<number>("subscribe_weather", { instanceId, monitorId, source: config.source, cityId: config.cityId });
        return () => {
            stop();
            void invokeRuntime("unsubscribe_weather", { subscription }).catch(() => undefined);
        };
    } catch (error) {
        stop();
        throw error;
    }
};

/** 统一把宿主拒绝值转成带说明的错误；两种通道都可能返回字符串或错误对象。 */
const failure = (error: unknown, fallback: string): Error => {
    if (error instanceof Error) return error;
    if (typeof error === "string") return new Error(error);
    if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return new Error(error.message);
    return new Error(fallback);
};

/** 敏感材料只通过设置命令提交，不进入事件广播和普通偏好。 */
export const weatherSettings = async (command: WeatherSettingsCommand): Promise<WeatherSettingsSnapshot> =>
    parseWeatherSettingsSnapshot(await invokeRuntime("weather_settings", { command }));

/** 完成一次天气读取，拒绝值统一转成带说明的错误。 */
const read = async <T>(command: string, payload: Record<string, unknown> | undefined, fallback: string): Promise<T> => {
    try {
        return await invokeRuntime<T>(command, payload);
    } catch (error) {
        throw failure(error, fallback);
    }
};

/** 获取格子绑定的来源与城市标识对应的预报整包；城市对象由 Rust 解析。 */
export const getWeatherForecast = (source: WeatherSource, cityId: string): Promise<unknown> => read("get_weather_forecast", { source, cityId }, "天气请求失败");

/** 读取已配置来源列表，未配置来源不出现在结果中。 */
export const listWeatherSources = (): Promise<WeatherSourceStatus[]> => read("list_weather_sources", undefined, "天气来源读取失败");

/** 读取某来源已添加的城市列表。 */
export const getWeatherCities = (source: WeatherSource): Promise<WeatherCity[]> => read("get_weather_cities", { source }, "天气城市读取失败");

/** 使用已保存凭据搜索某来源的城市，搜索结果不持久化。 */
export const searchWeatherCities = (source: WeatherSource, query: string): Promise<WeatherCity[]> =>
    read("search_weather_cities", { source, query }, "城市搜索失败");

/** 按来源验证候选城市；候选来自搜索结果，由 Rust 按该来源规则校验。 */
export const validateWeatherCity = (source: WeatherSource, city: WeatherCity): Promise<WeatherCity> =>
    read("validate_weather_city", { source, city }, "城市验证失败");

/** 把候选城市加入某来源的城市列表，返回更新后的完整列表；城市对象由 Rust 按来源规则校验。 */
export const addWeatherCity = (source: WeatherSource, city: WeatherCity): Promise<WeatherCity[]> =>
    read("add_weather_city", { source, city }, "天气城市保存失败");

/** 按标识从某来源的城市列表移除一项，返回更新后的完整列表；不改变任何格子的绑定。 */
export const removeWeatherCity = (source: WeatherSource, cityId: string): Promise<WeatherCity[]> =>
    read("remove_weather_city", { source, cityId }, "天气城市删除失败");

/** 创建绑定来源与城市的天气格子；不传显示器时由 Rust 使用主显示器。 */
export const createWeatherTile = (source: WeatherSource, cityId: string, monitorId?: string): Promise<string> =>
    read("create_weather_tile", { source, cityId, monitorId: monitorId ?? null }, "天气格子创建失败");
