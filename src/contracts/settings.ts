/** 两组外观各自选择的深浅模式。 */
export type ApplicationTheme = "system" | "light" | "dark";

/** 桌面网格独立保存的主题与强调色。 */
export interface DesktopAppearance {
    theme: ApplicationTheme; // 桌面深浅模式。
    accentColor: string; // 桌面自定义强调色。
    useWindowsAccentColor: boolean; // 桌面是否使用 Windows 强调色。
}

/** 独立网格的格子尺寸与空白，单位为 DIP。 */
export interface GridPreferences {
    width: number; // 格子宽度。
    height: number; // 格子高度。
    columnGap: number; // 横向间距。
    rowGap: number; // 纵向间距。
}

export const DEFAULT_DESKTOP_GRID: GridPreferences = { width: 68, height: 90, columnGap: 10, rowGap: 8 }; // 桌面默认步进 78 × 98。
export const DEFAULT_FENCE_GRID: GridPreferences = { width: 68, height: 90, columnGap: 6, rowGap: 8 }; // 围栏平铺默认值。

/** 验证网格配置，不接受非有限值、小数或越界尺寸。 */
export const parseGridPreferences = (value: unknown): GridPreferences => {
    if (typeof value !== "object" || value === null) throw new Error("网格配置无效");
    const grid = value as Record<string, unknown>; // 待校验的边界载荷。
    for (const [key, min, max] of [
        ["width", 64, 160],
        ["height", 90, 180],
        ["columnGap", 0, 48],
        ["rowGap", 0, 48],
    ] as const) {
        const field = grid[key]; // 当前网格字段。
        if (typeof field !== "number" || !Number.isInteger(field) || field < min || field > max) throw new Error("网格尺寸或间距超出范围");
    }
    return { width: grid.width as number, height: grid.height as number, columnGap: grid.columnGap as number, rowGap: grid.rowGap as number };
};

// Windows 界面字体选项与后端白名单保持一致。
export const APPLICATION_FONTS = ["Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", "SimSun", "Arial"] as const;

/** 已由 Rust 保存的应用偏好。 */
export interface ApplicationPreferences {
    configDirectory: string; // 下次启动使用的配置目录。
    theme: ApplicationTheme; // 应用界面与自绘菜单的深浅模式。
    desktopAppearance: DesktopAppearance; // 独立于界面与菜单的桌面主题。
    doubleClickToOpen: boolean; // 是否通过双击打开桌面项目。
    accentColor: string; // 应用界面与自绘菜单的强调色。
    useWindowsAccentColor: boolean; // 跟随 Windows 强调色，保留自定义颜色。
    cornerRadius: number; // 普通表面圆角。
    fontFamily: string; // 当前选择的界面字体。
    fontSize: number; // 普通界面字号。
    desktopGrid: GridPreferences; // 未归类桌面网格。
    fenceGrid: GridPreferences; // 围栏内网格。
    clockEnabled: boolean; // 时间格子显隐。
    musicEnabled: boolean; // 音乐格子显隐。
    collectEnabled: boolean; // 收集箱格子显隐。
    collectAutoSaveOnExit: boolean; // 编辑窗退出时自动保存。
    collectImageImportMode: "copy" | "move" | "link"; // 图片收入方式。
    musicShowSource: boolean; // 音乐格子显示媒体来源。
    clockHour12: boolean; // 十二小时制。
    clockShowSeconds: boolean; // 显示秒数。
    debugEnabled: boolean; // Debug 格子显隐。
    debugShowDetails: boolean; // 显示详细状态。
    componentSnap: boolean; // 功能格子边缘对齐。
    followSystemReducedMotion: boolean; // 是否由 Windows 减少动态效果决定实际动画状态。
    interfaceAnimations: boolean; // 不跟随系统时使用的界面装饰动画偏好。
    folderPortalEnabled: boolean; // 文件夹门户功能入口显隐。
    pauseOnFullscreen: boolean; // 前台全屏或最大化覆盖本屏时暂停刷新。
    pauseOnFocus: boolean; // 外部应用获得焦点时暂停全部桌面刷新。
}

/** 设置页读取的真实应用和 Windows 状态。 */
export interface SettingsSnapshot {
    preferences: ApplicationPreferences; // 已保存的用户偏好。
    autostart: boolean; // Windows 中当前程序的启动状态。
    version: string; // 后端构建产生的版本号。
    debugBuild: boolean; // 后端构建决定测试功能可用性。
}

