import type { ApplicationPreferences } from "../../contracts/settings.ts";
import { applyThemeSelection } from "./applyThemeSelection.ts";

let failedHref = ""; // 加载失败的地址不再重挂，避免快照反复推送打成循环。

/** 主题样式表地址。stylesheet 已是插件根相对路径，不再另拼目录。 */
const themeStylesheetHref = (theme: { stylesheet: string; version: string }) => `/plugins/${theme.stylesheet}?v=${theme.version}`;

/** 读取已校验偏好的主题标识，null 表示内置主题。 */
export const readThemePluginId = (preferences: Pick<ApplicationPreferences, "themePluginId"> | undefined) => {
    const id = preferences?.themePluginId; // 直接读取正式契约字段。
    return typeof id === "string" && id ? id : undefined;
};

/** 样式装配与失效回退共用的清单投影。 */
type ThemeSnapshot = { manifests: readonly { id: string; role?: string; stylesheet?: string; version?: string }[] };

/** 区分未就绪、内置主题与三类失效，不读取 entry 或加载脚本。 */
export const resolveTheme = (
    snapshot: ThemeSnapshot | undefined,
    preferences: Pick<ApplicationPreferences, "themePluginId" | "disabledPlugins"> | undefined,
): { stylesheet: { stylesheet: string; version: string } | null; reason: "disabled" | "removed" | "unavailable" | null } => {
    const id = readThemePluginId(preferences); // 当前已保存的主题标识。
    if (!id) return { stylesheet: null, reason: null };
    if (preferences?.disabledPlugins.includes(id)) return { stylesheet: null, reason: "disabled" };
    if (!snapshot) return { stylesheet: null, reason: null };
    const manifest = snapshot.manifests.find((item) => item.id === id); // 未找到与非法清单使用不同提示。
    if (!manifest) return { stylesheet: null, reason: "removed" };
    if (manifest.role === "theme" && manifest.stylesheet)
        return { stylesheet: { stylesheet: manifest.stylesheet, version: manifest.version ?? "" }, reason: null };
    return { stylesheet: null, reason: "unavailable" };
};

/** 装配只消费统一判定结果，回退路径使用同一个解析器。 */
export const activeThemeStylesheet = (
    snapshot: ThemeSnapshot | undefined,
    preferences: Pick<ApplicationPreferences, "themePluginId" | "disabledPlugins"> | undefined,
) => resolveTheme(snapshot, preferences).stylesheet;

/** 每文档记录已处理的失效状态，不承担跨窗口互斥。 */
export interface ThemeRecoveryState {
    key: string;
}

/**
 * 桌面宿主注入 `window.__DESKTOP_TOOLS_NATIVE__`，这条通道的命令表没有 `save_preferences`。
 * 设置窗没有该桥，命令走 Tauri。按桥是否存在判断能不能写偏好，不看窗口标题或地址。
 */
export const canSavePreferences = () => globalThis.window?.__DESKTOP_TOOLS_NATIVE__ == null;

/** 没有写入能力的文档不发起还原，也不把跳过记成成功。有能力时保存失败仍原样抛出。 */
export const startThemeRecovery = (
    state: ThemeRecoveryState,
    snapshot: ThemeSnapshot | undefined,
    preferences: ApplicationPreferences | undefined,
    savePreferences: (next: ApplicationPreferences) => Promise<ApplicationPreferences>,
) => {
    if (!canSavePreferences()) return Promise.resolve(null);
    return recoverUnavailableTheme(state, snapshot, preferences, savePreferences);
};

/** 每次连续失效只提交一次；成功结果由调用方接收，失败不修改内存偏好。 */
export const recoverUnavailableTheme = async (
    state: ThemeRecoveryState,
    snapshot: ThemeSnapshot | undefined,
    preferences: ApplicationPreferences | undefined,
    savePreferences: (next: ApplicationPreferences) => Promise<ApplicationPreferences>,
) => {
    const { reason } = resolveTheme(snapshot, preferences); // 与样式表选择共用判定。
    if (!preferences || !reason) {
        state.key = "";
        return null;
    }
    const key = `${preferences.themePluginId}:${reason}`; // 修订改变但失效原因相同不重复提交。
    if (state.key === key) return null;
    state.key = key;
    const saved = await applyThemeSelection(preferences, null, savePreferences); // 永远写入清空 id 的完整回退结果。
    const label = { disabled: "主题已停用", removed: "主题已移除", unavailable: "主题不可用" }[reason]; // 三类用户可见原因。
    return { preferences: saved, message: `${label}，已还原原有外观并切换回默认主题。` };
};

/** 挂载或摘除唯一的主题样式表。null 时移除残留，回到内置主题。加载失败时摘掉该表并回调一次。 */
export const applyThemeStylesheet = (theme: { stylesheet: string; version: string } | null, onError?: (message: string) => void) => {
    const root = globalThis.document; // 每份文档各自挂自己的那一张表。
    const head = root.head; // 样式表只进 head。
    if (!head) return;
    const links = [...root.querySelectorAll("link[data-theme-plugin]")]; // 调用前已存在的主题 link。
    if (!theme) {
        failedHref = "";
        for (const link of links) link.remove();
        return;
    }
    const href = themeStylesheetHref(theme); // 带版本的同源地址。
    if (href === failedHref) {
        for (const link of links) link.remove();
        return;
    }
    if (links.length === 1 && links[0].getAttribute("href") === href) return;
    const link = root.createElement("link"); // 新表。先挂上，再摘旧表。
    link.rel = "stylesheet";
    link.setAttribute("href", href);
    link.setAttribute("data-theme-plugin", "");
    link.onerror = () => {
        if (!link.isConnected) return;
        failedHref = href;
        link.remove();
        onError?.("主题样式表加载失败，已回退到内置主题。");
    };
    head.appendChild(link);
    for (const old of links) old.remove();
};
