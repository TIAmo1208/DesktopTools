import type { WorkspaceRect } from "./workspace";
import { isFontFamilyName } from "./settings.ts";
import { TILE_MIN_HEIGHT, TILE_MIN_WIDTH } from "./tile.ts";

/** 官方组件可以声明并由宿主授权的能力。 */
export type ComponentCapability = "read-host-time" | "read-current-media" | "probe-service-status";

/** 插件呈现角色，缺省为格子。 */
export type ComponentRole = "tile" | "theme";

/** 主题自带的字体，由宿主注册进当前文档的字体集。 */
export interface ThemeFont {
    family: string; // 用户可见的字体族名，也是偏好里保存的值。
    file: string; // 插件根内的字体文件相对路径。
    weight: number; // 字重，100–900。
    style: "normal" | "italic"; // 字形。
}

/** 通过偏好校验的主题外观覆盖，null 表示不覆盖。 */
export interface ThemeAppearance {
    accentColor: string | null;
    desktopAccentColor: string | null;
    cornerRadius: number | null;
    fontFamily: string | null;
    fontSize: number | null;
}

/** 一个官方组件的版本化清单。 */
export interface ComponentManifest {
    id: string;
    name: string;
    entry: string;
    script: string; // 插件根内的脚本相对路径。官方清单为空串；主题没有入口文件时同样为空串。
    role?: ComponentRole; // 缺省为 tile。
    stylesheet?: string; // 插件根内的主题样式表路径。
    stylesheetCacheKey?: string; // 仅用于主题样式表 URL 缓存失效。
    themeAppearance?: ThemeAppearance | null; // 非主题为 null。
    themeFonts?: ThemeFont[]; // 主题自带的字体，非主题缺省为空。
    requiredCapabilities: ComponentCapability[];
    // 用户是否可从工作区入口创建该组件的新实例。
    creatable: boolean;
    // 用户是否可移除该组件的实例。
    removable: boolean;
    maxTiles: number; // 可创建的格子上限。0 表示不限。
    version?: string; // 插件版本。缺省不展示。
    description?: string; // 插件简介。缺省不展示。
    author?: string; // 插件作者。缺省时信息行不拼 @。
    repository?: string; // 仓库地址。只表示有没有，页面不拿它打开。
    persistenceVersion: number;
}

/** 工作区持久化的一个组件实例。 */
export interface ComponentInstanceSnapshot {
    instanceId: string;
    componentId: string;
    monitorId: string;
    projectionMonitorId: string | null;
    projectionBounds: WorkspaceRect | null;
    bounds: WorkspaceRect;
    stateVersion: number;
    state: unknown;
}

/** 一个组件实例当前的隔离加载状态。 */
export type ComponentLoadStatus = "ready" | "failed";

/** Component Runtime 发布给前端的一个实例注册结果。 */
export interface ComponentRegistrationSnapshot {
    instanceId: string;
    componentId: string;
    entry: string | null;
    authorizedCapabilities: ComponentCapability[];
    status: ComponentLoadStatus;
    error: string | null;
}

/** 一个未能进入官方组件注册表的清单失败结果。 */
export interface ComponentRegistrationFailure {
    componentId: string;
    message: string;
}

/** Component Runtime 的不可变注册表和实例快照。 */
export interface ComponentRuntimeSnapshot {
    revision: number;
    manifests: ComponentManifest[];
    instances: ComponentRegistrationSnapshot[];
    failures: ComponentRegistrationFailure[];
}

/** 宿主定时发布给已授权组件的事件。 */
export interface ComponentHostEvent {
    sequence: number;
    kind: "time-tick";
    unixMillis: number;
}

/** 服务探活的四档状态，未知不得当成离线。 */
export type ProbeLevel = "ok" | "warn" | "off" | "unknown";

/** 服务探活的结构化结果，时长由前端用 startTime 现算。 */
export interface ProbeServiceStatus {
    level: ProbeLevel;
    startTime: number | null;
    checkedAt: number;
}

