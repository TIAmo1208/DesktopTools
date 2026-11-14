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

/** 字体族名只做字形名边界校验：内置系统字体和插件自带字体共用这条规则。 */
export const isFontFamilyName = (value: unknown): value is string =>
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 64 &&
    value.trim() === value &&
    !/[;{}()'"\\]/.test(value) &&
    !/[\u0000-\u001f\u007f-\u009f]/.test(value);

/** 选择主题之前的用户外观快照，不包含系统强调色开关。 */
export interface ThemeBaseline {
    accentColor: string;
    desktopAccentColor: string;
    cornerRadius: number;
    islandRadius: number; // 岛展开卡圆角。
    fontFamily: string;
    fontSize: number;
}

/** 校验完整外观基线，不读取插件目录。 */
export const parseThemeBaseline = (value: unknown): ThemeBaseline => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("主题基线无效");
    const item = value as Record<string, unknown>; // 基线所有字段均必填。
    for (const key of ["accentColor", "desktopAccentColor"]) {
        if (typeof item[key] !== "string" || !/^#[0-9a-f]{6}$/i.test(item[key])) throw new Error("主题基线颜色无效");
    }
    if (typeof item.cornerRadius !== "number" || !Number.isInteger(item.cornerRadius) || item.cornerRadius < 0 || item.cornerRadius > 12)
        throw new Error("主题基线圆角无效");
    const islandRadius = item.islandRadius === undefined ? 12 : item.islandRadius; // 缺失字段使用当前默认值。
    if (typeof islandRadius !== "number" || !Number.isInteger(islandRadius) || islandRadius < 0 || islandRadius > 16) throw new Error("主题基线岛圆角无效");
    if (typeof item.fontSize !== "number" || !Number.isInteger(item.fontSize) || item.fontSize < 12 || item.fontSize > 18) throw new Error("主题基线字号无效");
    if (typeof item.fontFamily !== "string" || !isFontFamilyName(item.fontFamily)) throw new Error("主题基线字体无效");
    return {
        accentColor: item.accentColor as string,
        desktopAccentColor: item.desktopAccentColor as string,
        cornerRadius: item.cornerRadius,
        islandRadius,
        fontFamily: item.fontFamily,
        fontSize: item.fontSize,
    };
};

/** 已由 Rust 保存的应用偏好。 */
export interface ApplicationPreferences {
    configDirectory: string; // 下次启动使用的配置目录。
    pluginDirectory: string; // 空串由 Rust 解析为可执行文件同级的 plugin 目录。
    theme: ApplicationTheme; // 应用界面与自绘菜单的深浅模式。
    themePluginId: string | null; // 当前主题插件，null 为内置主题。
    themeBaseline: ThemeBaseline | null; // 未应用主题覆盖时的用户原值。
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
    weatherEnabled: boolean; // 天气格子显隐偏好，独立于凭据状态。
    musicEnabled: boolean; // 音乐格子显隐。
    collectEnabled: boolean; // 收集箱格子显隐。
    collectAutoSaveOnExit: boolean; // 编辑窗退出时自动保存。
    collectImageImportMode: "copy" | "move" | "link"; // 图片收入方式。
    collectImagePreviewWidth: number; // 列表图片宽度，单位 DIP。
    musicShowSource: boolean; // 音乐格子显示媒体来源。
    musicLyricsEnabled: boolean; // 音乐格子显示歌词。
    musicSpectrumEnabled: boolean; // 音乐格子显示实时频谱。
    musicSpectrumDevice: string; // 频谱采集设备标识，空字符串表示跟随系统默认。
    musicSpectrumBars: number; // 频谱段数。
    musicSpectrumFps: number; // 频谱刷新帧率。
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
    islandEnabled: boolean; // 岛总开关。
    islandEdge: "" | "top" | "bottom" | "left" | "right"; // 空串按顶部处理，偏移为 0 时居中。
    islandOffset: number; // 沿贴边方向的有限 DIP 偏移。
    islandHideOnFullscreen: boolean; // 前台全屏时隐藏岛。
    islandCardOrder: string[]; // 已启用岛卡及展开顺序，最多 32 条。
    islandRadius: number; // 岛展开卡圆角，独立于界面圆角。
    islandForm: "capsule" | "notch"; // 胶囊离边 6px；刘海贴边并以贴边侧直角衔接。
    disabledPlugins: string[]; // 宿主停用的插件标识，不属于插件自己的设置文件。
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
    const pluginDirectory = candidate.pluginDirectory === undefined ? "" : candidate.pluginDirectory; // 空串交给 Rust 解析默认目录。
    if (typeof pluginDirectory !== "string" || (pluginDirectory !== "" && !pluginDirectory.trim())) throw new Error("插件目录无效");
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
    if (typeof fontFamily !== "string" || !isFontFamilyName(fontFamily)) throw new Error("字体数据无效");
    const features = {
        clockEnabled: true,
        weatherEnabled: false,
        musicEnabled: false,
        collectEnabled: true,
        collectAutoSaveOnExit: false,
        musicShowSource: true,
        musicLyricsEnabled: true,
        musicSpectrumEnabled: false,
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
        islandEnabled: false,
        islandHideOnFullscreen: true,
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
    const collectImagePreviewWidth = candidate.collectImagePreviewWidth === undefined ? 78 : candidate.collectImagePreviewWidth; // 缺失配置使用当前默认值。
    if (
        typeof collectImagePreviewWidth !== "number" ||
        !Number.isInteger(collectImagePreviewWidth) ||
        collectImagePreviewWidth < 48 ||
        collectImagePreviewWidth > 156
    )
        throw new Error("图片预览宽度必须为 48 到 156 DIP 的整数");
    const musicSpectrumDevice = candidate.musicSpectrumDevice === undefined ? "" : candidate.musicSpectrumDevice; // 缺失配置跟随系统默认设备。
    if (typeof musicSpectrumDevice !== "string") throw new Error("频谱设备数据无效");
    const musicSpectrumBars = candidate.musicSpectrumBars === undefined ? 32 : candidate.musicSpectrumBars; // 缺失配置使用默认段数。
    if (typeof musicSpectrumBars !== "number" || !Number.isInteger(musicSpectrumBars) || musicSpectrumBars < 16 || musicSpectrumBars > 64)
        throw new Error("频谱段数必须为 16 到 64 的整数");
    const musicSpectrumFps = candidate.musicSpectrumFps === undefined ? 30 : candidate.musicSpectrumFps; // 缺失配置使用默认帧率。
    if (typeof musicSpectrumFps !== "number" || !Number.isInteger(musicSpectrumFps) || musicSpectrumFps < 15 || musicSpectrumFps > 60)
        throw new Error("频谱帧率必须为 15 到 60 的整数");
    const islandEdge = candidate.islandEdge === undefined ? "" : candidate.islandEdge; // 未设置时按顶部，偏移默认 0。
    if (islandEdge !== "" && islandEdge !== "top" && islandEdge !== "bottom" && islandEdge !== "left" && islandEdge !== "right")
        throw new Error("岛贴边方向无效");
    const islandOffset = candidate.islandOffset === undefined ? 0 : candidate.islandOffset; // 未设置时无偏移。
    if (typeof islandOffset !== "number" || !Number.isFinite(islandOffset)) throw new Error("岛偏移必须为有限的 DIP 数值");
    const islandRadius = candidate.islandRadius === undefined ? 12 : candidate.islandRadius; // 展开卡默认圆角。
    if (typeof islandRadius !== "number" || !Number.isInteger(islandRadius) || islandRadius < 0 || islandRadius > 16) throw new Error("岛圆角数据无效");
    const islandForm = candidate.islandForm === undefined ? "capsule" : candidate.islandForm; // 缺失配置使用胶囊形态。
    if (islandForm !== "capsule" && islandForm !== "notch") throw new Error("岛形态数据无效");
    const cards = candidate.islandCardOrder === undefined ? [] : candidate.islandCardOrder; // 与 Rust Vec<String> 边界一致。
    if (!Array.isArray(cards) || cards.some((card) => typeof card !== "string")) throw new Error("岛卡顺序数据无效");
    const islandCardOrder = [
        ...new Set(cards.filter((card: string) => ["official.", "plugin."].some((prefix) => card.startsWith(prefix) && card.length > prefix.length))),
    ].slice(0, 32); // 按首次出现顺序规范化。
    const disabledPlugins = parseDisabledPlugins(candidate.disabledPlugins); // 缺失、非数组和非法条目都按空或丢弃处理。
    const themePluginId = candidate.themePluginId ?? null; // 缺省不选择外部主题，不检查目录存在性。
    if (themePluginId !== null && (typeof themePluginId !== "string" || !themePluginId.trim())) throw new Error("主题插件标识无效");
    const themeBaseline = candidate.themeBaseline == null ? null : parseThemeBaseline(candidate.themeBaseline); // 仅校验保存值。
    return {
        theme: candidate.theme,
        themePluginId,
        themeBaseline,
        desktopAppearance: {
            theme: desktopAppearance.theme,
            accentColor: desktopAppearance.accentColor,
            useWindowsAccentColor: desktopAppearance.useWindowsAccentColor,
        },
        configDirectory,
        pluginDirectory,
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
        collectImagePreviewWidth,
        musicSpectrumDevice,
        musicSpectrumBars,
        musicSpectrumFps,
        islandEdge,
        islandOffset,
        islandRadius,
        islandForm,
        islandCardOrder,
        disabledPlugins,
    };
};

/** 停用名单只保留带 plugin. 前缀且前缀后仍有名称的标识，去重后最多 64 条。 */
const parseDisabledPlugins = (value: unknown): string[] => {
    if (!Array.isArray(value)) return [];
    const kept: string[] = []; // 按首次出现顺序保留的合法标识。
    const seen = new Set<string>(); // 已保留的标识。
    for (const item of value) {
        if (typeof item !== "string" || !item.startsWith("plugin.") || item.length <= "plugin.".length || seen.has(item) || kept.length >= 64) continue;
        seen.add(item);
        kept.push(item);
    }
    return kept;
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
