import assert from "node:assert/strict";
import test from "node:test";

import { parseWeatherSettingsSnapshot, parseWeatherSourceStatus, parseWeatherTileBinding, parseWeatherTileConfig } from "./weather.ts";
import { parsePreferences } from "./settings.ts";

// 城市对象样例；坐标与链接由后端负责，前端只做字段类型校验。
const city = {
    id: "101010100",
    name: "北京",
    adm1: "北京市",
    adm2: "北京",
    country: "中国",
    lat: "39.90",
    lon: "116.40",
    tz: "Asia/Shanghai",
    fxLink: "https://www.qweather.com/weather/beijing-101010100.html",
};
// 已配置来源样例，不含启用字段与城市列表，用于逐项补齐。
const source = { source: "foreca", address: "weatherapi.foreca.net", identity: {}, availability: "unconfirmed" };

test("普通偏好不再包含天气来源，缺失来源状态字段时按默认值解析", () => {
    const defaults = parsePreferences({ theme: "system", doubleClickToOpen: true }); // 缺字段的旧配置。
    assert.equal("weatherSource" in defaults, false); // 全局来源偏好已删除，缺失即为当前默认行为。
    assert.equal(defaults.weatherEnabled, false); // 天气显隐偏好继续保留。
    assert.equal("weatherSource" in parsePreferences({ ...defaults, weatherSource: "foreca" }), false); // 旧字段被忽略而不是校验失败。
});

test("来源状态去掉启用字段并携带城市列表", () => {
    const status = parseWeatherSourceStatus({ ...source, cities: [city] }); // 后端返回的完整来源状态。
    assert.deepEqual(
        status.cities.map((item) => item.id),
        ["101010100"],
    );
    assert.equal("enabled" in status, false); // 启用字段随全局来源偏好一起消失。
    assert.equal(parseWeatherSourceStatus({ ...source, cities: [] }).cities.length, 0); // 未添加城市时为空列表。
    for (const invalid of [
        { ...source, cities: undefined },
        { ...source, cities: {} },
        { ...source, availability: "enabled" },
        { ...source, source: "other" },
    ])
        assert.throws(() => parseWeatherSourceStatus(invalid));
});

test("设置快照按来源校验城市列表与会话身份", () => {
    const snapshot = parseWeatherSettingsSnapshot({ sessionId: 2, cities: [city], checks: [], sources: [{ ...source, cities: [city] }] });
    assert.equal(snapshot.sessionId, 2);
    assert.equal(snapshot.sources[0].cities[0].name, "北京");
    assert.equal(parseWeatherSettingsSnapshot({ cities: [], checks: [], sources: [] }).sessionId, null); // 未创建草稿时没有会话身份。
    for (const invalid of [{ sessionId: 0, cities: [], checks: [], sources: [] }, { sessionId: 1, cities: [], checks: [], sources: [{ ...source }] }, null])
        assert.throws(() => parseWeatherSettingsSnapshot(invalid));
});

test("格子配置只解析来源与城市标识，缺一即未选择", () => {
    assert.deepEqual(parseWeatherTileConfig({ weatherSource: "qweather", weatherCityId: "101010100" }), { source: "qweather", cityId: "101010100" });
    assert.equal(parseWeatherTileConfig({ weatherMode: "summary" }), undefined); // 新建格子初始不绑定来源与城市。
    assert.equal(parseWeatherTileConfig({ weatherSource: "qweather" }), undefined); // 只有来源不算已选择。
    assert.equal(parseWeatherTileConfig({ weatherSource: "other", weatherCityId: "101010100" }), undefined);
    assert.equal(parseWeatherTileConfig(null), undefined);
    // 两个标识独立解析：只选来源、尚未选城市时来源仍然有效，重启后不退回未选来源。
    assert.deepEqual(parseWeatherTileBinding({ weatherSource: "qweather", weatherCityId: "101010100" }), { source: "qweather", cityId: "101010100" });
    assert.deepEqual(parseWeatherTileBinding({ weatherSource: "qweather" }), { source: "qweather", cityId: undefined });
    assert.deepEqual(parseWeatherTileBinding({ weatherCityId: "101010100" }), { source: undefined, cityId: "101010100" });
    assert.deepEqual(parseWeatherTileBinding({ weatherSource: "other", weatherCityId: 7 }), { source: undefined, cityId: undefined });
    assert.deepEqual(parseWeatherTileBinding(null), {});
});
