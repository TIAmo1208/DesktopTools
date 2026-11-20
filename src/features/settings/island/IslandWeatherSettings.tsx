import { useEffect, useState } from "react";
import { listenWeatherConfiguration, listWeatherSources } from "../../../bridge/weather";
import { settingsErrorMessage } from "../../../bridge/settings";
import type { ApplicationPreferences } from "../../../contracts/settings";
import { WEATHER_SOURCES, type WeatherSourceStatus } from "../../../contracts/weather";

/** 浮岛天气卡只保存来源与城市标识，城市对象仍由该来源列表维护。 */
export const IslandWeatherSettings = ({
    preferences,
    disabled,
    onPreferences,
}: {
    preferences?: ApplicationPreferences;
    disabled: boolean;
    onPreferences: (patch: Partial<ApplicationPreferences>) => void;
}) => {
    const [sources, setSources] = useState<WeatherSourceStatus[]>();
    const [failure, setFailure] = useState<string>();
    const source = preferences?.islandWeatherSource ?? "";
    const cityId = preferences?.islandWeatherCityId ?? "";
    const current = sources?.find((item) => item.source === source);

    useEffect(() => {
        let active = true; // 卸载后拒绝迟到读取。
        let stop: (() => void) | undefined; // 天气配置变化通知退订入口。
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
        void listenWeatherConfiguration(() => load()).then((unlisten) => {
            if (active) stop = unlisten;
            else unlisten();
        });
        return () => {
            active = false;
            stop?.();
        };
    }, []);

    return (
        <div className="settings__rows">
            {!sources ? <p className="island-settings__empty" role="status">正在读取天气配置…</p> : null}
            {failure ? <p className="island-settings__empty">{failure}</p> : null}
            <label className="settings__row">
                <span className="settings__row-title">天气 API</span>
                <select
                    className="settings__select"
                    value={source}
                    disabled={disabled || !sources?.length}
                    onChange={(event) => onPreferences({ islandWeatherSource: event.currentTarget.value as ApplicationPreferences["islandWeatherSource"], islandWeatherCityId: "" })}
                >
                    <option value="">未选择</option>
                    {(sources ?? []).map((item) => (
                        <option value={item.source} key={item.source}>
                            {WEATHER_SOURCES[item.source].name}
                        </option>
                    ))}
                </select>
            </label>
            <label className="settings__row">
                <span className="settings__row-title">城市或地区</span>
                <select
                    className="settings__select"
                    value={cityId}
                    disabled={disabled || !current?.cities.length}
                    onChange={(event) => onPreferences({ islandWeatherCityId: event.currentTarget.value })}
                >
                    <option value="">未选择</option>
                    {(current?.cities ?? []).map((city) => (
                        <option value={city.id} key={city.id}>
                            {[city.name, city.adm2].filter(Boolean).join(" · ")}
                        </option>
                    ))}
                </select>
            </label>
            {sources && !sources.length ? <p className="island-settings__empty">尚未配置天气 API，请到「功能格子 → 天气」添加。</p> : null}
            {source && sources?.length && !current ? <p className="island-settings__empty">已保存的天气 API 已删除，请重新选择。</p> : null}
            {source && current && !current.cities.length ? <p className="island-settings__empty">该来源尚未添加城市，请在天气设置中添加。</p> : null}
        </div>
    );
};
