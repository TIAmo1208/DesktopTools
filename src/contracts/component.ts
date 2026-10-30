import type { WorkspaceRect } from "./workspace";
import { TILE_MIN_HEIGHT, TILE_MIN_WIDTH } from "./tile.ts";

/** 官方组件可以声明并由宿主授权的能力。 */
export type ComponentCapability = "read-host-time" | "read-current-media";

/** 一个官方组件的版本化清单。 */
export interface ComponentManifest {
    id: string;
    name: string;
    entry: string;
    script: string; // 插件根内的脚本相对路径。官方清单为空串。
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

/** 一次受能力检查保护的组件宿主调用结果。 */
export interface ComponentCapabilityResult {
    instanceId: string;
    capability: ComponentCapability;
    payload: { unixMillis: number };
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
    if (value !== "read-host-time" && value !== "read-current-media") throw new ComponentContractError();
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

/** 校验组件运行时快照。 */
export const parseComponentRuntimeSnapshot = (value: unknown): ComponentRuntimeSnapshot => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件运行时快照。
    if (!Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0) throw new ComponentContractError();
    if (!Array.isArray(candidate.manifests) || !Array.isArray(candidate.instances) || !Array.isArray(candidate.failures)) throw new ComponentContractError();
    const manifests = candidate.manifests.map((manifest) => {
        if (typeof manifest !== "object" || manifest === null) throw new ComponentContractError();
        const item = manifest as Record<string, unknown>; // 等待字段校验的官方组件清单。
        if (typeof item.id !== "string" || typeof item.name !== "string" || typeof item.entry !== "string" || typeof item.script !== "string")
            throw new ComponentContractError();
        if (!Array.isArray(item.requiredCapabilities) || !Number.isSafeInteger(item.persistenceVersion) || (item.persistenceVersion as number) <= 0)
            throw new ComponentContractError();
        return {
            id: item.id,
            name: item.name,
            entry: item.entry,
            script: item.script,
            requiredCapabilities: item.requiredCapabilities.map(parseCapability),
            creatable: parseDeclarationFlag(item.creatable),
            removable: parseDeclarationFlag(item.removable),
            maxTiles: parseMaxTiles(item.maxTiles),
            version: parseOptionalText(item.version),
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

/** 校验组件宿主能力调用结果。 */
export const parseComponentCapabilityResult = (value: unknown): ComponentCapabilityResult => {
    if (typeof value !== "object" || value === null) throw new ComponentContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的组件能力结果。
    if (typeof candidate.instanceId !== "string") throw new ComponentContractError();
    const capability = parseCapability(candidate.capability);
    if (typeof candidate.payload !== "object" || candidate.payload === null) throw new ComponentContractError();
    const payload = candidate.payload as Record<string, unknown>; // 宿主时间能力的结构化载荷。
    if (!Number.isSafeInteger(payload.unixMillis)) throw new ComponentContractError();
    return { instanceId: candidate.instanceId, capability, payload: { unixMillis: payload.unixMillis as number } };
};
