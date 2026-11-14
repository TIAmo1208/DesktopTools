import type { ApplicationPreferences } from "../../contracts/settings.ts";
import { interfaceFontStack } from "../../features/settings/themeFonts.ts";

/** 外观解析只需要偏好中的主题、强调色、圆角、字体与两组网格。 */
export type AppearancePreferences = Pick<
    ApplicationPreferences,
    "theme" | "desktopAppearance" | "accentColor" | "useWindowsAccentColor" | "cornerRadius" | "islandRadius" | "fontFamily" | "fontSize" | "desktopGrid" | "fenceGrid"
>;

/** 解析后的外观：界面与桌面深浅模式，以及写到文档根上的 CSS 自定义属性。 */
export interface AppearanceProjection {
    theme: string; // 界面与自绘菜单的实际深浅模式。
    desktopTheme: string; // 桌面层独立解析的深浅模式。
    variables: Record<string, string>; // 文档根 CSS 自定义属性；偏好未加载时为空。
}

/** 文档根的最小形状；测试用普通对象替换，生产传真实 `Document`。 */
export interface AppearanceRoot {
    documentElement: {
        dataset: { [key: string]: string | undefined };
        style: { setProperty: (name: string, value: string) => void };
    };
}

/** `system` 按当前系统深浅模式展开，显式模式原样返回。 */
const resolveTheme = (theme: string, systemDark: boolean) => (theme === "system" ? (systemDark ? "dark" : "light") : theme);

/** 强调色：跟随系统且已读到系统颜色时用系统值，读取失败或未开启时保留自定义颜色。 */
const resolveAccent = (useWindowsAccentColor: boolean, customColor: string, windowsAccentColor: string | undefined) =>
    useWindowsAccentColor ? (windowsAccentColor ?? customColor) : customColor;

/**
 * 解析当前外观：界面与桌面各自解析系统深浅模式，强调色各自决定是否跟随系统。
 * 只做纯计算，不读写 DOM；偏好未加载时只给出系统深浅模式。
 */
export const resolveAppearance = (preferences: AppearancePreferences | undefined, systemDark: boolean, windowsAccentColor: string | undefined): AppearanceProjection => {
    const theme = resolveTheme(preferences?.theme ?? "system", systemDark); // 加载期间沿用系统主题。
    const desktopTheme = resolveTheme(preferences?.desktopAppearance.theme ?? "system", systemDark); // 桌面独立解析系统深浅模式。
    if (!preferences) return { theme, desktopTheme, variables: {} };
    const accent = resolveAccent(preferences.useWindowsAccentColor, preferences.accentColor, windowsAccentColor); // 界面强调色。
    const desktop = preferences.desktopAppearance; // 桌面主题保持独立。
    const variables: Record<string, string> = {
        "--color-focus": accent,
        "--color-accent": accent,
        "--interface-accent": accent,
        "--desktop-accent": resolveAccent(desktop.useWindowsAccentColor, desktop.accentColor, windowsAccentColor),
        "--surface-radius": `${preferences.cornerRadius}px`,
        "--island-radius": `${preferences.islandRadius}px`,
        "--interface-font": interfaceFontStack(preferences.fontFamily),
        "--interface-font-size": `${preferences.fontSize}px`,
    };
    for (const [scope, grid] of [
        ["desktop", preferences.desktopGrid],
        ["fence", preferences.fenceGrid],
    ] as const) {
        variables[`--${scope}-grid-width`] = `${grid.width}px`;
        variables[`--${scope}-grid-height`] = `${grid.height}px`;
        variables[`--${scope}-grid-column-gap`] = `${grid.columnGap}px`;
        variables[`--${scope}-grid-row-gap`] = `${grid.rowGap}px`;
    }
    return { theme, desktopTheme, variables };
};

/** 把解析结果写到文档根：深浅模式用数据集，其余走 CSS 自定义属性。 */
export const applyAppearanceToDocument = (root: AppearanceRoot, projection: AppearanceProjection) => {
    root.documentElement.dataset.theme = projection.theme;
    root.documentElement.dataset.desktopTheme = projection.desktopTheme;
    for (const [name, value] of Object.entries(projection.variables)) root.documentElement.style.setProperty(name, value);
};
