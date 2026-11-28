import type { ComponentTileSize } from "./component.ts";
import { pluginAssetPath } from "./plugin.ts";

/** 作者只指定本地视图身份，不提供内容地址、脚本或原生窗口标签。尺寸为 DIP。 */
export interface PluginWindowRequest {
    view: string;
    windowKey: string;
    title: string;
    size: ComponentTileSize;
    minSize?: ComponentTileSize;
    data?: unknown;
}

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const identity = (value: unknown, length: number): value is string => typeof value === "string" && value.length > 0 && value.length <= length && /^[a-zA-Z0-9_.:-]+$/.test(value);
const dimension = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value > 0 && value <= 16_384;
const size = (value: unknown): value is ComponentTileSize => record(value) && dimension(value.width) && dimension(value.height);

/** 在进入原生桥前拒绝非法请求；宿主独立复验授权、尺寸和 JSON 容量。 */
export const parsePluginWindowRequest = (value: unknown): PluginWindowRequest => {
    if (!record(value) || Object.keys(value).some((key) => !["view", "windowKey", "title", "size", "minSize", "data"].includes(key))
        || !identity(value.view, 64) || !identity(value.windowKey, 128) || typeof value.title !== "string" || !value.title.trim()
        || /[\u0000-\u001f\u007f-\u009f]/.test(value.title) || [...value.title].length > 128 || !size(value.size)) throw new Error("插件窗口请求无效");
    const minimum = value.minSize;
    if (minimum !== undefined && (!size(minimum) || minimum.width > value.size.width || minimum.height > value.size.height)) throw new Error("插件窗口最小尺寸无效");
    if (value.data !== undefined) {
        const json = JSON.stringify(value.data);
        if (!json || new TextEncoder().encode(json).length > 65_536) throw new Error("插件窗口数据无效或过大");
    }
    return { view: value.view, windowKey: value.windowKey, title: value.title.trim(), size: value.size, minSize: minimum, data: value.data };
};

/** 窗口身份与脚本仅取自当前原生窗口的注册表，地址栏不传作者输入。 */
export const parsePluginWindowContext = (value: unknown) => {
    if (!record(value) || typeof value.pluginId !== "string" || !value.pluginId.startsWith("plugin.") || typeof value.instanceId !== "string" || !value.instanceId
        || !identity(value.view, 64) || !identity(value.windowKey, 128) || typeof value.script !== "string" || !pluginAssetPath(value.script)
        || (value.stylesheet && (typeof value.stylesheet !== "string" || !pluginAssetPath(value.stylesheet)))) throw new Error("插件窗口身份或本地资源无效");
    return { pluginId: value.pluginId, instanceId: value.instanceId, script: value.script, stylesheet: typeof value.stylesheet === "string" ? value.stylesheet : "", view: value.view, windowKey: value.windowKey, data: value.data };
};
