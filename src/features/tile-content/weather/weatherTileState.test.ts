import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { runInNewContext } from "node:vm";

const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");

/** 缺失状态由格子两个标识与已配置来源列表推导；执行生产实现而不是复制分支。 */
const loadMissingState = () => {
    const declaration = source.match(/const weatherMissingState = \([\s\S]*?\n\};/)?.[0];
    assert.ok(declaration);
    return runInNewContext(`${stripTypeScriptTypes(declaration)}; weatherMissingState`, {});
};

/** 文案矩阵与按键按缺失原因取值，测试与内容区共用同一份映射。 */
const loadLabels = () => {
    const declaration = source.match(/export const WEATHER_MISSING_LABELS = \{[\s\S]*?\} as const\s*;/)?.[0];
    assert.ok(declaration);
    return runInNewContext(
        stripTypeScriptTypes(declaration)
            .replace("export const", "const")
            .replace(/as const/, "") + "; WEATHER_MISSING_LABELS",
        {},
    );
};

test("底栏署名与详情可用性按格子来源决定，未选来源不显示错误署名", async () => {
    // 署名由来源表提供，格子组件里不再有按来源的三元或分支：新增来源只加表项，不改组件。
    assert.doesNotMatch(source, /const weatherAttribution = \(/); // 不在组件里重复定义署名规则。
    assert.match(source, /\{weatherAttribution\(config\.source\)\} · \{status\}/); // 底栏署名取当前格子所选来源。
    assert.match(source, /const labels = missing \? WEATHER_MISSING_LABELS\[missing\] : undefined;/); // 缺失时只呈现提示，不渲染底栏。
    assert.match(source, /const details = city\?\.fxLink \? [\s\S]*?: undefined;/); // 详情链接为空的来源保持禁用。
    assert.doesNotMatch(source, /和风 · \{status\}/); // 不再写死署名。
    const table = await readFile(new URL("../../../contracts/weather.ts", import.meta.url), "utf8"); // 来源表是署名的唯一来源。
    assert.match(table, /qweather: \{[^}]*attribution: "和风"/);
    assert.match(table, /foreca: \{[^}]*attribution: "Powered by Foreca"/); // 两家署名不同，且都来自表项。
});
test("五种缺失状态各自给出文案与按键", () => {
    const missingState = loadMissingState();
    const labels = loadLabels();
    const qweather = [{ source: "qweather" }];
    const foreca = [{ source: "foreca" }];
    const cases = [
        { sources: [], source: undefined, cityId: undefined, absent: false, state: "no-sources" },
        { sources: foreca, source: undefined, cityId: undefined, absent: false, state: "no-source" },
        { sources: qweather, source: "qweather", cityId: undefined, absent: false, state: "no-city" },
        { sources: qweather, source: "qweather", cityId: "101010100", absent: true, state: "city-removed" },
        { sources: foreca, source: "qweather", cityId: "101010100", absent: false, state: "source-removed" },
    ]; // 键序与内容区的判定顺序一致。
    assert.deepEqual(
        Object.keys(labels),
        cases.map((item) => item.state),
    ); // 五种状态都要有文案。
    for (const item of cases) {
        assert.equal(missingState(item.source, item.cityId, item.sources, item.absent), item.state);
        assert.ok(labels[item.state].message.length > 0);
        assert.ok(labels[item.state].action.length > 0);
    }
    assert.deepEqual(JSON.parse(JSON.stringify(labels["no-sources"])), { message: "尚未在设置中添加天气 API", action: "打开设置" });
    assert.deepEqual(JSON.parse(JSON.stringify(labels["no-source"])), { message: "请选择天气 API", action: "选择 API" });
    assert.deepEqual(JSON.parse(JSON.stringify(labels["no-city"])), { message: "请选择城市或地区", action: "选择城市" });
    assert.deepEqual(JSON.parse(JSON.stringify(labels["city-removed"])), { message: "该城市已从列表移除，请重新选择", action: "选择城市" });
    assert.deepEqual(JSON.parse(JSON.stringify(labels["source-removed"])), { message: "天气 API 配置已删除，请重新选择", action: "选择 API" });
    // 只有列表已成功读取且确实不含该城市才算已移除；未读取或读取失败都不能报“已从列表移除”。
    assert.equal(missingState("qweather", "101010100", qweather, false), undefined);
    assert.equal(missingState("qweather", "101010100", qweather, true), "city-removed");
});

test("切换来源只改来源标识并清空城市标识，城市确认同时写入列表与格子", () => {
    const declaration = source.match(/export const weatherSourcePayload = .*$/m)?.[0];
    assert.ok(declaration);
    const weatherSourcePayload = runInNewContext(stripTypeScriptTypes(declaration).replace("export const", "const") + "; weatherSourcePayload", {});
    // 硬约束：切换来源只提交格子状态，不新增命令，也不保留旧城市标识。
    const payload = weatherSourcePayload("foreca", { weatherSource: "qweather", weatherCityId: "101010100", unrelated: "kept" });
    assert.equal(payload.weatherCityId, undefined); // 城市标识以删除语义提交。
    assert.equal(JSON.stringify(payload), '{"weatherSource":"foreca","unrelated":"kept"}'); // 合并前的载荷不再携带旧城市标识。
    // 合并提交时 undefined 键必须真正删除，否则会被基准状态补回，切源后仍绑定旧城市。
    const mergeDeclaration = source.match(/export const mergeStatePayload = \([\s\S]*?\n\};/)?.[0];
    assert.ok(mergeDeclaration);
    const mergeStatePayload = runInNewContext(stripTypeScriptTypes(mergeDeclaration).replace("export const", "const") + "; mergeStatePayload", {});
    const merged = mergeStatePayload({ weatherSource: "qweather", weatherCityId: "101010100", unrelated: "kept" }, payload);
    assert.equal(JSON.stringify(merged), '{"weatherSource":"foreca","unrelated":"kept"}');
    assert.equal("weatherCityId" in merged, false); // 持久状态不再包含旧城市标识。
    const submit = source.slice(source.indexOf("    const handleSource ="), source.indexOf("    const handleCity ="));
    assert.ok(submit.includes("const handleSource"));
    assert.match(submit, /commitState\(weatherSourcePayload\(next, state\)/); // 沿用现有组件状态提交命令。
    assert.match(submit, /setCityId\(undefined\)/); // 本地城市标识同步清空，不残留旧绑定。
    assert.match(submit, /setCity\(undefined\)/); // 内存中的城市对象一并清空。
    const confirm = source.match(/const confirmCity = \(target: WeatherCity\) =>[\s\S]*?\n    const details/)?.[0];
    assert.ok(confirm);
    assert.match(confirm, /addWeatherCity\(source as WeatherSource, target\)/); // 候选城市对象传入 Rust 校验后写入该来源城市列表。
    assert.match(confirm, /await handleCity\(target\)/); // 再绑定到当前格子。
    assert.match(source, /state: mergeStatePayload\(state, payload\)/); // 沿用现有组件状态提交命令。
});

test("验证只给结论、确认才写入城市列表，两条路径都不带副作用", async () => {
    const settings = await readFile(new URL("../../settings/WeatherSettings.tsx", import.meta.url), "utf8");
    const validate = settings.match(/\/\*\* 按已保存凭据验证候选城市[\s\S]*?\n        \}\);/)?.[0];
    assert.ok(validate); // 验证分支不能改用例掩盖：它只校验候选对象，不写入列表。
    assert.match(validate, /validateWeatherCity\(picker\.source, target\)/); // 候选对象直接交给 Rust 校验。
    assert.doesNotMatch(validate, /addWeatherCity|removeWeatherCity|save-city/); // 验证不带写入或回滚副作用。
    const confirm = settings.match(/\/\*\* 确认时才把候选城市写入该来源城市列表[\s\S]*?\n        \}\);/)?.[0];
    assert.ok(confirm);
    assert.match(confirm, /addWeatherCity\(picker\.source, target\)/); // 点“添加”时才写入该来源城市列表。
    assert.match(confirm, /setSources\(/); // 写入后按返回值刷新城市列表。
    // 两条路径都只使用已保存凭据，未保存时给出可读提示而不是发起请求。
    assert.match(validate, /if \(!sourceSaved\(picker\.source\)\)/);
    assert.match(confirm, /if \(!sourceSaved\(picker\.source\)\) throw new Error/);
    assert.doesNotMatch(settings, /type: "save-city"/); // 编辑弹窗不再走草稿会话写入城市。
    assert.doesNotMatch(settings, /picker\.draft/); // 城市弹窗不再区分草稿与按来源两种形态。
    const tile = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const tileValidate = tile.match(/\/\*\* 按来源验证所选城市[\s\S]*?\n        \}\);/)?.[0];
    assert.ok(tileValidate);
    assert.doesNotMatch(tileValidate, /addWeatherCity|handleCity/); // 格子侧验证只写弹窗状态行，不写入城市列表也不改绑定。
    assert.match(tileValidate, /setCityDialog\(/);
});

