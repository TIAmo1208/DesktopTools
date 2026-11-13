import { useCallback, useEffect, useRef, useState } from "react";

import { finishPopupLayer, getPopupLayerRequest, listenPopupLayerRequest, setPopupLayerFrame } from "../../bridge/popupLayer";
import type { PopupLayerResult, PopupLayerView } from "../../contracts/popupLayer";
import type { DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import { isConfirmPayload, PopupConfirm } from "./PopupConfirm";
import { PopupMenu } from "./PopupMenu";
import { isWeatherCityPayload, isWeatherSourcePayload, PopupWeatherCity, PopupWeatherSource } from "./PopupWeather";

import "./popupLayer.css";

/**
 * 置顶弹层页面入口：按当前请求装配一种浮层，结束时回传结果。
 *
 * 页面不持有业务闭包，只负责呈现与测量；宿主收到结果后再执行原本的命令。
 */
export const PopupLayerPage = () => {
    const [view, setView] = useState<PopupLayerView | null>();
    const [failure, setFailure] = useState<string>();
    const current = useRef<number | undefined>(undefined); // 当前生效的请求身份，用于丢弃换请求后迟到的回传。
    const settled = useRef<number | undefined>(undefined); // 已成功结束的请求身份，避免重复回传。
    const settling = useRef<number | undefined>(undefined); // 正在回传的请求身份，避免并发重复。
    const retried = useRef<number | undefined>(undefined); // 已走过一次失败兜底的请求身份，避免重试循环。

    /** 读取当前请求；窗口刚创建与每次换请求都走这一条路径。 */
    const load = useCallback(() => {
        void getPopupLayerRequest()
            .then((next) => {
                if (next) {
                    current.current = next.requestId; // 换请求后，旧请求的迟到回传必须丢弃。
                    setView(next);
                    setFailure(undefined);
                }
            })
            .catch((error: unknown) => setFailure(error instanceof Error ? error.message : "弹层请求读取失败"));
    }, []);

    useEffect(() => {
        load();
        let active = true; // 卸载后拒绝迟到的换请求通知。
        let stop: (() => void) | undefined;
        void listenPopupLayerRequest(() => {
            if (active) load();
        })
            .then((unlisten) => {
                if (active) stop = unlisten;
                else unlisten();
            })
            .catch((error: unknown) => console.warn("弹层：换请求订阅失败", error));
        return () => {
            active = false;
            stop?.();
        };
    }, [load]);

    /**
     * 回传结果：成功后才记为已结束，失败时保留请求让兜底路径重试。
     * `terminal` 为真表示这次回传会结束原生请求；keep-open 的选择不结束。
     */
    const send = useCallback(async (payload: PopupLayerResult, terminal: boolean) => {
        if (payload.requestId !== current.current) return; // 已被新请求替换，丢弃迟到结果。
        if (settling.current === payload.requestId || (terminal && settled.current === payload.requestId)) return;
        settling.current = payload.requestId;
        try {
            await finishPopupLayer(payload);
            if (terminal) {
                settled.current = payload.requestId;
                // 只有结果仍属于当前请求时才清空内容，避免把新请求的界面一起收掉。
                setView((shown) => (shown && shown.requestId === payload.requestId ? undefined : shown));
            }
        } catch (error) {
            setFailure(error instanceof Error ? error.message : "弹层结果回传失败");
        } finally {
            settling.current = undefined;
        }
    }, []);

    /** 结束本次请求并清空内容，窗口由原生侧隐藏。 */
    const finish = useCallback(
        (result: Omit<PopupLayerResult, "requestId" | "kind" | "dismissed"> & { dismissed?: boolean }) => {
            if (!view) return;
            const payload: PopupLayerResult = { ...result, requestId: view.requestId, kind: view.kind, dismissed: result.dismissed === true };
            void send(payload, true);
        },
        [send, view],
    );

    /** 菜单命令要求保持打开：只回传本次选择，原生请求与窗口继续有效。 */
    const keepOpen = useCallback(
        (indexPath: number[]) => {
            if (!view) return;
            void send({ requestId: view.requestId, kind: view.kind, indexPath, dismissed: false, keepOpen: true }, false);
        },
        [send, view],
    );

    /** 回传失败或载荷无效时兜底收束一次：不能留一个没有结果的原生请求。 */
    useEffect(() => {
        if (!failure || !view || settled.current === view.requestId || retried.current === view.requestId) return;
        retried.current = view.requestId;
        void send({ requestId: view.requestId, kind: view.kind, dismissed: true }, true);
    }, [failure, send, view]);

    /** 上报窗口摆放；同一份内容只报一次。 */
    const report = useCallback(
        (frame: { width: number; height: number; anchorOffsetX: number; anchorOffsetY: number }) => {
            if (!view) return;
            void setPopupLayerFrame({ requestId: view.requestId, ...frame }).catch((error: unknown) =>
                setFailure(error instanceof Error ? error.message : "弹层定位失败"),
            );
        },
        [view],
    );

    if (failure) return <div className="popup-layer__failure">{failure}</div>;
    if (!view) return null;

    if (view.kind === "menu") {
        const payload = view.payload as { items?: DesktopTileMenuItem[]; focusFirst?: boolean };
        const items = payload.items;
        if (!Array.isArray(items)) return <InvalidPayload message="弹层菜单载荷无效" onInvalid={setFailure} />;
        return (
            <div className="popup-layer">
                <PopupMenu
                    key={view.requestId}
                    items={items}
                    available={view.available}
                    focusFirst={payload.focusFirst === true}
                    onLayout={report}
                    onSelect={(indexPath, stayOpen) => (stayOpen ? keepOpen(indexPath) : finish({ indexPath }))}
                    onDismiss={() => finish({ dismissed: true })}
                />
            </div>
        );
    }

    if (view.kind === "confirm") {
        if (!isConfirmPayload(view.payload)) return <InvalidPayload message="弹层确认载荷无效" onInvalid={setFailure} />;
        return (
            <div className="popup-layer">
                <PopupConfirm
                    key={view.requestId}
                    payload={view.payload}
                    available={view.available}
                    onLayout={report}
                    onConfirm={() => finish({ confirmed: true })}
                    onDismiss={() => finish({ confirmed: false, dismissed: true })}
                />
            </div>
        );
    }

    if (view.kind === "weather-city") {
        if (!isWeatherCityPayload(view.payload)) return <InvalidPayload message="天气城市弹窗载荷无效" onInvalid={setFailure} />;
        return (
            <div className="popup-layer">
                <PopupWeatherCity
                    key={view.requestId}
                    payload={view.payload}
                    available={view.available}
                    onLayout={report}
                    onConfirmed={(city) => finish({ value: { city } })}
                    onDismiss={() => finish({ dismissed: true })}
                />
            </div>
        );
    }

    if (!isWeatherSourcePayload(view.payload)) return <InvalidPayload message="天气来源弹窗载荷无效" onInvalid={setFailure} />;
    return (
        <div className="popup-layer">
            <PopupWeatherSource
                key={view.requestId}
                payload={view.payload}
                available={view.available}
                onLayout={report}
                onSelected={(source) => finish({ value: { source } })}
                onDismiss={() => finish({ dismissed: true })}
            />
        </div>
    );
};

/** 载荷无效时的可见说明；挂载后立刻把失败原因交给页面收束请求。 */
const InvalidPayload = ({ message, onInvalid }: { message: string; onInvalid: (message: string) => void }) => {
    useEffect(() => onInvalid(message), [message, onInvalid]);
    return <div className="popup-layer__failure">{message}</div>;
};
