/** 插件在 renderTile 期间注册的一条右键菜单项。 */
export interface PluginMenuEntry {
    id: string; // 插件内稳定标识，同一实例内去重。
    label: string; // 用户可见命令。
    disabled?: boolean; // 当前是否置灰。
    description?: string; // 禁用原因等辅助说明。
    separatorBefore?: boolean; // 与上一组分离。
    danger?: boolean; // 删除类命令。
    onSelect: () => void; // 点击回调，只能调用插件自己的逻辑。
}

const menuItems = new Map<string, PluginMenuEntry[]>(); // 每个实例当前一轮注册的菜单项。
const listeners = new Map<string, Set<() => void>>(); // 菜单项变化的宿主订阅。

/** 过滤非法项：标识或标题为空、点击不是函数的项丢弃；重复标识保留首项。 */
export const normalizePluginMenuItems = (items: readonly unknown[], onWarn: (message: string) => void = (message) => console.warn(message)): PluginMenuEntry[] => {
    const seen = new Set<string>(); // 本组已保留的标识。
    const normalized: PluginMenuEntry[] = []; // 通过校验的菜单项。
    for (const entry of items) {
        if (typeof entry !== "object" || entry === null) continue;
        const item = entry as Record<string, unknown>; // 待校验的菜单项。
        if (typeof item.id !== "string" || !item.id || typeof item.label !== "string" || !item.label || typeof item.onSelect !== "function") {
            onWarn("插件菜单项缺少标识、标题或回调，已忽略");
            continue;
        }
        if (seen.has(item.id)) {
            onWarn(`插件菜单项标识重复，保留首项：${item.id}`);
            continue;
        }
        seen.add(item.id);
        normalized.push({
            id: item.id,
            label: item.label,
            disabled: item.disabled === true,
            description: typeof item.description === "string" ? item.description : undefined,
            separatorBefore: item.separatorBefore === true,
            danger: item.danger === true,
            onSelect: item.onSelect as () => void,
        });
    }
    return normalized;
};

/** 通知某个实例的宿主重新读取菜单。 */
const emit = (instanceId: string) => {
    for (const listener of listeners.get(instanceId) ?? []) listener();
};

/** 用最近一轮 renderTile 注册的菜单项替换该实例的旧列表。 */
export const setPluginMenuItems = (instanceId: string, items: readonly unknown[], onWarn?: (message: string) => void) => {
    menuItems.set(instanceId, normalizePluginMenuItems(items, onWarn));
    emit(instanceId);
};

/** 清空该实例的菜单项；与 renderTile 清理函数同一生命周期。 */
export const clearPluginMenuItems = (instanceId: string) => {
    if (!menuItems.delete(instanceId)) return;
    emit(instanceId);
};

/** 读取该实例当前的菜单项；宿主初次渲染时使用。 */
export const readPluginMenuItems = (instanceId: string): PluginMenuEntry[] => menuItems.get(instanceId) ?? [];

/** 订阅该实例的菜单项变化，返回退订函数。 */
export const subscribePluginMenuItems = (instanceId: string, listener: () => void) => {
    let set = listeners.get(instanceId);
    if (!set) {
        set = new Set();
        listeners.set(instanceId, set);
    }
    set.add(listener);
    return () => {
        set.delete(listener);
        if (set.size === 0) listeners.delete(instanceId);
    };
};
