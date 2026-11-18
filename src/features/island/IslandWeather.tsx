import { useEffect, useRef, useState } from "react";
import { getWeatherForecast, listenWeatherConfiguration, listenWeatherForecast, listWeatherSources } from "../../bridge/weather";
import { settingsErrorMessage } from "../../bridge/settings";
import type { WeatherSourceStatus } from "../../contracts/weather";
import { usePreferences } from "../../app/preferences/PreferencesProvider";
import { WeatherGlyph } from "../../shared/WeatherGlyph";
import { islandWeatherBinding, islandWeatherProjection, islandWeatherSourceName, type IslandWeatherForecast } from "./islandWeatherModel";
import type { WeatherOutlookFacts } from "./islandWeatherOutlook";
import "./islandWeather.css";

type IslandWeatherMode = "collapsed" | "hover" | "expanded"; // 与浮岛三态一致。

/** 浮岛页持有唯一一份天气订阅；卡片和锚点只读同一份投影。 */
export const useIslandWeather = (instanceId: string | undefined, monitorId?: string) => {
    const { preferences } = usePreferences();
    const [sources, setSources] = useState<WeatherSourceStatus[]>();
    const [failure, setFailure] = useState<string>();
    const [forecast, setForecast] = useState<IslandWeatherForecast & WeatherOutlookFacts>();
    const [configurationEpoch, setConfigurationEpoch] = useState(0); // 来源凭据或城市列表变化后重建订阅与读取。
    const source = preferences?.islandWeatherSource ?? "";
    const cityId = preferences?.islandWeatherCityId ?? "";
    const islandEnabled = preferences?.islandEnabled !== false; // 关闭浮岛只隐藏窗口，页面仍在时需要停止天气活动。
    const binding = sources ? islandWeatherBinding(sources, source, cityId) : undefined;

    useEffect(() => {
        if (!instanceId) return;
        let active = true; // 卸载后拒绝迟到读取。
        let stop: (() => void) | undefined; // 配置通知退订入口。
        const load = () => {
            void listWeatherSources()
                .then((next) => {
                    if (!active) return;
                    setSources(next);
                    setFailure(undefined);
                })
                .catch((error: unknown) => {
                    if (!active) return;
                    setSources([]);
                    setFailure(settingsErrorMessage(error));
                });
        };
        load();
        void listenWeatherConfiguration(() => {
            if (!active) return;
            load();
            setConfigurationEpoch((value) => value + 1); // 凭据或城市列表变化后重读预报，不沿用旧订阅。
        }).then((unlisten) => {
            if (active) stop = unlisten;
            else unlisten();
        });
        return () => {
            active = false;
            stop?.();
        };
    }, [instanceId]);

    // 设置页新增城市不广播配置事件；来源或城市标识变化时重读一次列表，避免新城市被判为已移除。
    const selectionLoaded = useRef(false);
    useEffect(() => {
        if (!selectionLoaded.current) {
            selectionLoaded.current = true;
            return;
        }
        if (!instanceId || !source) return;
        let active = true; // 选择连续变化后拒绝迟到列表。
        void listWeatherSources()
            .then((next) => {
                if (active) setSources(next);
            })
            .catch(() => undefined);
        return () => {
            active = false;
        };
    }, [source, cityId, instanceId]);

    const readySource = binding?.state === "ready" ? binding.source : "";
    const readyCityId = binding?.state === "ready" ? binding.city.id : "";
    useEffect(() => {
        setForecast(undefined);
        setFailure(undefined);
        if (!islandEnabled || !instanceId || !readySource || !readyCityId || !monitorId) return;
        let active = true; // 换来源、换城市或卸载后拒绝旧响应。
        let revision = 0; // 隐藏恢复和配置变化共用同一代次。
        let stop: (() => void) | undefined; // 活动订阅退订入口。
        let pending = false; // 登记完成前只允许一份订阅。
        const config = { source: readySource, cityId: readyCityId } as const;
        const load = () => {
            if (!active || document.visibilityState === "hidden") return;
            if (!stop && !pending) {
                pending = true;
                void listenWeatherForecast(instanceId, monitorId, config, (value) => {
                    const next = value as IslandWeatherForecast;
                    if (active && next?.source === config.source && next?.city?.id === config.cityId) setForecast(next);
                })
                    .then((unlisten) => {
                        if (!active) {
                            unlisten();
                            return;
                        }
                        pending = false;
                        stop = unlisten;
                    })
                    .catch(() => {
                        pending = false;
                        if (active) setFailure("天气订阅失败");
                    });
            }
            const current = ++revision;
            void getWeatherForecast(config.source, config.cityId)
                .then((value) => {
                    if (active && current === revision) {
                        setForecast(value as IslandWeatherForecast);
                        setFailure(undefined);
                    }
                })
                .catch((error: unknown) => {
                    if (active && current === revision) setFailure(settingsErrorMessage(error));
                });
        };
        const handleVisibility = () => {
            if (document.visibilityState === "visible") load();
        };
        load();
        document.addEventListener("visibilitychange", handleVisibility);
        return () => {
            active = false;
            revision++;
            stop?.();
            document.removeEventListener("visibilitychange", handleVisibility);
        };
    }, [islandEnabled, readySource, readyCityId, instanceId, monitorId, configurationEpoch]);

    return {
        ...islandWeatherProjection(binding, forecast, failure),
        // 展望只使用同城、同来源的现有预报，不开启新订阅。
        outlookFacts: binding?.state === "ready" && forecast?.source === binding.source && forecast.city?.id === binding.city.id ? forecast : undefined,
        binding,
        failure,
    };
};

/** 展开卡保留当天详情与来源，紧凑态只消费投影中的锚点摘要。 */
export const IslandWeather = ({ mode, weather }: { mode: IslandWeatherMode; weather: ReturnType<typeof useIslandWeather> }) => {
    const { binding, summary, detail, failure, unit } = weather;
    if (binding?.state !== "ready" || !summary) return <p className="island-weather__status">{unit.primary}</p>;
    return (
        <div className={`island-weather island-weather--${mode}`} aria-label={`${binding.city.name}天气`}>
            <div className="island-weather__main">
                <WeatherGlyph className="island-weather__glyph" code={summary.code} />
                <strong>{summary.temperature}</strong>
                <span>{summary.condition}</span>
            </div>
            {mode !== "collapsed" && <small className="island-weather__detail">{detail}</small>}
            {mode === "expanded" && (
                <small className="island-weather__source">
                    {failure ?? islandWeatherSourceName(binding.source)}
                </small>
            )}
        </div>
    );
};
