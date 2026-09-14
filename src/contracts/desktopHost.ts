/** 正式透明桌面宿主的接管阶段。 */
export type TakeoverPhase = "starting" | "ready" | "committing" | "committed" | "failed" | "restored";

/** React 发布的一个语义交互命中区域。 */
export interface HitRegion {
    id: string;
    kind: string;
    left: number;
    top: number;
    width: number;
    height: number;
    priority: number;
    targetPath?: string;
}

/** React 双帧完成后发布的宿主准备证据。 */
export interface FrontendHostReadiness {
    monitorId: string;
    viewportWidth: number;
    viewportHeight: number;
    devicePixelRatio: number;
    totalSystemImages: number;
    loadedSystemImages: number;
    hitRegions: HitRegion[];
}

/** 一台正式宿主使用的显示器身份、物理边界、工作区与 DPI。 */
export interface DesktopHostMonitor {
    id: string;
    name: string;
    primary: boolean;
    x: number;
    y: number;
    width: number;
    height: number;
    workX: number;
    workY: number;
    workWidth: number;
    workHeight: number;
    scaleFactor: number;
    dpi: number;
}

/** 一台显示器宿主的独立上下文快照。 */
export interface DesktopHostContextSnapshot {
    monitor: DesktopHostMonitor;
    readiness: DesktopHostReadiness;
}

/** 正式桌面接管准备状态与输入诊断状态。 */
export interface DesktopHostReadiness {
    guardian: boolean;
    page: boolean;
    systemImages: boolean;
    pixels: boolean;
    input: boolean;
    hitMap: boolean;
}

/** 前后端共享的正式宿主快照。 */
export interface DesktopHostSnapshot {
    phase: TakeoverPhase;
    committed: boolean;
    hosts: DesktopHostContextSnapshot[];
}

/** 标识正式宿主契约边界的载荷错误。 */
export class DesktopHostContractError extends Error {
    readonly code = "invalid-desktop-host-snapshot"; // 可稳定检索的契约错误码。

    /** 创建正式宿主契约错误。 */
    constructor() {
        super("桌面宿主数据无效");
        this.name = "DesktopHostContractError";
    }
}

/** 校验 command 返回值与 event 载荷并生成正式宿主快照。 */
export const parseDesktopHostSnapshot = (value: unknown): DesktopHostSnapshot => {
    if (typeof value !== "object" || value === null) throw new DesktopHostContractError();

    // 等待字段校验的正式宿主快照载荷。
    const candidate = value as Record<string, unknown>;
    if (!["starting", "ready", "committing", "committed", "failed", "restored"].includes(candidate.phase as string)) throw new DesktopHostContractError();
    if (typeof candidate.committed !== "boolean") throw new DesktopHostContractError();
    if (!Array.isArray(candidate.hosts) || candidate.hosts.length === 0) throw new DesktopHostContractError();
    const hosts = candidate.hosts.map((host) => {
        if (typeof host !== "object" || host === null) throw new DesktopHostContractError();
        const hostCandidate = host as Record<string, unknown>; // 等待字段校验的逐屏宿主上下文。
        if (typeof hostCandidate.monitor !== "object" || hostCandidate.monitor === null) throw new DesktopHostContractError();
        if (typeof hostCandidate.readiness !== "object" || hostCandidate.readiness === null) throw new DesktopHostContractError();
        const monitor = hostCandidate.monitor as Record<string, unknown>; // 等待字段校验的显示器物理状态。
        if (typeof monitor.id !== "string" || monitor.id.length === 0 || typeof monitor.name !== "string" || monitor.name.length === 0)
            throw new DesktopHostContractError();
        if (typeof monitor.primary !== "boolean") throw new DesktopHostContractError();
        for (const field of ["x", "y", "width", "height", "workX", "workY", "workWidth", "workHeight", "scaleFactor", "dpi"])
            if (typeof monitor[field] !== "number" || !Number.isFinite(monitor[field])) throw new DesktopHostContractError();
        const readiness = hostCandidate.readiness as Record<string, unknown>; // 等待字段校验的逐屏六项门槛。
        for (const field of ["guardian", "page", "systemImages", "pixels", "input", "hitMap"])
            if (typeof readiness[field] !== "boolean") throw new DesktopHostContractError();
        return hostCandidate as unknown as DesktopHostContextSnapshot;
    }); // 已完成契约校验的逐屏宿主集合。

    return { phase: candidate.phase as TakeoverPhase, committed: candidate.committed, hosts };
};
