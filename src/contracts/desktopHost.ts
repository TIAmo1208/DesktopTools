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
}

/** React 双帧完成后发布的宿主准备证据。 */
export interface FrontendHostReadiness {
    viewportWidth: number;
    viewportHeight: number;
    devicePixelRatio: number;
    totalSystemImages: number;
    loadedSystemImages: number;
    hitRegions: HitRegion[];
}

/** 单显示器正式宿主使用的物理工作区。 */
export interface DesktopHostMonitor {
    workX: number;
    workY: number;
    workWidth: number;
    workHeight: number;
    scaleFactor: number;
    dpi: number;
}

/** 正式桌面接管的六项门槛状态。 */
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
    monitor: DesktopHostMonitor;
    readiness: DesktopHostReadiness;
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
    if (typeof candidate.monitor !== "object" || candidate.monitor === null) throw new DesktopHostContractError();
    if (typeof candidate.readiness !== "object" || candidate.readiness === null) throw new DesktopHostContractError();

    // 等待字段校验的物理工作区载荷。
    const monitor = candidate.monitor as Record<string, unknown>;
    for (const field of ["workX", "workY", "workWidth", "workHeight", "scaleFactor", "dpi"])
        if (typeof monitor[field] !== "number" || !Number.isFinite(monitor[field])) throw new DesktopHostContractError();

    // 等待字段校验的六项门槛载荷。
    const readiness = candidate.readiness as Record<string, unknown>;
    for (const field of ["guardian", "page", "systemImages", "pixels", "input", "hitMap"])
        if (typeof readiness[field] !== "boolean") throw new DesktopHostContractError();

    return candidate as unknown as DesktopHostSnapshot;
};
