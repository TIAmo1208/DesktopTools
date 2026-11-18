import assert from "node:assert/strict";
import test from "node:test";
import { islandWeatherOutlook, type WeatherOutlook, type WeatherOutlookFacts } from "./islandWeatherOutlook.ts";

const BASE = Date.parse("2026-01-01T12:00:00+08:00"); // 固定基准时刻，测试不依赖真实时钟。

/** 按基准时刻的分钟偏移构造一个逐小时槽。 */
const hour = (minutes: number, condition: { text?: string; code?: string }, value?: number) => ({
    forecastTime: new Date(BASE + minutes * 60_000).toISOString(),
    temperature: value === undefined ? undefined : { value },
    condition,
});

const at = (minutes: number, facts: WeatherOutlookFacts): WeatherOutlook | undefined => islandWeatherOutlook(BASE + minutes * 60_000, facts);

test("数据不足或字段异常时不编造展望", () => {
    assert.equal(islandWeatherOutlook(BASE, undefined), undefined);
    assert.equal(islandWeatherOutlook(BASE, {}), undefined);
    assert.equal(at(0, { current: { temperature: { value: 20 } } }), undefined); // 缺当前天况不推断雨况。
    assert.equal(at(0, { current: { condition: { text: "晴", code: "abc" } }, hourly: { hours: [hour(60, { text: "小雨", code: "305" })] } }), undefined); // 当前代码不可解析。
    assert.equal(islandWeatherOutlook(Number.NaN, { current: { condition: { code: "100" } } }), undefined); // 时间不是有限数。
    const current = { condition: { text: "晴", code: "100" }, temperature: { value: 20 } };
    assert.equal(at(0, { current, hourly: { hours: [] } }), undefined); // 无未来槽。
    assert.equal(at(0, { current, hourly: { hours: [{ forecastTime: "not-a-time", condition: { text: "小雨", code: "305" } }] } }), undefined); // 全部时间戳无效。
    assert.equal(at(0, { current, hourly: { hours: [hour(-90, { text: "小雨", code: "305" })] } }), undefined); // 唯一槽在 90 分钟前已结束。
    assert.equal(at(0, { current, hourly: { hours: [hour(60, { text: "晴", code: "100" }, 20)] } }), undefined); // 温差与天况都不成立。
});

test("无雨转有雨：按距离给时间量词，边界恰好一小时写 1小时后", () => {
    const current = { condition: { text: "晴", code: "100" } };
    const rainAt60 = { current, hourly: { hours: [hour(60, { text: "小雨", code: "305" })] } };
    assert.deepEqual(at(0, rainAt60), { text: "预计1小时后有雨", code: 305, kind: "rain" });
    assert.deepEqual(at(1, rainAt60), { text: "预计稍后有雨", code: 305, kind: "rain" }); // 不足一小时按稍后。
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(37, { text: "阵雨", code: "303" })] } }), { text: "预计稍后有雨", code: 303, kind: "rain" });
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(122, { text: "小雨", code: "305" })] } }), { text: "预计2小时后有雨", code: 305, kind: "rain" }); // 取最接近的整小时。
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(-20, { text: "小雨", code: "305" })] } }), { text: "预计稍后有雨", code: 305, kind: "rain" }); // 覆盖当前的槽从现在起算。
    const twoSlots = { current, hourly: { hours: [hour(122, { text: "小雨", code: "305" }), hour(60, { text: "多云", code: "101" })] } };
    assert.deepEqual(at(0, twoSlots), { text: "预计2小时后有雨", code: 305, kind: "rain" }); // 数组顺序不影响结果。
});

