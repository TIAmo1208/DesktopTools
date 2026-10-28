import { getPreferences } from "./settings";
import { invokeRuntime } from "./runtime";

/** 传给作者渲染函数的外壳状态。格子和设置条目共用。 */
export interface PluginShellContext {
    pluginId: string; // 清单标识。
    instanceId: string; // 当前格子实例；设置条目可以不对应单个格子。
    collapsed: boolean; // 外框实际折叠状态。
    settings: unknown; // 读取命令给出的设置，没有文件时为 null。
    saveSettings: (settings: unknown) => Promise<void>; // 调用保存命令。
}

/** 一个插件自己的设置行。 */
export interface PluginSettingsEntry {
    id: string; // 插件内唯一。
    name: string; // 行标题。
    render: (container: HTMLElement, context: PluginShellContext) => void | (() => void); // 条目内容，可返回清理函数。
}

/** 插件脚本注册的外壳对象。 */
export interface PluginShellApi {
    renderTile?: (container: HTMLElement, context: PluginShellContext) => void | (() => void); // 格子内容，可返回清理函数。
    settingsEntries?: PluginSettingsEntry[]; // 没有该字段表示没有设置条目。
}

declare global {
    interface Window {
        __DESKTOP_TOOLS_PLUGINS__?: Record<string, PluginShellApi>; // 按插件标识注册的外壳对象。
    }
}

const knownPluginIds = new Set<string>(); // 当前允许注册的 plugin. 标识。
const accepted = new Map<string, PluginShellApi>(); // 每个标识保留的首份注册。
const shellListeners = new Set<(pluginId: string) => void>(); // 注册成功后的订阅。
const loadedScripts = new Set<string>(); // 设置窗口已成功执行的脚本。
const failedScripts = new Map<string, string>(); // 设置窗口脚本失败，不自动重试。
const scriptListeners = new Set<(pluginId: string) => void>(); // 脚本执行成功后的订阅。
const loadingScripts = new Map<string, Promise<string | undefined>>(); // 同一标识的在途加载。

/** 页面持有注册表；插件按约定往上面赋值。 */
export const publishPluginShell = () => {
    const page = globalThis.window; // 设置窗口与逐屏各自持有一份注册表。
    if (!page) return;
    page.__DESKTOP_TOOLS_PLUGINS__ ??= pluginShell;
};

/** 用当前快照替换可注册的插件标识。 */
export const adoptPluginManifests = (manifests: readonly { id: string }[]) => {
    publishPluginShell();
    knownPluginIds.clear();
    for (const manifest of manifests) if (manifest.id.startsWith("plugin.")) knownPluginIds.add(manifest.id);
};

/** 读取已接受的首份注册。 */
export const readPluginApi = (pluginId: string) => accepted.get(pluginId);

/** 订阅外壳注册。返回退订函数。 */
export const subscribePluginShell = (onPlugin: (pluginId: string) => void) => {
    shellListeners.add(onPlugin);
    return () => {
        shellListeners.delete(onPlugin);
    };
};

/** 通知外壳注册订阅者。 */
const emitPluginShell = (pluginId: string) => {
    for (const listener of shellListeners) listener(pluginId);
};

/** 丢掉不合格的设置条目，不影响 renderTile。 */
const normalizeSettingsEntries = (pluginId: string, value: PluginShellApi): PluginShellApi => {
    if (value.settingsEntries === undefined) return value;
    if (!Array.isArray(value.settingsEntries)) {
        console.warn(`插件设置条目无效：${pluginId}`);
        const rest = { ...value }; // 保留格子渲染，丢掉整组条目。
        delete rest.settingsEntries;
        return rest;
    }
    const seen = new Set<string>(); // 本插件已经保留的条目标识。
    const settingsEntries: PluginSettingsEntry[] = []; // 合格条目。
    for (const entry of value.settingsEntries) {
        const id = entry && typeof entry === "object" ? entry.id : undefined; // 条目标识。
        const name = entry && typeof entry === "object" ? entry.name : undefined; // 行标题。
        if (typeof id !== "string" || id.length === 0 || typeof name !== "string" || name.length === 0) {
            console.warn(`插件设置条目缺少标识或标题：${pluginId}`);
            continue;
        }
        if (seen.has(id)) {
            console.warn(`插件设置条目标识重复：${pluginId} ${id}`);
            continue;
        }
        seen.add(id);
        settingsEntries.push(entry);
    }
    return { ...value, settingsEntries };
};

