import assert from "node:assert/strict";
import test from "node:test";

import type { WeatherCity } from "../../../contracts/weather.ts";
import {
    WEATHER_EMPTY_DRAFT,
    assembleWeatherCredentials,
    confirmWeatherCityDraft,
    emptyWeatherCityDraft,
    requiredWeatherFields,
    weatherCityCommitSteps,
    weatherDraftChanged,
    weatherDraftCities,
    weatherLeaveDecision,
    withdrawWeatherCityDraft,
} from "./weatherSettingsFlow.ts";

/** 构造一个最小城市对象，只填测试用到的字段。 */
const city = (id: string, name = id): WeatherCity => ({
    id,
    name,
    adm1: "省",
    adm2: "市",
    country: "中国",
    lat: "0",
    lon: "0",
    tz: "Asia/Shanghai",
    fxLink: "",
});

test("新增弹窗要求全部字段，编辑弹窗允许沿用的敏感材料留空", () => {
    assert.deepEqual(requiredWeatherFields("qweather", true), ["host", "developerId", "projectId", "credentialId", "privateKey"]);
    assert.deepEqual(requiredWeatherFields("qweather", false), ["host", "developerId", "projectId", "credentialId"]);
    assert.deepEqual(requiredWeatherFields("foreca", true), ["apiKey"]);
    assert.deepEqual(requiredWeatherFields("foreca", false), []);
});

test("凭据只装配当前来源声明的字段并去掉首尾空白", () => {
    const qweather = assembleWeatherCredentials("qweather", { ...WEATHER_EMPTY_DRAFT, host: " api.example.com ", developerId: " Q123 ", apiKey: "不应提交" });
    assert.deepEqual(qweather, { host: "api.example.com", developerId: "Q123", projectId: "", credentialId: "", privateKey: "" });
    assert.deepEqual(assembleWeatherCredentials("foreca", { ...WEATHER_EMPTY_DRAFT, apiKey: " token " }), { apiKey: "token" });
    assert.deepEqual(assembleWeatherCredentials(undefined, { ...WEATHER_EMPTY_DRAFT, host: "h" }), { host: "h", developerId: "", projectId: "", credentialId: "", privateKey: "" });
});

test("编辑弹窗的城市列表等于已保存项减待删除加待添加", () => {
    const draft = { added: [city("c")], removed: ["b"] };
    assert.deepEqual(
        weatherDraftCities([city("a"), city("b")], draft).map((item) => item.id),
        ["a", "c"],
    );
});

test("确认城市按标识替换，不产生重复行", () => {
    const draft = { added: [city("a", "旧名")], removed: [] };
    const next = confirmWeatherCityDraft(draft, city("a", "新名"));
    assert.deepEqual(next.added.map((item) => item.name), ["新名"]);
    assert.equal(draft.added[0].name, "旧名"); // 原草稿不被改写。
    assert.deepEqual(confirmWeatherCityDraft(emptyWeatherCityDraft(), city("b")).added.map((item) => item.id), ["b"]);
});

test("撤回刚加上的城市只撤销添加，不再记入删除", () => {
    const next = withdrawWeatherCityDraft({ added: [city("a")], removed: [] }, city("a"));
    assert.deepEqual(next, { added: [], removed: [] });
});

test("撤回已保存的城市记入删除且不重复", () => {
    const once = withdrawWeatherCityDraft({ added: [], removed: [] }, city("b"));
    assert.deepEqual(once, { added: [], removed: ["b"] });
    assert.deepEqual(withdrawWeatherCityDraft(once, city("b")), { added: [], removed: ["b"] });
});

test("保存按先添加后移除的顺序提交", () => {
    const steps = weatherCityCommitSteps({ added: [city("c")], removed: ["a", "b"] });
    assert.deepEqual(steps.additions.map((item) => item.id), ["c"]);
    assert.deepEqual(steps.removals, ["a", "b"]);
});

test("草稿变更按打开时快照判断", () => {
    const opened = JSON.stringify(WEATHER_EMPTY_DRAFT);
    assert.equal(weatherDraftChanged({ ...WEATHER_EMPTY_DRAFT }, opened), false);
    assert.equal(weatherDraftChanged({ ...WEATHER_EMPTY_DRAFT, host: "h" }, opened), true);
});

test("离开决策按忙碌与未保存状态收敛", () => {
    assert.equal(weatherLeaveDecision({ busy: "save", dirty: true }), "block");
    assert.equal(weatherLeaveDecision({ busy: "remove", dirty: false }), "block");
    assert.equal(weatherLeaveDecision({ busy: "", dirty: true }), "confirm");
    assert.equal(weatherLeaveDecision({ busy: "probe", dirty: false }), "proceed");
});
