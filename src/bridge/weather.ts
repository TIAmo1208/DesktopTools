import type { WeatherCity, WeatherSettingsCommand, WeatherSettingsSnapshot } from "../contracts/weather";
import { invokeRuntime, listenRuntime } from "./runtime";

/** 配置失效通知不携带凭据或缓存内容。 */
export const listenWeatherConfiguration = (onChange: () => void) => listenRuntime("weather-configuration-changed", onChange);

/** 先接收更新再登记活动实例；相同位置由 Rust 共享调度。 */
export const listenWeatherForecast = async (instanceId: string, monitorId: string, city: WeatherCity, onForecast: (value: unknown) => void) => {
    const stop = await listenRuntime("weather-forecast-changed", onForecast);
    try {
        const subscription = await invokeRuntime<number>("subscribe_weather", { instanceId, monitorId, city });
        return () => {
            stop();
            void invokeRuntime("unsubscribe_weather", { subscription }).catch(() => undefined);
        };
    } catch (error) {
        stop();
        throw error;
    }
};

/** 敏感材料只通过设置命令提交，不进入事件广播和普通偏好。 */
export const weatherSettings = (command: WeatherSettingsCommand): Promise<WeatherSettingsSnapshot> => invokeRuntime("weather_settings", { command });

/** 获取天气格子当前城市的预报整包。 */
export const getWeatherForecast = (city: WeatherCity): Promise<unknown> =>
    invokeRuntime("get_weather_forecast", { city }).catch((error: unknown) => {
        if (error instanceof Error) throw error;
        if (typeof error === "string") throw new Error(error);
        if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") throw new Error(error.message);
        throw new Error("天气请求失败");
    });

/** 使用已保存凭据搜索当前天气格子的会话城市。 */
export const searchWeatherCities = (query: string): Promise<WeatherCity[]> =>
    invokeRuntime<WeatherCity[]>("search_weather_cities", { query }).catch((error: unknown) => {
        if (error instanceof Error) throw error;
        if (typeof error === "string") throw new Error(error);
        if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") throw new Error(error.message);
        throw new Error("城市搜索失败");
    });

/** 获取后端保存的当前城市；搜索结果不持久化。 */
export const getWeatherCity = (): Promise<WeatherCity | null> => invokeRuntime<WeatherCity | null>("get_weather_city");

/** 保存当前选中的城市，供下次启动获取天气。 */
export const setWeatherCity = (city: WeatherCity): Promise<WeatherCity> => invokeRuntime<WeatherCity>("set_weather_city", { city });
