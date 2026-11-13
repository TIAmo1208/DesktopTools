import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import {
    parsePopupLayerResult,
    parsePopupLayerView,
    type PopupLayerFrame,
    type PopupLayerKind,
    type PopupLayerResult,
    type PopupLayerView,
} from "../contracts/popupLayer";
import { invokeRuntime, listenRuntime } from "./runtime";

/** 宿主监听的弹层结果事件；与 Rust 侧常量同名。 */
export const POPUP_LAYER_RESULT_EVENT = "popup-layer-result";
/** 复用弹层窗口时替换请求的事件；页面据此重新读取并量尺寸。 */
export const POPUP_LAYER_REQUEST_EVENT = "popup-layer-request-changed";

/** 宿主发起的一次弹层请求。 */
export interface OpenPopupLayerRequest {
    requestId: number;
    kind: PopupLayerKind;
    monitor: number;
    anchorX: number;
    anchorY: number;
    payload: Record<string, unknown>;
}

/** 宿主请求打开或复用弹层窗口。 */
export const openPopupLayer = async (request: OpenPopupLayerRequest): Promise<void> => {
    await invokeRuntime("open_popup_layer", { request }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 宿主取消当前弹层；省略身份时结束进行中的请求。 */
export const dismissPopupLayer = async (requestId?: number): Promise<void> => {
    await invokeRuntime("dismiss_popup_layer", requestId === undefined ? {} : { requestId }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 弹层页面读取当前请求。 */
export const getPopupLayerRequest = async (): Promise<PopupLayerView | null> => parsePopupLayerView(await invokeRuntime("get_popup_layer_request"));

/** 弹层页面上报内容尺寸与锚点偏移。 */
export const setPopupLayerFrame = async (frame: PopupLayerFrame): Promise<void> => {
    await invokeRuntime("set_popup_layer_frame", { frame });
};

/** 弹层页面回传结果。 */
export const finishPopupLayer = async (result: PopupLayerResult): Promise<void> => {
    await invokeRuntime("finish_popup_layer", { result });
};

/** 宿主侧更新进行中弹层的载荷；复选菜单保持打开时用它刷新勾选状态。 */
export const updatePopupLayerPayload = async (requestId: number, payload: Record<string, unknown>): Promise<void> => {
    await invokeRuntime("update_popup_layer_payload", { requestId, payload }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
};

/** 宿主订阅弹层结果；契约不合法的事件直接丢弃。 */
export const listenPopupLayerResult = (handler: (result: PopupLayerResult) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>(POPUP_LAYER_RESULT_EVENT, (payload) => {
        try {
            handler(parsePopupLayerResult(payload));
        } catch (error) {
            console.warn("弹层：结果载荷无效", error);
        }
    });

/** 弹层页面订阅换请求事件。 */
export const listenPopupLayerRequest = (handler: () => void): Promise<UnlistenFn> => listenRuntime<unknown>(POPUP_LAYER_REQUEST_EVENT, () => handler());

/** 当前宿主页面对应的显示器枚举索引；与逐屏宿主的创建顺序一致。 */
export const currentHostMonitorIndex = (): number => {
    const index = Number.parseInt(new URLSearchParams(window.location.search).get("monitor") ?? "0", 10); // 桌面宿主页面由 Rust 写入。
    return Number.isSafeInteger(index) && index >= 0 ? index : 0;
};
