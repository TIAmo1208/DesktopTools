/** 插件 loopback 通道的模式：宿主监听接推送，或宿主向本机端点发请求。 */
export type LoopbackChannelMode = "listen" | "request";

/** 一条通道的注册请求；端口与地址由模式二选一。 */
export interface LoopbackChannelRegistration {
    instanceId: string; // 通道所属实例。
    channelId: string; // 插件内稳定的通道标识。
    mode: LoopbackChannelMode; // 方向。
    port?: number; // listen：宿主绑定的本机端口。
    baseUrl?: string; // request：对端本机地址，不带路径。
    credentialRef?: string; // Windows Credential Manager 目标名。
    headerName?: string; // 注入或校验的请求头名。
}

/** 注册成功后宿主给出的通道句柄。 */
export interface LoopbackChannelHandle {
    channelId: string;
    mode: LoopbackChannelMode;
    port: number | null; // request 通道没有本地端口。
    baseUrl: string | null; // listen 通道没有对端地址。
    registeredAt: number; // 注册时刻，Unix 秒。
}

/** 通道收到的一条原始请求；宿主不解析正文。 */
export interface LoopbackRequestEntry {
    sequence: number; // 宿主递增序号。
    receivedAt: number; // 收到时刻，Unix 秒。
    method: string; // 原始方法。
    path: string; // 原始路径。
    headers: Array<{ name: string; value: string }>; // 保留重复与顺序。
    body: string; // 原始正文文本，超出上限的请求不会出现。
}

/** request 通道的原始响应。 */
export interface LoopbackResponse {
    status: number; // HTTP 状态码，非 2xx 由插件判断。
    headers: Array<{ name: string; value: string }>;
    body: string;
}

/** 通道数据结构错误。 */
export class LoopbackContractError extends Error {
    readonly code = "invalid-loopback-contract"; // 可稳定检索的契约错误码。

    /** 创建 loopback 契约错误。 */
    constructor() {
        super("loopback 通道数据无效");
        this.name = "LoopbackContractError";
    }
}

/** 判断未知值是否为正整数。 */
const isPositiveInteger = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) > 0;

/** 校验原始请求头：名称非空，值允许空串，保留顺序。 */
const parseHeaders = (value: unknown): Array<{ name: string; value: string }> => {
    if (!Array.isArray(value)) throw new LoopbackContractError();
    return value.map((header) => {
        if (typeof header !== "object" || header === null) throw new LoopbackContractError();
        const item = header as Record<string, unknown>; // 待校验的单个请求头。
        if (typeof item.name !== "string" || !item.name || typeof item.value !== "string") throw new LoopbackContractError();
        return { name: item.name, value: item.value };
    });
};

/** 校验通道句柄；listen 必须有端口，request 必须有本机地址。 */
export const parseLoopbackChannelHandle = (value: unknown): LoopbackChannelHandle => {
    if (typeof value !== "object" || value === null) throw new LoopbackContractError();
    const item = value as Record<string, unknown>; // 待校验的通道句柄。
    if (typeof item.channelId !== "string" || !item.channelId) throw new LoopbackContractError();
    if (item.mode !== "listen" && item.mode !== "request") throw new LoopbackContractError();
    if (item.port !== null && !isPositiveInteger(item.port)) throw new LoopbackContractError();
    if (item.baseUrl !== null && typeof item.baseUrl !== "string") throw new LoopbackContractError();
    if (item.mode === "listen" ? item.port === null : item.baseUrl === null) throw new LoopbackContractError();
    if (!isPositiveInteger(item.registeredAt)) throw new LoopbackContractError();
    return {
        channelId: item.channelId,
        mode: item.mode,
        port: item.port,
        baseUrl: item.baseUrl,
        registeredAt: item.registeredAt,
    };
};

/** 校验一条原始请求；read 的数组项逐条复用。 */
export const parseLoopbackRequestEntry = (value: unknown): LoopbackRequestEntry => {
    if (typeof value !== "object" || value === null) throw new LoopbackContractError();
    const item = value as Record<string, unknown>; // 待校验的缓冲条目。
    if (!isPositiveInteger(item.sequence) || !isPositiveInteger(item.receivedAt)) throw new LoopbackContractError();
    if (typeof item.method !== "string" || typeof item.path !== "string" || typeof item.body !== "string") throw new LoopbackContractError();
    return { sequence: item.sequence, receivedAt: item.receivedAt, method: item.method, path: item.path, headers: parseHeaders(item.headers), body: item.body };
};

/** 校验一次读取返回的原始请求数组。 */
export const parseLoopbackRequestEntries = (value: unknown): LoopbackRequestEntry[] => {
    if (!Array.isArray(value)) throw new LoopbackContractError();
    return value.map(parseLoopbackRequestEntry);
};

/** 校验 request 通道返回的原始响应。 */
export const parseLoopbackResponse = (value: unknown): LoopbackResponse => {
    if (typeof value !== "object" || value === null) throw new LoopbackContractError();
    const item = value as Record<string, unknown>; // 待校验的原始响应。
    if (!Number.isSafeInteger(item.status) || (item.status as number) < 100 || (item.status as number) > 599) throw new LoopbackContractError();
    if (typeof item.body !== "string") throw new LoopbackContractError();
    return { status: item.status as number, headers: parseHeaders(item.headers), body: item.body };
};
