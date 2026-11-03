import type { ComponentManifest } from "../../contracts/component.ts";
import { APPLICATION_FONTS, parsePreferences, parseThemeBaseline, type ApplicationPreferences, type ThemeBaseline } from "../../contracts/settings.ts";

const DEFAULT_FONT = "Segoe UI Variable"; // 主题字体撤下后回到的界面字体。
const SYSTEM_FONTS = new Set<string>(APPLICATION_FONTS); // 内置字体不可能由主题带进来。

/** 主题声明的字体族；这些字体跟着主题走，不属于用户外观。 */
const themeFontsOf = (manifest: ComponentManifest | null) => new Set((manifest?.themeFonts ?? []).map((font) => font.family));

/** 已选中主题时的当前字体：非系统字体视为主题带进来的，按主题字体处理。 */
const activeThemeFonts = (preferences: ApplicationPreferences) =>
    preferences.themePluginId && !SYSTEM_FONTS.has(preferences.fontFamily) ? new Set([preferences.fontFamily]) : new Set<string>();

/** 捕获五项原值，不改变系统强调色开关；主题自带的字体替换成默认字体。 */
const captureBaseline = (preferences: ApplicationPreferences, themeFonts: ReadonlySet<string> = new Set()): ThemeBaseline => ({
    accentColor: preferences.accentColor,
    desktopAccentColor: preferences.desktopAppearance.accentColor,
    cornerRadius: preferences.cornerRadius,
    fontFamily: themeFonts.has(preferences.fontFamily) ? DEFAULT_FONT : preferences.fontFamily,
    fontSize: preferences.fontSize,
});

/** 还原前逐字段校验旧基线，非法值使用当前版本默认值，不阻止退出主题。 */
const restoreBaseline = (baseline: ThemeBaseline, currentFont: string): ThemeBaseline => {
    let restored = captureBaseline(parsePreferences({ theme: "system", doubleClickToOpen: true })); // 当前默认值是唯一兜底来源。
    for (const key of Object.keys(restored) as (keyof ThemeBaseline)[]) {
        try {
            restored = parseThemeBaseline({ ...restored, [key]: baseline[key] });
        } catch {
            // 旧基线的非法字段保留默认值，其余合法字段继续恢复。
        }
    }
    // 基线字体与当前字体相同、又不是系统字体时，它是上一主题带进来的字体，不能当成用户原值还原。
    if (restored.fontFamily === currentFont && !SYSTEM_FONTS.has(restored.fontFamily)) restored = { ...restored, fontFamily: DEFAULT_FONT };
    return restored;
};

/** 切回内置主题；停用、卸载和缺失主题共用此还原操作。 */
export const clearTheme = (preferences: ApplicationPreferences): ApplicationPreferences => {
    const baseline = preferences.themeBaseline
        ? restoreBaseline(preferences.themeBaseline, preferences.fontFamily)
        : captureBaseline(preferences, activeThemeFonts(preferences)); // 无基线时保留当前外观。
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
    const baseline = preferences.themeBaseline
        ? restoreBaseline(preferences.themeBaseline, preferences.fontFamily)
        : captureBaseline(preferences, themeFontsOf(manifest)); // 换主题不重新快照。
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
