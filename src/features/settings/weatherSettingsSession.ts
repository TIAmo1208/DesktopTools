import type { WeatherCredentials } from "../../contracts/weather.ts";

/** 后端草稿会话的端口；测试用假实现替换真实命令。 */
export interface WeatherSessionPorts {
    begin: (credentials: WeatherCredentials) => Promise<number | null>; // 创建绑定当前草稿的后端会话。
    cancel: (sessionId: number) => Promise<void>; // 放弃会话，迟到结果由调用方按身份丢弃。
}

/** 天气设置的异步操作闸：单调身份、后端草稿会话与迟到结果取消。 */
export interface WeatherOperationGate {
    readonly sessionId: number | null; // 当前有效的后端会话身份。
    nextIdentity: () => number; // 开始一次异步操作，返回本次身份。
    isCurrent: (identity: number) => boolean; // 身份是否仍是最新一次操作。
    invalidate: () => void; // 作废当前身份并释放后端会话。
    ensure: (credentials: WeatherCredentials, mounted: () => boolean, identity: number) => Promise<number>; // 取或创建会话，迟到创建立即取消。
}

/**
 * 创建天气设置的操作闸。身份只在 `invalidate` 与 `nextIdentity` 时前进，
 * 会话只在 `ensure` 首次成功时建立；旧身份或已卸载页面拿到的会话当场取消，不写入状态。
 */
export const createWeatherOperationGate = (ports: WeatherSessionPorts): WeatherOperationGate => {
    let revision = 0; // 当前操作身份，用于拒绝迟到结果。
    let session: number | null = null; // 后端草稿会话身份。

    /** 作废身份并释放会话；重复作废不重复取消。 */
    const invalidate = () => {
        revision++;
        const previous = session;
        session = null;
        if (previous !== null) void ports.cancel(previous).catch(() => undefined);
    };

    return {
        get sessionId() {
            return session;
        },
        nextIdentity: () => ++revision,
        isCurrent: (identity) => identity === revision,
        invalidate,
        ensure: async (credentials, mounted, identity) => {
            if (session !== null) return session;
            const created = await ports.begin(credentials); // 仅明确搜索、验证或保存时上传草稿。
            if (created === null) throw new Error("会话创建失败");
            if (!mounted() || identity !== revision) {
                void ports.cancel(created).catch(() => undefined); // 页面已卸载或身份已作废，不保留迟到会话。
                throw new Error("操作已取消");
            }
            session = created;
            return created;
        },
    };
};