/** 一次受能力检查保护的组件宿主调用结果。 */
export interface ComponentCapabilityResult {
    instanceId: string;
    capability: ComponentCapability;
    payload: { unixMillis: number } | ProbeServiceStatus;
}

/** 标识组件清单、实例或宿主事件契约错误。 */
export class ComponentContractError extends Error {
    readonly code = "invalid-component-contract"; // 可稳定检索的契约错误码。

    /** 创建组件公开契约错误。 */
    constructor() {
        super("组件运行时数据无效");
        this.name = "ComponentContractError";
    }
}

/** 判断未知值是否为有限数字。 */
const isFiniteNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

/** 持久边界遵守公共底线，临时投射可以适应更小的视口。 */
const parseBounds = (value: unknown, projection = false): WorkspaceRect => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件边界。
    if (!isFiniteNumber(candidate.left) || !isFiniteNumber(candidate.top)) throw new ComponentContractError();
    if (!isFiniteNumber(candidate.width) || candidate.width <= 0 || !isFiniteNumber(candidate.height) || candidate.height <= 0)
        throw new ComponentContractError();
    if (!projection && (candidate.width < TILE_MIN_WIDTH || candidate.height < TILE_MIN_HEIGHT)) throw new ComponentContractError();
    return { left: candidate.left, top: candidate.top, width: candidate.width, height: candidate.height };
};

/** 校验一个工作区组件实例。 */
export const parseComponentInstanceSnapshot = (value: unknown): ComponentInstanceSnapshot => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件实例。
    if (typeof candidate.instanceId !== "string" || candidate.instanceId.length === 0) throw new ComponentContractError();
    if (typeof candidate.componentId !== "string" || candidate.componentId.length === 0) throw new ComponentContractError();
    if (typeof candidate.monitorId !== "string") throw new ComponentContractError();
    if (candidate.projectionMonitorId !== null && typeof candidate.projectionMonitorId !== "string") throw new ComponentContractError();
    if (!Number.isSafeInteger(candidate.stateVersion) || (candidate.stateVersion as number) <= 0) throw new ComponentContractError();
    return {
        instanceId: candidate.instanceId,
        componentId: candidate.componentId,
        monitorId: candidate.monitorId,
        projectionMonitorId: candidate.projectionMonitorId as string | null,
        projectionBounds: candidate.projectionBounds === null ? null : parseBounds(candidate.projectionBounds, true),
        bounds: parseBounds(candidate.bounds),
        stateVersion: candidate.stateVersion as number,
        state: candidate.state,
    };
};

/** 校验组件能力名称。 */
const parseCapability = (value: unknown): ComponentCapability => {
    if (value !== "read-host-time" && value !== "read-current-media" && value !== "probe-service-status") throw new ComponentContractError();
    return value;
};

/** 可缺省的短文本。空串当没写，别的类型拒绝。 */
const parseOptionalText = (value: unknown): string | undefined => {
    if (value === undefined || value === "") return undefined;
    if (typeof value !== "string") throw new ComponentContractError();
    return value;
};

/** 格子上限。缺省和 0 都是不限，负数与非整数拒绝。 */
const parseMaxTiles = (value: unknown): number => {
    if (value === undefined || value === 0) return 0;
    if (!Number.isSafeInteger(value) || (value as number) < 0) throw new ComponentContractError();
    return value as number;
};

/** 校验可缺省的创建或移除声明，缺失按不可用处理。 */
const parseDeclarationFlag = (value: unknown): boolean => {
    if (value === undefined) return false;
    if (typeof value !== "boolean") throw new ComponentContractError();
    return value;
};

