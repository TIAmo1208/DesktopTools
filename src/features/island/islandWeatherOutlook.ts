/**
 * 天气展望纯模型：没有 API 未来预报文案时，用已获取的天况事实自行推算一句短展望。
 * 只消费调用方传入的时间与事实（不读 Date.now），只依据仓库已有的字段与语义，
 * 表达不了的组合返回 undefined，由调用方不显示，不编造文案。
 */

/** 展开态小时槽与当前天况共用的最小预报字段；结构与 weather / 浮岛收到的预报线一致。 */
export interface WeatherOutlookFacts
{
    current?: { temperature?: { value?: number }; condition?: { text?: string; code?: string } };
    hourly?: { hours?: { forecastTime?: string; temperature?: { value?: number }; condition?: { text?: string; code?: string } }[] };
}

/** 展望的图标语义；渲染层按 kind 与 code 选择天气图形。 */
export type WeatherOutlookKind = "rain" | "clear-rain" | "cloud" | "sun" | "cooler" | "warmer";

/** 天气展望：一句话短文案 + 目标时段的天气代码（供图标选择）。 */
export interface WeatherOutlook
{
    text: string;
    code: number;
    kind: WeatherOutlookKind;
}

const HOUR_MS = 3_600_000; // 小时槽的覆盖长度，来自供应商逐小时粒度。
/** 仓库统一的降水边界：WeatherGlyph 与浮岛口径都以 code ≥ 300 视为降水。 */
const RAIN_CODE = 300;
/** 温差展望阈值：较当前 ≥ 3℃ 才认为明显。 */
const SWING_CELSIUS = 3;

/** 当前正降雪（含雪不含雨）时不表达雨停；雪的起停不在本模型文案内。 */
const isSnowyNow = (raining: boolean, text?: string): boolean => raining === true && typeof text === "string" && text.includes("雪") && !text.includes("雨");

/** 小时槽类型；与预报线一致，各字段都可能缺失。 */
type WeatherOutlookHour = NonNullable<NonNullable<WeatherOutlookFacts["hourly"]>["hours"]>[number];

/** 供应商 weather code 解析成数字；缺字段或不可解析的代码不参与判定。 */
const weatherCode = (condition?: { code?: string }): number | undefined =>
{
    const code = condition?.code;
    if (typeof code !== "string") return undefined;
    const value = Number(code);
    return Number.isFinite(value) ? value : undefined;
};

/** 短语的时间量词：不足一小时写「稍后」，一小时整写「1小时后」，其余取最接近的整小时。 */
const timePhrase = (deltaMs: number): string | undefined =>
{
    if (deltaMs < HOUR_MS) return undefined;
    return `${Math.round(deltaMs / HOUR_MS)}小时后`;
};

/** 天况转变的固定短词：按来源文本的固定字面分类，识别不了的文本不表达。 */
const skyWord = (text?: string): "转晴" | "转多云" | "转阴" | undefined =>
{
    if (typeof text !== "string") return undefined;
    if (text.includes("阴")) return "转阴";
    if (text.includes("云")) return "转多云";
    if (text.includes("晴")) return "转晴";
    return undefined;
};

/** 降水事件文案：文本明说雪时不在本模型表达（任务只覆盖雨），其余 ≥ 300 槽按仓库雨形口径表达为雨。 */
const rainWord = (raining: boolean, text?: string): "有雨" | "雨停" | undefined =>
{
    if (typeof text === "string" && text.includes("雪") && !text.includes("雨")) return undefined;
    return raining ? "雨停" : "有雨";
};

/** 小时槽的开始时刻；无效时间戳不参与判定。 */
const hourStart = (forecastTime?: string): number | undefined =>
{
    const value = Date.parse(forecastTime ?? "");
    return Number.isFinite(value) ? value : undefined;
};

interface RainBoundary { at: number; code: number; word: "有雨" | "雨停"; }