test("无已配置来源的按键打开设置窗口，失败保留用户可见反馈", async () => {
    const action = source.match(/    \/\*\* 缺失状态的主按键[\s\S]*?\n    \};/)?.[0];
    assert.ok(action);
    assert.match(source, /import \{ openSettingsWindow \} from "\.\.\/\.\.\/\.\.\/bridge\/desktopHost";/); // 入口由桥接层提供。
    assert.doesNotMatch(source, /请从应用托盘或桌面菜单打开天气设置/); // 不再退化为提示文案。
    const messages: any[] = [];
    const opened: string[] = [];
    const handleMissingAction = runInNewContext(stripTypeScriptTypes(action) + "\nhandleMissingAction", {
        setCityOpen: (value: boolean) => opened.push(`city:${value}`),
        setSourcePickerOpen: (value: boolean) => opened.push(`source:${value}`),
        openSettingsWindow: () => {
            opened.push("settings");
            return Promise.reject("设置窗口打开失败：窗口不可用"); // 跨 Realm 的拒绝值同样进入用户可见反馈。
        },
        publishMessage: (value: unknown) => messages.push(value),
    });
    handleMissingAction("no-sources");
    handleMissingAction("no-sources"); // 每次点击都请求打开，不再只提示一次。
    assert.deepEqual(opened, ["settings", "settings"]);
    assert.equal(messages.length, 0); // 失败原因在延迟回调中给出，不阻塞点击。
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(messages.length, 2, "每次点击失败都要有可见反馈");
    assert.equal(messages[0].type, "error");
    assert.ok(messages[0].content.length > 0);
    handleMissingAction("no-city");
    assert.equal(opened.at(-1), "city:true");
    handleMissingAction("source-removed");
    assert.equal(opened.at(-1), "source:true");
});