test("正在下雨：找最早的停雨时段；无停雨与转雪都不编造", () => {
    const current = { condition: { text: "小雨", code: "305" } };
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(85, { text: "多云", code: "101" })] } }), { text: "预计1小时后雨停", code: 101, kind: "clear-rain" });
    assert.equal(at(0, { current, hourly: { hours: [hour(60, { text: "中雨", code: "306" })] } }), undefined); // 未来仍是降水槽不算停。
    assert.equal(at(0, { current, hourly: { hours: [hour(60, { text: "小雪", code: "401" })] } }), undefined); // 降雪槽同为降水，找得到停雨才表达。
    assert.deepEqual(at(0, { current: { condition: { text: "小雪", code: "401" } }, hourly: { hours: [hour(60, { text: "多云", code: "101" })] } }), undefined); // 当前在天况不表达雨停。
});

test("无降水事件时表达天况转变：转多云、转晴、转阴", () => {
    const sunnyNow = { condition: { text: "晴", code: "100" } }; // 当前事实形状。
    const cloudyNow = { condition: { text: "多云", code: "101" } };
    const sunnyHour = { text: "晴", code: "100" }; // 小时槽条件形状。
    const cloudyHour = { text: "多云", code: "101" };
    assert.deepEqual(at(0, { current: sunnyNow, hourly: { hours: [hour(85, cloudyHour)] } }), { text: "预计1小时后转多云", code: 101, kind: "cloud" });
    assert.deepEqual(at(0, { current: cloudyNow, hourly: { hours: [hour(60, sunnyHour)] } }), { text: "预计1小时后转晴", code: 100, kind: "sun" });
    assert.deepEqual(at(0, { current: sunnyNow, hourly: { hours: [hour(122, { text: "阴", code: "104" })] } }), { text: "预计2小时后转阴", code: 104, kind: "cloud" });
    assert.deepEqual(at(1, { current: sunnyNow, hourly: { hours: [hour(60, cloudyHour)] } }), { text: "预计稍后转多云", code: 101, kind: "cloud" });
    assert.deepEqual(at(0, { current: cloudyNow, hourly: { hours: [hour(60, { text: "阴转多云", code: "104" })] } }), { text: "预计1小时后转阴", code: 104, kind: "cloud" }); // 按字面首类为准。
    assert.equal(at(0, { current: cloudyNow, hourly: { hours: [hour(60, cloudyHour)] } }), undefined); // 目标与当前同字面不硬转。
    assert.equal(at(0, { current: cloudyNow, hourly: { hours: [hour(60, { text: "Sunny", code: "100" })] } }), undefined); // 不可分类文本不表达。
    const rainFirst = { current: sunnyNow, hourly: { hours: [hour(30, cloudyHour), hour(60, { text: "小雨", code: "305" })] } };
    assert.deepEqual(at(0, rainFirst), { text: "预计1小时后有雨", code: 305, kind: "rain" }); // 降水事件优先于更早的天况转变。
    const nearest = { current: sunnyNow, hourly: { hours: [hour(122, { text: "阴", code: "104" }), hour(60, cloudyHour)] } };
    assert.deepEqual(at(0, nearest), { text: "预计1小时后转多云", code: 101, kind: "cloud" }); // 数组顺序不影响取最近。
});

test("明显温差给升温或降温，转变与降水优先", () => {
    const current = { condition: { text: "晴", code: "100" }, temperature: { value: 20 } };
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(85, { text: "晴", code: "100" }, 17)] } }), { text: "预计1小时后降温3℃", code: 100, kind: "cooler" });
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(122, { text: "晴", code: "100" }, 24)] } }), { text: "预计2小时后升温4℃", code: 100, kind: "warmer" });
    assert.deepEqual(at(0, { current, hourly: { hours: [hour(37, { text: "晴", code: "100" }, 17)] } }), { text: "预计稍后降温3℃", code: 100, kind: "cooler" });
    assert.equal(at(0, { current, hourly: { hours: [hour(60, { text: "晴", code: "100" }, 17.5)] } }), undefined); // 2.5℃ 不算明显。
    const lateSwing = { current, hourly: { hours: [hour(30, { text: "晴", code: "100" }, 19), hour(122, { text: "晴", code: "100" }, 24)] } };
    assert.deepEqual(at(0, lateSwing), { text: "预计2小时后升温4℃", code: 100, kind: "warmer" }); // 首个越阈槽生效。
    const skyFirst = { current, hourly: { hours: [hour(61, { text: "多云", code: "101" }, 17)] } };
    assert.deepEqual(at(0, skyFirst), { text: "预计1小时后转多云", code: 101, kind: "cloud" }); // 天况转变优先于温差。
    const rainFirst = { current: { condition: { text: "晴", code: "100" } }, hourly: { hours: [hour(30, { text: "小雨", code: "305" }), hour(90, { text: "晴", code: "100" }, 24)] } };
    assert.deepEqual(at(0, rainFirst), { text: "预计稍后有雨", code: 305, kind: "rain" });
});

