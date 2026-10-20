import { useEffect, useRef } from "react";
import closeIcon from "../../../resources/weather-close.svg?raw";
import { WEATHER_SOURCES, type WeatherSource, type WeatherSourceStatus } from "../../contracts/weather";
import "./WeatherSourcePicker.css";

/** 选择 API 轻量弹层：只列出设置中已配置的来源，来源名与服务地址由来源状态给出。 */
export const WeatherSourcePicker = ({
    sources,
    onSelect,
    onClose,
}: {
    sources: WeatherSourceStatus[]; // 设置中已配置的来源，空数组表示一家都没有。
    onSelect: (source: WeatherSource) => void; // 选中即提交格子来源，由调用方清空城市标识。
    onClose: () => void; // 取消关闭，不改变格子状态。
}) => {
    const panel = useRef<HTMLElement>(null); // 打开后聚焦的浮层本体。

    useEffect(() => {
        panel.current?.focus(); // 非模态浮层不做焦点陷阱，只把焦点收进浮层。
        const handleKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape") onClose(); // 关闭键由页面层接收，不使用顶层模态。
        };
        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [onClose]);

    return (
        <div
            className="weather-source__overlay"
            onPointerDown={(event) => {
                if (event.target === event.currentTarget) onClose(); // 点击浮层以外收起，不拦截桌面其他区域。
            }}
        >
            <section ref={panel} tabIndex={-1} className="weather-source" role="dialog" aria-modal="false" aria-label="选择 API">
                <header className="weather-source__head">
                    <h2>选择 API</h2>
                    <button type="button" className="weather-source__close" aria-label="关闭" title="关闭" onClick={onClose}>
                        <span className="weather-source__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} />
                    </button>
                </header>
                <div className="weather-source__list">
                    {sources.map((item) => (
                        <button type="button" key={item.source} onClick={() => onSelect(item.source)}>
                            <span>{WEATHER_SOURCES[item.source].name}</span>
                            <small>{item.address}</small>
                        </button>
                    ))}
                    {!sources.length && <small className="weather-source__empty">列表为空</small>}
                </div>
            </section>
        </div>
    );
};