/** 校验后端偏好载荷，拒绝错误类型和未知主题。 */
export const parsePreferences = (value: unknown): ApplicationPreferences => {
    if (typeof value !== "object" || value === null) throw new Error("应用偏好数据无效");
    const candidate = value as Record<string, unknown>; // 等待边界校验的原始偏好。
    const configDirectory = candidate.configDirectory === undefined ? "%APPDATA%\\TIAmo\\DesktopTools" : candidate.configDirectory; // 旧配置使用新的默认目录。
    if (typeof configDirectory !== "string" || !configDirectory.trim()) throw new Error("配置目录无效");
    if (candidate.theme !== "system" && candidate.theme !== "light" && candidate.theme !== "dark") throw new Error("应用主题数据无效");
    if (typeof candidate.doubleClickToOpen !== "boolean") throw new Error("项目打开方式数据无效");
    const accentColor = candidate.accentColor === undefined ? "#2563eb" : candidate.accentColor; // 旧配置缺省时的主题色。
    const useWindowsAccentColor = candidate.useWindowsAccentColor === undefined ? false : candidate.useWindowsAccentColor; // 旧配置继续使用自定义颜色。
    if (typeof useWindowsAccentColor !== "boolean") throw new Error("Windows 主题色开关无效");
    const cornerRadius = candidate.cornerRadius === undefined ? 6 : candidate.cornerRadius; // 旧配置缺省时的圆角。
    const fontFamily = candidate.fontFamily === undefined ? "Segoe UI Variable" : candidate.fontFamily; // 旧配置缺省时的字体。
    const fontSize = candidate.fontSize === undefined ? 13 : candidate.fontSize; // 旧配置缺省时的字号。
    if (typeof accentColor !== "string" || !/^#[0-9a-f]{6}$/i.test(accentColor)) throw new Error("主题色数据无效");
    const desktop =
        candidate.desktopAppearance === undefined ? { theme: "system", accentColor: "#2563eb", useWindowsAccentColor: false } : candidate.desktopAppearance; // 桌面主题缺失时使用固定默认值。
    if (typeof desktop !== "object" || desktop === null) throw new Error("桌面网格主题数据无效");
    const desktopAppearance = desktop as Record<string, unknown>; // 桌面主题的待校验字段。
    if (desktopAppearance.theme !== "system" && desktopAppearance.theme !== "light" && desktopAppearance.theme !== "dark")
        throw new Error("桌面网格深浅模式无效");
    if (typeof desktopAppearance.accentColor !== "string" || !/^#[0-9a-f]{6}$/i.test(desktopAppearance.accentColor)) throw new Error("桌面网格主题色无效");
    if (typeof desktopAppearance.useWindowsAccentColor !== "boolean") throw new Error("桌面网格 Windows 主题色开关无效");
    if (typeof cornerRadius !== "number" || !Number.isInteger(cornerRadius) || cornerRadius < 0 || cornerRadius > 12) throw new Error("圆角数据无效");
    if (typeof fontSize !== "number" || !Number.isInteger(fontSize) || fontSize < 12 || fontSize > 18) throw new Error("字号数据无效");
    if (typeof fontFamily !== "string" || !APPLICATION_FONTS.some((font) => font === fontFamily)) throw new Error("字体数据无效");
    const features = {
        clockEnabled: true,
        musicEnabled: false,
        collectEnabled: true,
        collectAutoSaveOnExit: false,
        musicShowSource: true,
        clockHour12: false,
        clockShowSeconds: true,
        debugEnabled: true,
        debugShowDetails: true,
        componentSnap: true,
        followSystemReducedMotion: true,
        interfaceAnimations: true,
        folderPortalEnabled: true,
        pauseOnFullscreen: false,
        pauseOnFocus: false,
    }; // 旧配置默认行为。
    for (const key of Object.keys(features) as (keyof typeof features)[]) {
        if (candidate[key] === undefined) continue;
        if (typeof candidate[key] !== "boolean") throw new Error("功能格子配置无效");
        features[key] = candidate[key];
    }
    if (
        candidate.collectImageImportMode !== undefined &&
        candidate.collectImageImportMode !== "copy" &&
        candidate.collectImageImportMode !== "move" &&
        candidate.collectImageImportMode !== "link"
    )
        throw new Error("图片收入方式无效");
    const collectImageImportMode = (candidate.collectImageImportMode as "copy" | "move" | "link" | undefined) ?? "copy";
    return {
        theme: candidate.theme,
        desktopAppearance: {
            theme: desktopAppearance.theme,
            accentColor: desktopAppearance.accentColor,
            useWindowsAccentColor: desktopAppearance.useWindowsAccentColor,
        },
        configDirectory,
        doubleClickToOpen: candidate.doubleClickToOpen,
        accentColor,
        useWindowsAccentColor,
        cornerRadius,
        fontFamily,
        fontSize,
        desktopGrid: parseGridPreferences(candidate.desktopGrid === undefined ? DEFAULT_DESKTOP_GRID : candidate.desktopGrid),
        fenceGrid: parseGridPreferences(candidate.fenceGrid === undefined ? DEFAULT_FENCE_GRID : candidate.fenceGrid),
        ...features,
        collectImageImportMode,
    };
};

/** 解析当前窗口实际使用的动画状态。 */
export const resolveAnimationsEnabled = (preferences: ApplicationPreferences | undefined, reducedMotion: boolean) =>
    (preferences?.followSystemReducedMotion ?? true) ? !reducedMotion : (preferences?.interfaceAnimations ?? true);

/** 校验设置窗口的完整初始状态。 */
export const parseSettingsSnapshot = (value: unknown): SettingsSnapshot => {
    if (typeof value !== "object" || value === null) throw new Error("设置数据无效");
    const candidate = value as Record<string, unknown>; // 等待边界校验的完整设置。
    if (typeof candidate.autostart !== "boolean" || typeof candidate.version !== "string" || !candidate.version) throw new Error("设置状态数据无效");
    if (typeof candidate.debugBuild !== "boolean") throw new Error("构建模式数据无效");
    return {
        preferences: parsePreferences(candidate.preferences),
        autostart: candidate.autostart,
        version: candidate.version,
        debugBuild: candidate.debugBuild,
    };
};
