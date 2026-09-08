import { parsePreferences, parseSettingsSnapshot, type ApplicationPreferences } from "../contracts/settings";
import { invokeRuntime, listenRuntime } from "./runtime";

/** 校验系统颜色事件和读取命令的边界数据。 */
const parseWindowsAccentColor = (value: unknown): string => {
    if (typeof value !== "string" || !/^#[0-9a-f]{6}$/i.test(value)) throw new Error("Windows 强调色数据无效");
    return value;
};

/** 读取 Windows 实际强调色，不修改用户自定义颜色。 */
export const getWindowsAccentColor = async () => parseWindowsAccentColor(await invokeRuntime<unknown>("get_windows_accent_color"));

/** 订阅系统强调色变化并校验跨运行时载荷。 */
export const listenWindowsAccentColor = (onColor: (color: string) => void, onError: (error: unknown) => void) =>
    listenRuntime<unknown>("windows-accent-color-changed", (payload) => {
        try {
            onColor(parseWindowsAccentColor(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });

/** 将 Tauri 和原生 transport 的错误转换为可显示说明。 */
export const settingsErrorMessage = (error: unknown): string => {
    if (error instanceof Error) return error.message;
    if (typeof error === "string") return error;
    if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
    return "设置操作失败";
};

/** 读取 Rust 保存的应用偏好。 */
export const getPreferences = async () => parsePreferences(await invokeRuntime<unknown>("get_preferences"));

/** 读取设置页的偏好、启动状态和版本。 */
export const getSettings = async () => parseSettingsSnapshot(await invokeRuntime<unknown>("get_settings"));

/** 提交完整偏好并返回保存后的权威状态。 */
export const savePreferences = async (preferences: ApplicationPreferences) =>
    parsePreferences(await invokeRuntime<unknown>("save_preferences", { preferences }));

/** 修改 Windows 启动条目并校验真实返回状态。 */
export const setAutostart = async (enabled: boolean) => {
    const result = await invokeRuntime<unknown>("set_autostart", { enabled }); // Windows 写入后回读的启用状态。
    if (typeof result !== "boolean") throw new Error("开机启动返回状态无效");
    return result;
};

/** 打开后端确定的当前日志目录。 */
export const openLogs = () => invokeRuntime<void>("open_logs");

/** 订阅并校验当前运行时的偏好变化。 */
export const listenPreferences = (onPreferences: (preferences: ApplicationPreferences) => void, onError: (error: unknown) => void) =>
    listenRuntime<unknown>("preferences-changed", (payload) => {
        try {
            onPreferences(parsePreferences(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
