import { parseComponentRuntimeSnapshot } from "../contracts/component";
import { parsePluginInstallBatch, parsePluginInstallProgress, type PluginInstallPlan, type PluginInstallProgress } from "../contracts/plugin";

/** Tauri 的 Err 是字符串，不是 Error。字符串原样用，Error 取 message。 */
export const commandErrorMessage = (error: unknown) => {
    if (typeof error === "string" && error.trim()) return error.trim();
    if (error instanceof Error && error.message.trim()) return error.message.trim();
    return String(error);
};
import { getPreferences } from "./settings";
import { invokeRuntime, listenRuntime } from "./runtime";

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

const accepted = new Map<string, PluginShellApi>(); // 每个清单标识保留的首份外壳。
const entryShapeErrors = new Map<string, string>(); // 外壳在，但 settingsEntries 不是数组。
const shellListeners = new Set<(pluginId: string) => void>(); // 注册成功后的订阅。
const loadedScripts = new Set<string>(); // 设置窗口已成功执行的脚本。
const failedScripts = new Map<string, string>(); // 设置窗口脚本失败，不自动重试。
const scriptListeners = new Set<(pluginId: string) => void>(); // 脚本执行成功后的订阅。
const loadingScripts = new Map<string, Promise<string | undefined>>(); // 同一标识的在途加载。

/** 清单标识由加载方传入。脚本不再自己写注册键。 */
export const adoptPluginManifests = (_manifests: readonly { id: string }[]) => undefined;

/** 读取已按清单标识绑定的首份外壳。 */
export const readPluginApi = (pluginId: string) => accepted.get(pluginId);

/** 用清单标识绑定一份外壳。同一标识只保留首份。 */
export const bindPluginApi = (pluginId: string, value: unknown) => {
    if (accepted.has(pluginId)) {
        console.warn(`插件重复注册，保留首份：${pluginId}`);
        return false;
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) {
        console.warn(`插件导出不是对象：${pluginId}`);
        return false;
    }
    accepted.set(pluginId, normalizeSettingsEntries(pluginId, value as PluginShellApi));
    emitPluginShell(pluginId);
    return true;
};

/** 像外壳的普通对象。数组和原始值都不是。 */
const asShell = (value: unknown) => (value && typeof value === "object" && !Array.isArray(value) ? (value as PluginShellApi) : undefined);

/** 从模块取出外壳。条目在 default 上就用 default；只挂在命名空间上时补回来。 */
export const shellFromModule = (loaded: unknown) => {
    if (!loaded || typeof loaded !== "object" || Array.isArray(loaded)) return { ok: false as const, message: "插件模块没有默认导出" };
    const record = loaded as PluginShellApi & { default?: unknown }; // 动态 import 的命名空间。
    const namespace = asShell(loaded); // 命名导出落在这一层。
    if (!("default" in record) || record.default === undefined)
        return namespace ? { ok: true as const, value: namespace } : { ok: false as const, message: "插件模块没有默认导出" };
    const exported = asShell(record.default); // 默认导出才是作者写的外壳。
    if (!exported) return { ok: false as const, message: "插件导出不是对象" };
    if (exported.settingsEntries !== undefined || namespace?.settingsEntries === undefined) return { ok: true as const, value: exported };
    return { ok: true as const, value: { ...exported, settingsEntries: namespace.settingsEntries } };
};

/** 已绑定，但 settingsEntries 的形状不合法。没有这条记录表示没导出或已经是数组。 */
export const pluginSettingsShape = (pluginId: string) => entryShapeErrors.get(pluginId);

/** 这个窗口里脚本已经执行过。再次进入不必重新 import。 */
export const pluginScriptLoaded = (pluginId: string) => loadedScripts.has(pluginId);

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

