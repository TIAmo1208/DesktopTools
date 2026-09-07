/** 一个 30 天布局归档摘要。 */
export interface VirtualDesktopArchiveSummary {
    archiveId: string;
    sourceVirtualDesktopId: string;
    archivedAtUnix: number;
    expiresAtUnix: number;
}

/** Rust 发布的 Windows 虚拟桌面环境快照。 */
export interface VirtualDesktopSnapshot {
    windowsBuild: number;
    currentVirtualDesktopId: string;
    virtualDesktopIds: string[];
    archives: VirtualDesktopArchiveSummary[];
}

/** 虚拟桌面切换开始事件载荷。 */
export interface VirtualDesktopSwitchingEvent {
    previousId: string;
    currentId: string;
}

/** 标识虚拟桌面 command/event 载荷错误。 */
export class VirtualDesktopContractError extends Error {
    readonly code = "invalid-virtual-desktop-snapshot"; // 可稳定检索的契约错误码。

    /** 创建虚拟桌面契约错误。 */
    constructor() {
        super("虚拟桌面数据无效");
        this.name = "VirtualDesktopContractError";
    }
}

/** 校验 Rust 发布的虚拟桌面环境快照。 */
export const parseVirtualDesktopSnapshot = (value: unknown): VirtualDesktopSnapshot => {
    if (typeof value !== "object" || value === null) throw new VirtualDesktopContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的虚拟桌面快照。
    if (!Number.isSafeInteger(candidate.windowsBuild) || (candidate.windowsBuild as number) <= 0) throw new VirtualDesktopContractError();
    if (typeof candidate.currentVirtualDesktopId !== "string" || candidate.currentVirtualDesktopId.length === 0) throw new VirtualDesktopContractError();
    if (!Array.isArray(candidate.virtualDesktopIds) || candidate.virtualDesktopIds.some((desktopId) => typeof desktopId !== "string" || desktopId.length === 0))
        throw new VirtualDesktopContractError();
    if (!candidate.virtualDesktopIds.includes(candidate.currentVirtualDesktopId)) throw new VirtualDesktopContractError();
    if (!Array.isArray(candidate.archives)) throw new VirtualDesktopContractError();
    const archives = candidate.archives.map((archive) => {
        if (typeof archive !== "object" || archive === null) throw new VirtualDesktopContractError();
        const archiveCandidate = archive as Record<string, unknown>; // 等待字段校验的布局归档摘要。
        if (typeof archiveCandidate.archiveId !== "string" || archiveCandidate.archiveId.length === 0) throw new VirtualDesktopContractError();
        if (typeof archiveCandidate.sourceVirtualDesktopId !== "string" || archiveCandidate.sourceVirtualDesktopId.length === 0)
            throw new VirtualDesktopContractError();
        if (!Number.isSafeInteger(archiveCandidate.archivedAtUnix) || !Number.isSafeInteger(archiveCandidate.expiresAtUnix))
            throw new VirtualDesktopContractError();
        return archiveCandidate as unknown as VirtualDesktopArchiveSummary;
    }); // 已完成契约校验的布局归档摘要集合。
    return {
        windowsBuild: candidate.windowsBuild as number,
        currentVirtualDesktopId: candidate.currentVirtualDesktopId,
        virtualDesktopIds: [...candidate.virtualDesktopIds] as string[],
        archives,
    };
};

/** 校验虚拟桌面切换开始事件载荷。 */
export const parseVirtualDesktopSwitchingEvent = (value: unknown): VirtualDesktopSwitchingEvent => {
    if (typeof value !== "object" || value === null) throw new VirtualDesktopContractError();
    const candidate = value as Record<string, unknown>; // 等待字段校验的桌面切换载荷。
    if (typeof candidate.previousId !== "string" || candidate.previousId.length === 0) throw new VirtualDesktopContractError();
    if (typeof candidate.currentId !== "string" || candidate.currentId.length === 0) throw new VirtualDesktopContractError();
    return { previousId: candidate.previousId, currentId: candidate.currentId };
};
