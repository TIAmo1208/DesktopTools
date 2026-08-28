/** 应用运行状态。 */
export type ApplicationStatus = "ready" | "active";

/** 前后端共享的应用状态快照。 */
export interface ApplicationSnapshot {
    status: ApplicationStatus;
    revision: number;
}

/** 标识应用契约边界的载荷错误。 */
export class ApplicationContractError extends Error {
    readonly code: string; // 可稳定检索的契约错误码。

    /** 创建应用契约错误。 */
    constructor(code = "invalid-application-snapshot", message = "应用状态数据无效") {
        super(message);
        this.name = "ApplicationContractError";
        this.code = code;
    }
}

/** 表示 Rust command 返回的已验证错误。 */
export class ApplicationCommandError extends Error {
    readonly code: string; // Rust command 返回的稳定错误码。

    /** 创建已验证的应用 command 错误。 */
    constructor(code: string, message: string) {
        super(message);
        this.name = "ApplicationCommandError";
        this.code = code;
    }
}

/** 校验 command 返回值与 event 载荷并生成应用快照。 */
export const parseApplicationSnapshot = (value: unknown): ApplicationSnapshot => {
    if (typeof value !== "object" || value === null) throw new ApplicationContractError();

    // 等待字段校验的进程边界载荷。
    const candidate = value as Record<string, unknown>;
    if (candidate.status !== "ready" && candidate.status !== "active") throw new ApplicationContractError();
    if (!Number.isSafeInteger(candidate.revision) || (candidate.revision as number) < 0) throw new ApplicationContractError();

    return { status: candidate.status, revision: candidate.revision as number };
};

/** 校验 Rust command 错误载荷并生成前端错误。 */
export const parseApplicationCommandError = (value: unknown): ApplicationCommandError => {
    if (typeof value !== "object" || value === null) throw new ApplicationContractError("invalid-application-error", "应用错误数据无效");

    // 等待字段校验的 command 错误载荷。
    const candidate = value as Record<string, unknown>;
    if (typeof candidate.code !== "string" || candidate.code.length === 0) throw new ApplicationContractError("invalid-application-error", "应用错误数据无效");
    if (typeof candidate.message !== "string" || candidate.message.length === 0)
        throw new ApplicationContractError("invalid-application-error", "应用错误数据无效");

    return new ApplicationCommandError(candidate.code, candidate.message);
};
