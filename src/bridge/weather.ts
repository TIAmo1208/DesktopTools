import type { WeatherSettingsCommand, WeatherSettingsSnapshot } from "../contracts/weather";
import { invokeRuntime } from "./runtime";

/** 敏感材料只通过设置命令提交，不进入事件广播和普通偏好。 */
export const weatherSettings = (command: WeatherSettingsCommand): Promise<WeatherSettingsSnapshot> => invokeRuntime("weather_settings", { command });