test("时间一律来自调用方：同一事实不同时刻得到不同量词", () => {
    const facts: WeatherOutlookFacts = {
        current: { condition: { text: "晴", code: "100" } },
        hourly: { hours: [hour(90, { text: "小雨", code: "305" })] },
    };
    assert.deepEqual(islandWeatherOutlook(BASE, facts), islandWeatherOutlook(BASE, facts)); // 纯函数：同入参同结果。
    assert.deepEqual(islandWeatherOutlook(BASE, facts), { text: "预计2小时后有雨", code: 305, kind: "rain" }); // 90 分钟取整为 2。
    assert.deepEqual(islandWeatherOutlook(BASE - 55 * 60_000, facts), { text: "预计2小时后有雨", code: 305, kind: "rain" }); // 相对 145 分钟。
    assert.deepEqual(islandWeatherOutlook(BASE + 55 * 60_000, facts), { text: "预计稍后有雨", code: 305, kind: "rain" }); // 相对 35 分钟。
});

test("展望文案保持浮岛短词风格：简短、无标点、时间量词规范", () => {
    const sampleSources: WeatherOutlookFacts[] = [
        { current: { condition: { text: "晴", code: "100" }, temperature: { value: 20 } }, hourly: { hours: [hour(37, { text: "阵雨", code: "303" }, 21), hour(60, { text: "多云", code: "101" }, 19), hour(122, { text: "阴", code: "104" }, 24), hour(183, { text: "小雨", code: "305" }, 16)] } },
        { current: { condition: { text: "小雨", code: "305" } }, hourly: { hours: [hour(85, { text: "多云", code: "101" })] } },
        { current: { condition: { text: "多云", code: "101" } }, hourly: { hours: [hour(60, { text: "晴", code: "100" }, 17)] } },
        { current: { condition: { text: "晴", code: "100" }, temperature: { value: 20 } }, hourly: { hours: [hour(244, { text: "晴", code: "100" }, 25)] } }, // 4 小时后升温 5℃。
    ];
    const samples = sampleSources.map((facts) => islandWeatherOutlook(BASE, facts)).filter((item): item is WeatherOutlook => item !== undefined);
    assert.ok(samples.length >= 4, "四类情形都应产出展望");
    const kinds = new Set(samples.map((item) => item.kind));
    assert.ok(kinds.has("rain") && kinds.has("clear-rain") && kinds.has("warmer"), "覆盖雨、雨停、温差情形");
    for (const outlook of samples) {
        const han = [...outlook.text].filter((character) => /[\u4e00-\u9fff]/u.test(character)).length;
        assert.ok(han <= 12, `文案汉字数超限：${outlook.text}`);
        assert.ok(!/[，。！？；：、]/u.test(outlook.text), `文案带标点：${outlook.text}`);
        assert.match(outlook.text, /^预计(稍后|\d+小时后)/u, `时间量词格式不符：${outlook.text}`);
        assert.equal(Number.isInteger(outlook.code), true, `天气代码应为整数：${outlook.text}`);
    }
    assert.deepEqual(samples.find((item) => item.text.startsWith("预计稍后有雨"))?.text, "预计稍后有雨");
});