/** 丢掉不合格的设置条目，不影响 renderTile。不是数组时留下可区分的原因。 */
const normalizeSettingsEntries = (pluginId: string, value: PluginShellApi): PluginShellApi => {
    entryShapeErrors.delete(pluginId);
    if (value.settingsEntries === undefined) return value;
    if (!Array.isArray(value.settingsEntries)) {
        const message = "插件设置条目格式无效：不是数组"; // 页面要和「没导出」分开。
        entryShapeErrors.set(pluginId, message);
        console.warn(`${message}：${pluginId}`);
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
        bindPluginApi(key, value); // 测试和宿主都按清单标识写入。
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

/** 测试可换成假的模块加载。生产走同源动态 import。 */
export const pluginModules = {
    load: (specifier: string): Promise<unknown> => import(/* @vite-ignore */ specifier),
};

/** 插件根相对路径变成同源地址。空路径、绝对路径和 .. 都不加载。 */
const pluginScriptSpecifier = (script: string) => {
    const parts = script.split("/"); // 只接受相对片段。
    if (!script || script.startsWith("/") || parts.some((part) => !part || part === "." || part === "..")) return undefined;
    return `/plugins/${script}`;
};

/** 加载插件模块。停用的不加载；同一标识成功或失败后都不重试。reload 清掉记录后再导入。返回失败说明。 */
export const loadPluginScript = (pluginId: string, script = "", reload = false): Promise<string | undefined> => {
    if (reload) {
        failedScripts.delete(pluginId);
        loadedScripts.delete(pluginId);
    }
    if (loadedScripts.has(pluginId)) return Promise.resolve(undefined); // 已绑定的对象直接复用，不再导入。
    const failure = failedScripts.get(pluginId); // 已经失败的结果。
    if (failure) return Promise.resolve(failure);
    const pending = loadingScripts.get(pluginId); // 同一标识正在加载。
    if (pending) return pending;
    const task = (async () => {
        const disabled = (await getPreferences()).disabledPlugins; // 停用名单以当前偏好为准。
        if (disabled.includes(pluginId)) return undefined;
        const specifier = pluginScriptSpecifier(script); // 同源模块地址。
        if (!specifier) {
            const message = "插件脚本路径无效"; // 清单没给可用的相对路径。
            failedScripts.set(pluginId, message);
            return message;
        }
        try {
            const shell = shellFromModule(await pluginModules.load(specifier)); // 默认导出，按清单标识绑定。
            if (!shell.ok) {
                failedScripts.set(pluginId, shell.message);
                return shell.message;
            }
            bindPluginApi(pluginId, shell.value);
        } catch (error) {
            const detail = commandErrorMessage(error); // 模块错误原样留下，字符串拒绝也不丢掉。
            const message = detail ? `插件模块加载失败：${detail}` : "插件模块加载失败";
            failedScripts.set(pluginId, message);
            console.warn(`插件模块加载失败：${pluginId}`, error);
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

/** 在当前桌面创建一个该插件的格子。 */
export const createPluginComponent = (pluginId: string) => invokeRuntime<void>("create_plugin_component", { pluginId });

/** 选择插件目录。取消返回 undefined，选中默认目录时返回空串。 */
export const choosePluginDirectory = async () => {
    const result = await invokeRuntime<unknown>("choose_plugin_directory"); // 取消时后端给空。
    if (result === null || result === undefined) return undefined;
    if (typeof result !== "string") throw new Error("插件目录无效");
    return result;
};

/** 本次启动实际使用的插件目录，不跟后来改过的偏好。 */
export const resolvedPluginDirectory = async () => {
    const result = await invokeRuntime<unknown>("resolved_plugin_directory"); // 冻结路径。
    if (typeof result !== "string" || !result.trim()) throw new Error("插件目录无效");
    return result;
};

/** 选择结果。plans 去排队，errors 是每条独立原因。空的两边表示取消。 */
export const pluginInstallPlans = (value: unknown) => parsePluginInstallBatch(value);

/** 选择文件夹。返回计划与逐条错误。 */
export const preparePluginInstallsFromFolder = () => invokeRuntime<unknown>("prepare_plugin_install_from_folder").then(pluginInstallPlans);

/** 选择压缩包。返回计划与逐条错误。 */
export const preparePluginInstallsFromArchive = () => invokeRuntime<unknown>("prepare_plugin_install_from_archive").then(pluginInstallPlans);

/** 安装结果。旧后端只给插件标识；带 snapshot 时设置页不用再等事件。 */
export const parsePluginInstallApplied = (value: unknown) => {
    if (typeof value === "string" && value) return { pluginId: value };
    if (!value || typeof value !== "object") throw new Error("插件安装结果无效");
    const record = value as Record<string, unknown>; // 可能同时带标识和快照。
    const pluginId = typeof record.pluginId === "string" && record.pluginId ? record.pluginId : undefined;
    const rawSnapshot = record.snapshot !== undefined ? record.snapshot : Array.isArray(record.manifests) ? value : undefined;
    if (rawSnapshot === undefined || rawSnapshot === null) {
        if (!pluginId) throw new Error("插件安装结果无效");
        return { pluginId };
    }
    const snapshot = parseComponentRuntimeSnapshot(rawSnapshot); // 形状不对就抛，调用方再重读。
    return pluginId ? { pluginId, snapshot } : { snapshot };
};

/** 执行已确认的计划。降级必须带确认。优先带回刷新后的快照。 */
export const applyPluginInstall = async (planId: string, confirmDowngrade: boolean) => {
    const result = await invokeRuntime<unknown>("apply_plugin_install", { planId, confirmDowngrade }); // 标识，或带快照的对象。
    return parsePluginInstallApplied(result);
};

/** 让进行中的安装停下来。 */
export const cancelPluginInstall = (taskId: string) => invokeRuntime<void>("cancel_plugin_install", { taskId });

/** 订阅安装进度。返回退订函数。 */
export const listenPluginInstallProgress = (onProgress: (progress: PluginInstallProgress) => void) =>
    listenRuntime<unknown>("plugin-install-progress", (payload) => onProgress(parsePluginInstallProgress(payload)));

export type { PluginInstallPlan, PluginInstallProgress };

/** 在资源管理器中选中该插件的清单。 */
export const openPluginDirectory = (pluginId: string) => invokeRuntime<void>("open_plugin_directory", { pluginId });
