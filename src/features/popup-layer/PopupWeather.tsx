import { useLayoutEffect, useRef, useState } from "react";

import { searchWeatherCities, validateWeatherCity } from "../../bridge/weather";
import { SOURCE_CITY_CHECK, type WeatherCity, type WeatherSource, type WeatherSourceStatus } from "../../contracts/weather";
import type { PopupLayerSpace } from "../../contracts/popupLayer";
import { WeatherCityDialog, type WeatherCityDialogState, type WeatherCityDialogType } from "../weather/WeatherCityDialog";
import { WeatherSourcePicker } from "../weather/WeatherSourcePicker";
import type { MenuFrame } from "./PopupMenu";

/** 天气城市弹窗载荷：只带本次弹窗需要的来源、形态与当前城市。 */
export interface WeatherCityPayload {
    title: string; // 弹窗标题。
    type: WeatherCityDialogType; // 选择或添加形态。
    source: WeatherSource; // 候选城市的来源。
    city?: WeatherCity; // 打开时已绑定的城市。
}

/** 天气来源弹窗载荷：已配置来源与当前来源。 */
export interface WeatherSourcePayload {
    sources: WeatherSourceStatus[]; // 设置中已配置的来源。
    current?: WeatherSource; // 当前格子的来源。
}

/** 判断载荷是否是可用的城市弹窗内容。 */
export const isWeatherCityPayload = (value: unknown): value is WeatherCityPayload => {
    const item = value as Partial<WeatherCityPayload> | null;
    return typeof item?.title === "string" && (item.type === "add" || item.type === "select") && typeof item.source === "string";
};

/** 判断载荷是否是可用的来源弹窗内容。 */
export const isWeatherSourcePayload = (value: unknown): value is WeatherSourcePayload =>
    Array.isArray((value as Partial<WeatherSourcePayload> | null)?.sources);

/** 弹窗四周留白：窗口略大于本体，阴影不被裁掉。 */
const DIALOG_MARGIN = 16;

/** 弹窗居中：把内容摆到工作区中心，而不是跟随光标。 */
const centerFrame = (bounds: { width: number; height: number }, available: PopupLayerSpace): MenuFrame => {
    const width = bounds.width + DIALOG_MARGIN;
    const height = bounds.height + DIALOG_MARGIN;
    return {
        width,
        height,
        anchorOffsetX: (available.right - available.left) / 2 - width / 2,
        anchorOffsetY: (available.bottom - available.top) / 2 - height / 2,
    };
};

/** 置顶弹层里的天气城市弹窗：搜索与验证留在弹层，写入城市列表由宿主完成。 */
export const PopupWeatherCity = ({
    payload,
    available,
    onLayout,
    onConfirmed,
    onDismiss,
}: {
    payload: WeatherCityPayload;
    available: PopupLayerSpace;
    onLayout: (frame: MenuFrame) => void;
    onConfirmed: (city: WeatherCity) => void; // 用户确认的城市，尚未写入列表。
    onDismiss: () => void;
}) => {
    const host = useRef<HTMLDivElement>(null); // 用于测量弹窗本体尺寸。
    const reported = useRef(""); // 已上报的摆放。
    const busy = useRef(false); // 串行化城市动作。
    const [state, setState] = useState<WeatherCityDialogState>({ city: payload.city, checks: [], status: "", failure: false });
    const [pending, setPending] = useState(false); // 弹窗内的忙碌状态。

    useLayoutEffect(() => {
        const bounds = host.current?.querySelector<HTMLElement>(".weather-city")?.getBoundingClientRect();
        if (!bounds) return;
        const key = `${bounds.width.toFixed(2)}x${bounds.height.toFixed(2)}`;
        if (reported.current === key) return;
        reported.current = key;
        onLayout(centerFrame(bounds, available));
    }, [available, onLayout]);

    /** 城市动作串行执行，失败原因留在弹窗状态行。 */
    const run = async (action: () => Promise<void>) => {
        if (busy.current) return;
        busy.current = true;
        setPending(true);
        setState((current) => ({ ...current, status: "", failure: false }));
        try {
            await action();
        } catch (error) {
            setState((current) => ({ ...current, failure: true, status: error instanceof Error ? error.message : "天气城市操作失败" }));
        } finally {
            busy.current = false;
            setPending(false);
        }
    };

    return (
        <div ref={host} className="popup-layer__dialog">
            <WeatherCityDialog
                title={payload.title}
                type={payload.type}
                state={state}
                busy={pending}
                onSearch={async (query) => searchWeatherCities(payload.source, query)}
                onSelect={(next) => setState((current) => ({ ...current, city: next, checks: [], status: "", failure: false }))}
                onValidate={(target) =>
                    run(async () => {
                        const settled = await validateWeatherCity(payload.source, target); // 后端可能归一化候选城市。
                        setState((current) => ({ ...current, city: settled, checks: [SOURCE_CITY_CHECK], status: "验证通过" }));
                    })
                }
                onConfirm={async (target) => {
                    onConfirmed(target); // 回传宿主写入，弹窗由原生侧关闭。
                }}
                onClose={onDismiss}
            />
        </div>
    );
};

/** 置顶弹层里的天气来源弹窗：选中即回传，写入格子由宿主完成。 */
export const PopupWeatherSource = ({
    payload,
    available,
    onLayout,
    onSelected,
    onDismiss,
}: {
    payload: WeatherSourcePayload;
    available: PopupLayerSpace;
    onLayout: (frame: MenuFrame) => void;
    onSelected: (source: WeatherSource) => void;
    onDismiss: () => void;
}) => {
    const host = useRef<HTMLDivElement>(null); // 用于测量弹窗本体尺寸。
    const reported = useRef(""); // 已上报的摆放。

    useLayoutEffect(() => {
        const bounds = host.current?.querySelector<HTMLElement>(".weather-source")?.getBoundingClientRect();
        if (!bounds) return;
        const key = `${bounds.width.toFixed(2)}x${bounds.height.toFixed(2)}`;
        if (reported.current === key) return;
        reported.current = key;
        onLayout(centerFrame(bounds, available));
    }, [available, onLayout]);

    return (
        <div ref={host} className="popup-layer__dialog">
            <WeatherSourcePicker sources={payload.sources} onSelect={onSelected} onClose={onDismiss} />
        </div>
    );
};
