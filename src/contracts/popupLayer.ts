/** 桌面置顶弹层的种类；与 Rust 侧 `PopupLayerKind` 使用同一组 kebab-case 值。 */
export type PopupLayerKind = "menu" | "confirm" | "weather-city" | "weather-source";

/** 锚点四周可用空间（DIP），页面据此决定二级菜单方向与最大高度。 */
export interface PopupLayerSpace {
    left: number;
    right: number;
    top: number;
    bottom: number;
}

/** 弹层页面装配所需的请求视图。 */
export interface PopupLayerView {
    requestId: number;
    kind: PopupLayerKind;
    available: PopupLayerSpace;
    payload: unknown;
}

/** 弹层回传给宿主的结果；菜单命令由宿主按 `indexPath` 找回原闭包执行。 */
export interface PopupLayerResult {
    requestId: number;
    kind: PopupLayerKind;
    indexPath?: number[];
    confirmed?: boolean;
    value?: unknown;
    dismissed: boolean;
    keepOpen?: boolean; // 菜单命令保持打开：本次只回传选择，请求不结束。
}

/** 弹层载荷与尺寸上报的结构约定。 */
export interface PopupLayerFrame {
    requestId: number;
    width: number;
    height: number;
    anchorOffsetX: number;
    anchorOffsetY: number;
}

const KINDS: readonly PopupLayerKind[] = ["menu", "confirm", "weather-city", "weather-source"];

/** 标识弹层契约边界的数据错误。 */
export class PopupLayerContractError extends Error {
    readonly code = "invalid-popup-layer"; // 可稳定检索的契约错误码。

    /** 创建弹层契约错误。 */
    constructor(message = "弹层数据无效") {
        super(message);
        this.name = "PopupLayerContractError";
    }
}

/** 只接受普通对象，拒绝数组和 null。 */
const record = (value: unknown): Record<string, unknown> => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) throw new PopupLayerContractError();
    return value as Record<string, unknown>;
};

/** 有限数值校验；弹层坐标与尺寸不允许 NaN 或无穷。 */
const finite = (value: unknown): number => {
    if (typeof value !== "number" || !Number.isFinite(value)) throw new PopupLayerContractError();
    return value;
};

/** 命令路径只允许非负整数下标。 */
const indexPath = (value: unknown): number[] => {
    if (!Array.isArray(value) || value.length === 0 || value.length > 8) throw new PopupLayerContractError();
    return value.map((entry) => {
        if (!Number.isSafeInteger(entry) || (entry as number) < 0) throw new PopupLayerContractError();
        return entry as number;
    });
};

/** 读取请求视图；窗口已销毁时 Rust 返回 null。 */
export const parsePopupLayerView = (value: unknown): PopupLayerView | null => {
    if (value === null || value === undefined) return null;
    const item = record(value);
    if (!KINDS.includes(item.kind as PopupLayerKind)) throw new PopupLayerContractError();
    if (!Number.isSafeInteger(item.requestId) || (item.requestId as number) <= 0) throw new PopupLayerContractError();
    const space = record(item.available);
    return {
        requestId: item.requestId as number,
        kind: item.kind as PopupLayerKind,
        available: { left: finite(space.left), right: finite(space.right), top: finite(space.top), bottom: finite(space.bottom) },
        payload: item.payload ?? {},
    };
};

/** 读取结果事件；与请求身份不符的迟到结果由调用方丢弃。 */
export const parsePopupLayerResult = (value: unknown): PopupLayerResult => {
    const item = record(value);
    if (!KINDS.includes(item.kind as PopupLayerKind)) throw new PopupLayerContractError();
    if (!Number.isSafeInteger(item.requestId) || (item.requestId as number) <= 0) throw new PopupLayerContractError();
    if (typeof item.dismissed !== "boolean") throw new PopupLayerContractError();
    const result: PopupLayerResult = { requestId: item.requestId as number, kind: item.kind as PopupLayerKind, dismissed: item.dismissed };
    if (item.indexPath !== undefined) result.indexPath = indexPath(item.indexPath);
    if (item.confirmed !== undefined) {
        if (typeof item.confirmed !== "boolean") throw new PopupLayerContractError();
        result.confirmed = item.confirmed;
    }
    if (item.value !== undefined) result.value = item.value;
    if (item.keepOpen !== undefined) {
        if (typeof item.keepOpen !== "boolean") throw new PopupLayerContractError();
        result.keepOpen = item.keepOpen;
    }
    return result;
};