export const pluginShell: Record<string, PluginShellApi> = new Proxy({} as Record<string, PluginShellApi>, {
    get: (_target, key) => (typeof key === "string" ? accepted.get(key) : undefined),
    set: (_target, key, value: PluginShellApi) => {
        if (typeof key !== "string") return false;
        if (!knownPluginIds.has(key)) {
            console.warn(`插件注册了未知标识：${key}`);
            return true;
        }
        if (accepted.has(key)) {
            console.warn(`插件重复注册，保留首份：${key}`);
            return true;
        }
        if (typeof value !== "object" || value === null) {
            console.warn(`插件注册对象无效：${key}`);
            return true;
        }
        accepted.set(key, normalizeSettingsEntries(key, value));
        emitPluginShell(key);
        return true;
    },
});

/** 查询设置窗口脚本失败原因；没有失败记录时为空。 */
export const pluginScriptFailure = (pluginId: string) => failedScripts.get(pluginId);

/** 订阅设置窗口脚本执行成功。返回退订函数。 */
export const listenPluginScripts = (onScript: (pluginId: string) => void) => {
    scriptListeners.add(onScript);
    return () => {
        scriptListeners.delete(onScript);
    };
};

/** 在设置窗口执行插件脚本。停用的不加载；同一标识成功或失败后都不重试。返回失败说明。 */
export const loadPluginScript = (pluginId: string): Promise<string | undefined> => {
    if (loadedScripts.has(pluginId)) return Promise.resolve(undefined);
    const failure = failedScripts.get(pluginId); // 已经失败的结果。
    if (failure) return Promise.resolve(failure);
    const pending = loadingScripts.get(pluginId); // 同一标识正在执行。
    if (pending) return pending;
    const task = (async () => {
        const disabled = (await getPreferences()).disabledPlugins; // 停用名单以当前偏好为准。
        if (disabled.includes(pluginId)) return undefined;
        publishPluginShell();
        knownPluginIds.add(pluginId);
        try {
            await invokeRuntime("load_plugin_script", { pluginId });
        } catch (error) {
            const message = error instanceof Error ? error.message : "插件脚本加载失败"; // 可再次查询的失败说明。
            failedScripts.set(pluginId, message);
            console.warn(`插件脚本加载失败：${pluginId}`, error);
            return message;
        }
        loadedScripts.add(pluginId);
        for (const listener of scriptListeners) listener(pluginId);
        return undefined;
    })().finally(() => loadingScripts.delete(pluginId));
    loadingScripts.set(pluginId, task);
    return task;
};

/** 卸载插件。removeSettings 为真时同时删掉插件配置，返回实际删掉的文件名。 */
export const uninstallPlugin = async (pluginId: string, removeSettings = false) => {
    const result = await invokeRuntime<unknown>("uninstall_plugin", { pluginId, removeSettings }); // 后端只列出本次删除的文件名。
    if (typeof result !== "object" || result === null || !Array.isArray((result as { removed?: unknown }).removed)) throw new Error("插件卸载结果无效");
    const removed = (result as { removed: unknown[] }).removed; // 待确认的文件名。
    if (removed.some((name) => typeof name !== "string")) throw new Error("插件卸载结果无效");
    return removed as string[];
};

/** 在资源管理器中选中该插件的清单。 */
export const openPluginDirectory = (pluginId: string) => invokeRuntime<void>("open_plugin_directory", { pluginId });
