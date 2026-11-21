import { readPluginApi } from "../../../bridge/pluginScripts";
import type { ComponentCapability, ComponentInstanceSnapshot, ComponentManifest, ComponentTileType } from "../../../contracts/component";
import type { PluginInstallPlan } from "../../../contracts/plugin";
import type { ApplicationPreferences } from "../../../contracts/settings";

/** 设置页里的一个插件清单。 */
export interface PluginRow {
    id: string; // 清单标识。
    name: string; // 清单显示名。
    creatable: boolean; // 清单是否允许创建格子。
    maxTiles: number; // 格子上限。0 表示不限。
    tileTypes: ComponentTileType[]; // 清单声明的格子类型。空表示隐式单类型，菜单不展开。
    version?: string; // 清单版本。没有就不展示。
    description?: string; // 插件简介。没有就不展示。
    author?: string; // 作者。没有就不拼 @。
    repository?: string; // 有仓库才显示入口，不拿来拼地址。
    script: string; // 插件根内的脚本相对路径。
    role?: ComponentManifest["role"]; // 主题只在带入口脚本时加载脚本。缺省按格子。
    island: boolean; // 清单是否允许上岛；设置条目的实例回落与能力上下文使用。
    requiredCapabilities: ComponentCapability[]; // 清单声明的能力；未声明的通用能力不暴露给设置条目。
}

/** 卸载或移除共用的确认请求。 */
export type PluginConfirmRequest =
    | { kind: "uninstall"; plugin: PluginRow; returnFocus: HTMLElement }
    | { kind: "remove"; instance: ComponentInstanceSnapshot; returnFocus: HTMLElement }
    | { kind: "install"; plan: PluginInstallPlan; returnFocus: HTMLElement };

/** 只要带 plugin. 前缀的清单，按显示名排序。 */
export const pluginRows = (manifests: readonly ComponentManifest[]): PluginRow[] =>
    manifests
        .filter((manifest) => manifest.id.startsWith("plugin."))
        .map((manifest) => ({
            id: manifest.id,
            name: manifest.name,
            creatable: manifest.creatable === true,
            maxTiles: manifest.maxTiles ?? 0,
            tileTypes: manifest.tileTypes ?? [],
            version: manifest.version,
            description: manifest.description,
            author: manifest.author,
            repository: manifest.repository,
            script: manifest.script,
            role: manifest.role,
            island: manifest.island === true,
            requiredCapabilities: manifest.requiredCapabilities,
        }))
        .sort((left, right) => left.name.localeCompare(right.name, "zh"));

const PLUGIN_DIRECTORY_DEFAULT = "程序所在目录\\plugin"; // 空偏好由后端解析实际可执行文件目录。

/** 待保存路径。空串显示默认目录。 */
export const pluginDirectoryShown = (draft: string | undefined, saved: string | undefined) => {
    const value = draft !== undefined ? draft : (saved ?? ""); // 有草稿时以草稿为准。
    return value || PLUGIN_DIRECTORY_DEFAULT;
};

/** 升级和同版本覆盖，降级要先卸载。 */
export const installConfirmLabel = (downgrade: boolean) => (downgrade ? "卸载并安装" : "覆盖安装");

/** 下方放不下且上方更宽时向上翻。内容比两边都高就内部滚动。 */
export const taskMenuPlacement = (menu: { top: number; bottom: number; height: number }, frame: { top: number; bottom: number }) => {
    const spaceAbove = menu.top - frame.top; // 菜单顶到内容区顶。
    const spaceBelow = frame.bottom - menu.top; // 菜单顶到内容区底。
    return { flipUp: menu.bottom > frame.bottom && spaceAbove > spaceBelow, scroll: menu.height > Math.max(spaceAbove, spaceBelow) };
};

/** 已注册的条目。没有注册对象时返回 undefined，有注册但没有条目时返回空数组。 */
export const entriesFor = (pluginId: string) => {
    const api = readPluginApi(pluginId); // 首份外壳对象。
    if (!api) return undefined;
    return api.settingsEntries ?? [];
};

/** 主题只有带入口脚本时才加载脚本。格子即使 script 为空也交给加载函数，由它报路径无效。 */
export const requestPluginScript = (
    plugin: { id: string; role?: string; script?: string },
    load: (id: string, script: string, reload?: boolean) => void,
    reload = false,
) => {
    if (plugin.role === "theme" && !plugin.script) return;
    load(plugin.id, plugin.script ?? "", reload);
};

/** 通知第一行是插件名，第二行是结果。宿主只收一段正文。 */
export const installNotice = (name: string, body: string) => `${name}\n${body}`;

export const installSuccessMessage = () => "安装成功";

/** 安装失败只带后端原因，不再附加重启或默认停用说明。 */
export const installFailureMessage = (reason: string) => (reason.startsWith("安装失败") ? reason : `安装失败：${reason}`);

/** 浮岛卡是否实际启用：清单声明 island、岛开启、卡在顺序里且插件未停用。 */
export const pluginIslandCardActive = (
    pluginId: string,
    island: boolean,
    preferences?: Pick<ApplicationPreferences, "islandEnabled" | "islandCardOrder" | "disabledPlugins">,
): boolean =>
    island &&
    preferences?.islandEnabled === true &&
    !preferences.disabledPlugins.includes(pluginId) &&
    preferences.islandCardOrder.includes(pluginId);

/** 二级页可用的实例身份：工作区已就绪实例优先，其次已启用的浮岛卡；都没有时为空串。 */
export const pluginCapabilityInstanceId = (workspaceInstanceId: string, pluginId: string, islandActive: boolean): string =>
    workspaceInstanceId || (islandActive ? `island.${pluginId}` : "");

/** 需要实例授权的设置条目：没有可用实例或插件停用时都不渲染。 */
export const pluginSettingsAuthorized = (instanceId: string, pluginDisabled: boolean): boolean => Boolean(instanceId) && !pluginDisabled;

/** 「添加到桌面」只在清单允许创建且不是主题时出现；没有可创建类型就不渲染。 */
export const pluginShowsAddTile = (creatable: boolean, role?: ComponentManifest["role"]): boolean => creatable === true && role !== "theme";
