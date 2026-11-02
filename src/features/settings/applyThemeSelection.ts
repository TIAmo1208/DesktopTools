import type { ComponentManifest } from "../../contracts/component.ts";
import { parsePreferences, parseThemeBaseline, type ApplicationPreferences, type ThemeBaseline } from "../../contracts/settings.ts";

/** 捕获五项原值，不改变系统强调色开关。 */
const captureBaseline = (preferences: ApplicationPreferences): ThemeBaseline => ({
    accentColor: preferences.accentColor,
    desktopAccentColor: preferences.desktopAppearance.accentColor,
    cornerRadius: preferences.cornerRadius,
    fontFamily: preferences.fontFamily,
    fontSize: preferences.fontSize,
});

/** 还原前逐字段校验旧基线，非法值使用当前版本默认值，不阻止退出主题。 */
const restoreBaseline = (baseline: ThemeBaseline): ThemeBaseline => {
    let restored = captureBaseline(parsePreferences({ theme: "system", doubleClickToOpen: true })); // 当前默认值是唯一兜底来源。
    for (const key of Object.keys(restored) as (keyof ThemeBaseline)[]) {
        try {
            restored = parseThemeBaseline({ ...restored, [key]: baseline[key] });
        } catch {
            // 旧基线的非法字段保留默认值，其余合法字段继续恢复。
        }
    }
    return restored;
};

/** 切回内置主题；停用、卸载和缺失主题共用此还原操作。 */
export const clearTheme = (preferences: ApplicationPreferences): ApplicationPreferences => {
    const baseline = preferences.themeBaseline ? restoreBaseline(preferences.themeBaseline) : captureBaseline(preferences); // 无基线时保留当前外观。
    return {
        ...preferences,
        accentColor: baseline.accentColor,
        desktopAppearance: { ...preferences.desktopAppearance, accentColor: baseline.desktopAccentColor },
        cornerRadius: baseline.cornerRadius,
        fontFamily: baseline.fontFamily,
        fontSize: baseline.fontSize,
        themePluginId: null,
        themeBaseline: null,
    };
};

/** 首次捕获基线，换主题先还原再覆盖，未声明项不继承上一主题。 */
export const selectTheme = (preferences: ApplicationPreferences, manifest: ComponentManifest): ApplicationPreferences => {
    if (manifest.role !== "theme" || !manifest.id || preferences.disabledPlugins.includes(manifest.id)) throw new Error("主题不可用");
    const base = clearTheme(preferences); // 基线原值，而非上一主题的有效外观。
    const baseline = preferences.themeBaseline ?? captureBaseline(preferences); // 换主题不重新快照。
    const appearance = manifest.themeAppearance; // null 覆盖项保留基线值。
    return parsePreferences({
        ...base,
        themePluginId: manifest.id,
        themeBaseline: baseline,
        accentColor: appearance?.accentColor ?? base.accentColor,
        desktopAppearance: { ...base.desktopAppearance, accentColor: appearance?.desktopAccentColor ?? base.desktopAppearance.accentColor },
        cornerRadius: appearance?.cornerRadius ?? base.cornerRadius,
        fontFamily: appearance?.fontFamily ?? base.fontFamily,
        fontSize: appearance?.fontSize ?? base.fontSize,
    });
};

/** 交给既有 savePreferences 一次完整提交，失败不改调用方状态。 */
export const applyThemeSelection = (
    preferences: ApplicationPreferences,
    manifest: ComponentManifest | null,
    savePreferences: (next: ApplicationPreferences) => Promise<ApplicationPreferences>,
): Promise<ApplicationPreferences> => savePreferences(manifest ? selectTheme(preferences, manifest) : clearTheme(preferences));