/** 规则一：最近的雨起或雨停边界；与当前雨况相反的最早小时槽，槽已开始的按现在时刻起算。 */
const findRainBoundary = (nowMs: number, hours: readonly WeatherOutlookHour[] | undefined, raining: boolean): RainBoundary | undefined =>
{
    let best: RainBoundary | undefined;
    for (const hour of hours ?? [])
    {
        const start = hourStart(hour?.forecastTime);
        if (start === undefined || start + HOUR_MS <= nowMs) continue; // 已结束的槽与未来事件无关。
        const code = weatherCode(hour?.condition);
        if (code === undefined || (code >= RAIN_CODE) === raining) continue; // 只找与当前雨况相反的边界。
        const at = Math.max(start, nowMs);
        const word = rainWord(raining, hour?.condition?.text);
        if (word === undefined) continue; // 雪槽按「不编造」跳过，留给后续情形。
        if (!best || at < best.at) best = { at, code, word };
    }
    return best;
};

interface SkyChange { at: number; code: number; word: NonNullable<ReturnType<typeof skyWord>>; kind: "cloud" | "sun"; }

/** 规则二：无降水事件时最早的天况转变；当前文案不可分类时不推断。 */
const findSkyChange = (nowMs: number, hours: readonly WeatherOutlookHour[] | undefined, currentText?: string): SkyChange | undefined =>
{
    const from = skyWord(currentText);
    if (!from) return undefined;
    let best: SkyChange | undefined;
    for (const hour of hours ?? [])
    {
        const start = hourStart(hour?.forecastTime);
        if (start === undefined || start + HOUR_MS <= nowMs) continue;
        const code = weatherCode(hour?.condition);
        if (code === undefined || code >= RAIN_CODE) continue; // 降水边界已由规则一优先表达。
        const word = skyWord(hour?.condition?.text);
        if (!word || word === from) continue;
        const at = Math.max(start, nowMs);
        if (!best || at < best.at) best = { at, code, word, kind: word === "转晴" ? "sun" : "cloud" };
    }
    return best;
};

interface TemperatureSwing { at: number; degrees: number; rise: boolean; }

/** 规则三：最早的明显温差；只看有有限温度且越过多阈值的小时槽。 */
const findTemperatureSwing = (nowMs: number, hours: readonly WeatherOutlookHour[] | undefined, current?: number): TemperatureSwing | undefined =>
{
    if (current === undefined || !Number.isFinite(current)) return undefined;
    let best: TemperatureSwing | undefined;
    for (const hour of hours ?? [])
    {
        const start = hourStart(hour?.forecastTime);
        if (start === undefined || start + HOUR_MS <= nowMs) continue;
        const value = hour?.temperature?.value;
        if (typeof value !== "number" || !Number.isFinite(value)) continue;
        const delta = value - current;
        if (Math.abs(delta) < SWING_CELSIUS) continue;
        const at = Math.max(start, nowMs);
        if (!best || at < best.at) best = { at, degrees: Math.round(Math.abs(delta)), rise: delta > 0 };
    }
    return best;
};

/** 按优先级推算一句展望：雨边界 > 天况转变 > 明显温差，其余一律 undefined。 */
export const islandWeatherOutlook = (nowMs: number, facts?: WeatherOutlookFacts): WeatherOutlook | undefined =>
{
    if (!Number.isFinite(nowMs)) return undefined;
    const code = weatherCode(facts?.current?.condition);
    if (code === undefined) return undefined; // 当前天况缺失时不推断雨况。
    const raining = code >= RAIN_CODE;
    const hours = facts?.hourly?.hours;
    const rain = isSnowyNow(raining, facts?.current?.condition?.text) ? undefined : findRainBoundary(nowMs, hours, raining);
    if (rain)
    {
        const when = timePhrase(rain.at - nowMs);
        return { text: `预计${when ?? "稍后"}${rain.word}`, code: rain.code, kind: rain.word === "有雨" ? "rain" : "clear-rain" };
    }
    const sky = findSkyChange(nowMs, hours, facts?.current?.condition?.text);
    if (sky)
    {
        const when = timePhrase(sky.at - nowMs);
        return { text: `预计${when ?? "稍后"}${sky.word}`, code: sky.code, kind: sky.kind };
    }
    const swing = findTemperatureSwing(nowMs, hours, facts?.current?.temperature?.value);
    if (swing)
    {
        const when = timePhrase(swing.at - nowMs);
        const word = swing.rise ? "升温" : "降温";
        return { text: `预计${when ?? "稍后"}${word}${swing.degrees}℃`, code, kind: swing.rise ? "warmer" : "cooler" };
    }
    return undefined;
};