/** 校验主题外观覆盖，沿用偏好值域，不把缺省项变成覆盖值。 */
const parseThemeAppearance = (value: unknown): ThemeAppearance | null => {
    if (value === undefined || value === null) return null;
    if (typeof value !== "object" || Array.isArray(value)) throw new ComponentContractError();
    const item = value as Record<string, unknown>; // 待校验的覆盖字段。
    for (const key of ["accentColor", "desktopAccentColor"]) {
        if (item[key] != null && (typeof item[key] !== "string" || !/^#[0-9a-f]{6}$/i.test(item[key]))) throw new ComponentContractError();
    }
    for (const [key, min, max] of [
        ["cornerRadius", 0, 12],
        ["fontSize", 12, 18],
    ] as const) {
        const field = item[key]; // 待校验的整数范围。
        if (field != null && (typeof field !== "number" || !Number.isInteger(field) || field < min || field > max)) throw new ComponentContractError();
    }
    if (item.fontFamily != null && !isFontFamilyName(item.fontFamily)) throw new ComponentContractError();
    return {
        accentColor: (item.accentColor ?? null) as string | null,
        desktopAccentColor: (item.desktopAccentColor ?? null) as string | null,
        cornerRadius: (item.cornerRadius ?? null) as number | null,
        fontFamily: (item.fontFamily ?? null) as string | null,
        fontSize: (item.fontSize ?? null) as number | null,
    };
};

/** 校验主题自带字体：族名走公共规则，文件只认插件根内的字体扩展名。 */
const parseThemeFonts = (value: unknown): ThemeFont[] => {
    if (value === undefined || value === null) return [];
    if (!Array.isArray(value)) throw new ComponentContractError();
    if (value.length > 4) throw new ComponentContractError();
    return value.map((font): ThemeFont => {
        if (typeof font !== "object" || font === null) throw new ComponentContractError();
        const item = font as Record<string, unknown>; // 待校验的字体声明。
        if (!isFontFamilyName(item.family)) throw new ComponentContractError();
        if (typeof item.file !== "string" || item.file.startsWith("/") || item.file.includes("..") || !/\.(woff2|woff|ttf|otf)$/.test(item.file))
            throw new ComponentContractError();
        if (!Number.isInteger(item.weight) || (item.weight as number) < 100 || (item.weight as number) > 900) throw new ComponentContractError();
        if (item.style !== "normal" && item.style !== "italic") throw new ComponentContractError();
        return { family: item.family, file: item.file, weight: item.weight as number, style: item.style };
    });
};

/** 校验组件运行时快照。 */
export const parseComponentRuntimeSnapshot = (value: unknown): ComponentRuntimeSnapshot => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件运行时快照。
    if (!Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0) throw new ComponentContractError();
    if (!Array.isArray(candidate.manifests) || !Array.isArray(candidate.instances) || !Array.isArray(candidate.failures)) throw new ComponentContractError();
    const manifests = candidate.manifests.map((manifest): ComponentManifest => {
        if (typeof manifest !== "object" || manifest === null) throw new ComponentContractError();
        const item = manifest as Record<string, unknown>; // 等待字段校验的官方组件清单。
        if (typeof item.id !== "string" || typeof item.name !== "string" || typeof item.entry !== "string" || typeof item.script !== "string")
            throw new ComponentContractError();
        if (!Array.isArray(item.requiredCapabilities) || !Number.isSafeInteger(item.persistenceVersion) || (item.persistenceVersion as number) <= 0)
            throw new ComponentContractError();
        const role = item.role ?? "tile"; // 缺省保持格子角色。
        if (role !== "tile" && role !== "theme") throw new ComponentContractError();
        const stylesheet = item.stylesheet ?? ""; // 非主题没有样式表。
        if (typeof stylesheet !== "string" || (role === "theme" && !stylesheet)) throw new ComponentContractError();
        return {
            role,
            stylesheet,
            themeAppearance: parseThemeAppearance(item.themeAppearance),
            themeFonts: parseThemeFonts(item.themeFonts),
            id: item.id,
            name: item.name,
            entry: item.entry,
            script: item.script,
            requiredCapabilities: item.requiredCapabilities.map(parseCapability),
            creatable: parseDeclarationFlag(item.creatable),
            removable: parseDeclarationFlag(item.removable),
            maxTiles: parseMaxTiles(item.maxTiles),
            version: parseOptionalText(item.version),
            stylesheetCacheKey: parseOptionalText(item.stylesheetCacheKey),
            description: parseOptionalText(item.description),
            author: parseOptionalText(item.author),
            repository: parseOptionalText(item.repository),
            persistenceVersion: item.persistenceVersion as number,
        };
    });
    const instances = candidate.instances.map((instance) => {
        if (typeof instance !== "object" || instance === null) throw new ComponentContractError();
        const item = instance as Record<string, unknown>; // 等待字段校验的组件注册结果。
        if (typeof item.instanceId !== "string" || typeof item.componentId !== "string") throw new ComponentContractError();
        if (item.entry !== null && typeof item.entry !== "string") throw new ComponentContractError();
        if (!Array.isArray(item.authorizedCapabilities) || (item.status !== "ready" && item.status !== "failed")) throw new ComponentContractError();
        if (item.error !== null && typeof item.error !== "string") throw new ComponentContractError();
        const status: ComponentLoadStatus = item.status; // 已完成公开协议校验的组件加载状态。
        return {
            instanceId: item.instanceId,
            componentId: item.componentId,
            entry: item.entry as string | null,
            authorizedCapabilities: item.authorizedCapabilities.map(parseCapability),
            status,
            error: item.error as string | null,
        };
    });
    const failures = candidate.failures.map((failure) => {
        if (typeof failure !== "object" || failure === null) throw new ComponentContractError();
        const item = failure as Record<string, unknown>; // 等待字段校验的清单失败结果。
        if (typeof item.componentId !== "string" || typeof item.message !== "string") throw new ComponentContractError();
        return { componentId: item.componentId, message: item.message };
    });
    return { revision: candidate.revision as number, manifests, instances, failures };
};

/** 校验组件宿主事件。 */
export const parseComponentHostEvent = (value: unknown): ComponentHostEvent => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件宿主事件。
    if (!Number.isSafeInteger(candidate.sequence) || (candidate.sequence as number) <= 0) throw new ComponentContractError();
    if (candidate.kind !== "time-tick" || !Number.isSafeInteger(candidate.unixMillis)) throw new ComponentContractError();
    return { sequence: candidate.sequence as number, kind: "time-tick", unixMillis: candidate.unixMillis as number };
};

