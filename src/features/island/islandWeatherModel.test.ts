import assert from "node:assert/strict";
import test from "node:test";
import { islandWeatherBinding, islandWeatherProjection, islandWeatherSummary, islandWeatherToday } from "./islandWeatherModel.ts";
import type { WeatherCity, WeatherSourceStatus } from "../../contracts/weather.ts";

const city: WeatherCity = {
    id: "101010100",
    name: "北京",
    adm1: "北京市",
    adm2: "北京",
    country: "中国",
    lat: "39.9",
    lon: "116.4",
    tz: "Asia/Shanghai",
    fxLink: "",
};

const sources: WeatherSourceStatus[] = [
    { source: "qweather", address: "api.example", identity: {}, availability: "available", cities: [city] },
    { source: "foreca", address: "foreca.example", identity: {}, availability: "unconfirmed", cities: [] },
];

test("浮岛天气按设置中的来源与城市判定缺失态", () => {
    assert.deepEqual(islandWeatherBinding([], "", ""), { state: "no-sources", message: "未配置天气 API" });
    assert.deepEqual(islandWeatherBinding(sources, "", ""), { state: "no-source", message: "请选择天气 API" });
    assert.equal(islandWeatherBinding(sources, "qweather", "").state, "no-city");
    assert.equal(islandWeatherBinding(sources, "qweather", "missing").state, "city-removed");
    assert.equal(islandWeatherBinding(sources, "foreca", "").state, "no-city"); // 来源已配置但没有城市。
    const ready = islandWeatherBinding(sources, "qweather", city.id);
    assert.equal(ready.state, "ready");
    if (ready.state === "ready") assert.equal(ready.city.name, "北京");
});

test("浮岛天气摘要只取当天最高最低温和当前天气", () => {
    const today = todayStart(city.tz);
    const tomorrow = new Date(Date.parse(today) + 86_400_000).toISOString();
    const forecast = {
        source: "qweather" as const,
        city: { id: city.id, name: city.name, tz: city.tz },
        current: { temperature: { value: 12.6 }, condition: { text: "多云", code: "101" } },
        daily: {
            days: [
                { forecastStartTime: today, temperatureMin: { value: 3.2 }, temperatureMax: { value: 15.4 }, daytime: { condition: { code: "101" } } },
                { forecastStartTime: tomorrow, temperatureMin: { value: 1 }, temperatureMax: { value: 9 } },
            ],
        },
    };
    assert.equal(islandWeatherToday(forecast, city)?.forecastStartTime, today);
    assert.deepEqual(islandWeatherSummary(forecast, city), { temperature: "13°", condition: "多云", high: "15°", low: "3°", code: "101" });
    assert.deepEqual(islandWeatherSummary(undefined, city), { temperature: "--", condition: "正在获取", high: "--", low: "--", code: undefined });
});

/** 城市时区当天零点；测试不依赖运行机器的本地日期。 */
const todayStart = (tz: string) => {
    const key = new Intl.DateTimeFormat("en-CA", { timeZone: tz, year: "numeric", month: "2-digit", day: "2-digit" }).format(new Date());
    return `${key}T00:00:00+08:00`;
};

test("紧凑天气共用真实预报与当天摘要，并拒绝上一城市的迟到预报", () => {
    const binding = islandWeatherBinding(sources, "qweather", city.id);
    const now = Date.parse("2026-01-02T15:30:00+08:00");
    const forecast = { source: "qweather" as const, city, current: { temperature: { value: 12.6 }, condition: { text: "多云", code: "101" } } };
    const value = islandWeatherProjection(binding, forecast, undefined, now);
    assert.deepEqual(value.unit, { status: "pending", primary: "13° 多云" });
    assert.equal(value.summary?.code, "101");
    assert.match(value.detail ?? "", /^北京 · 今天/);
    assert.deepEqual(islandWeatherProjection(binding, { ...forecast, city: { id: "old" } }, undefined, now).unit, { status: "indeterminate", primary: "正在获取天气" });
    assert.deepEqual(islandWeatherProjection(binding, undefined, "天气订阅失败", now).unit, { status: "failed", primary: "天气订阅失败" });
});
