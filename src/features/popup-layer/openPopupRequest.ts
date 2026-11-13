import { currentHostMonitorIndex, listenPopupLayerResult, openPopupLayer } from "../../bridge/popupLayer";
import type { PopupLayerKind, PopupLayerResult } from "../../contracts/popupLayer";
import type { WeatherCity, WeatherSource } from "../../contracts/weather";
import type { ConfirmPayload } from "./PopupConfirm";
import type { WeatherCityPayload, WeatherSourcePayload } from "./PopupWeather";

/** 宿主侧弹层请求的自增序号；与时间戳拼成进程内唯一身份。 */
let sequence = 0;

/** 生成一次请求身份；弹层结果必须回传同一值才算本次请求。 */
const nextRequestId = (): number => {
    sequence = (sequence + 1) % 1000;
    return Date.now() * 1000 + sequence;
};

/**
 * 打开一次弹窗类浮层并等待结果。
 *
 * 对话框不使用光标作为锚点：桌面宿主客户区就是显示器工作区，因此取页面中心，
 * 由弹层页面把窗口摆到工作区中心。
 */
const openRequest = (kind: PopupLayerKind, payload: Record<string, unknown>): Promise<PopupLayerResult | undefined> =>
    new Promise((resolve) => {
        const requestId = nextRequestId();
        let active = true; // 退订后丢弃迟到结果。
        let stop: (() => void) | undefined;
        const release = () => {
            active = false;
            stop?.();
            stop = undefined;
        };
        const anchorX = window.innerWidth / 2; // 宿主客户区 DIP 中心。
        const anchorY = window.innerHeight / 2;
        void listenPopupLayerResult((result) => {
            if (!active || result.requestId !== requestId) return;
            release();
            resolve(result.dismissed ? undefined : result);
        })
            .then((unlisten) => {
                if (active) stop = unlisten;
                else unlisten();
                return openPopupLayer({ requestId, kind, monitor: currentHostMonitorIndex(), anchorX, anchorY, payload });
            })
            .catch((error: unknown) => {
                release();
                resolve(undefined);
                console.warn("弹层：打开失败", error);
            });
    });

/** 确认框：返回用户是否确认。 */
export const openPopupConfirm = async (payload: ConfirmPayload): Promise<boolean> => {
    const result = await openRequest("confirm", payload as unknown as Record<string, unknown>);
    return result?.confirmed === true;
};

/** 天气城市弹窗：返回用户确认的城市，写入城市列表仍由宿主完成。 */
export const openPopupWeatherCity = async (payload: WeatherCityPayload): Promise<WeatherCity | undefined> => {
    const result = await openRequest("weather-city", payload as unknown as Record<string, unknown>);
    const city = (result?.value as { city?: WeatherCity } | undefined)?.city;
    return city && typeof city.id === "string" ? city : undefined;
};

/** 天气来源弹窗：返回用户选择的来源，写入格子仍由宿主完成。 */
export const openPopupWeatherSource = async (payload: WeatherSourcePayload): Promise<WeatherSource | undefined> => {
    const result = await openRequest("weather-source", payload as unknown as Record<string, unknown>);
    const source = (result?.value as { source?: WeatherSource } | undefined)?.source;
    return source === "qweather" || source === "foreca" ? source : undefined;
};