/** 按能力校验载荷：探活只接受固定四档，出现其它值时拒绝。 */
export const parseComponentCapabilityResult = (value: unknown): ComponentCapabilityResult => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件能力结果。
    if (typeof candidate.instanceId !== "string") throw new ComponentContractError();
    const capability = parseCapability(candidate.capability);
    if (typeof candidate.payload !== "object" || candidate.payload === null) throw new ComponentContractError();
    const payload = candidate.payload as Record<string, unknown>; // 当前能力的结构化载荷，字段由能力决定。
    if (capability === "probe-service-status") return { instanceId: candidate.instanceId, capability, payload: parseProbeServiceStatus(payload) };
    if (!Number.isSafeInteger(payload.unixMillis)) throw new ComponentContractError();
    return { instanceId: candidate.instanceId, capability, payload: { unixMillis: payload.unixMillis as number } };
};

/** 校验探活载荷：只有 ok 带启动时间，其它档一律为 null。 */
const parseProbeServiceStatus = (payload: Record<string, unknown>): ProbeServiceStatus => {
    const level = payload.level;
    if (level !== "ok" && level !== "warn" && level !== "off" && level !== "unknown") throw new ComponentContractError();
    if (!Number.isSafeInteger(payload.checkedAt) || (payload.checkedAt as number) <= 0) throw new ComponentContractError();
    if (level === "ok") {
        if (!Number.isSafeInteger(payload.startTime) || (payload.startTime as number) <= 0) throw new ComponentContractError();
        return { level, startTime: payload.startTime as number, checkedAt: payload.checkedAt as number };
    }
    if (payload.startTime !== null) throw new ComponentContractError();
    return { level, startTime: null, checkedAt: payload.checkedAt as number };
};
