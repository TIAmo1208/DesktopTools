import assert from "node:assert/strict";
import test, { after } from "node:test";
import { readdir, readFile } from "node:fs/promises";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const file = new URL("./SettingsCityList.tsx", import.meta.url);
const pickerFile = new URL("../weather/WeatherSourcePicker.tsx", import.meta.url);
const resources = new URL("../../../resources/", import.meta.url);
// 同一份列表在来源展开区与编辑弹窗内共用，渲染契约只按外部可观察结果断言。
const city = { id: "101010100", name: "东城", adm1: "北京市", adm2: "北京", country: "中国", lat: "39.92", lon: "116.41", tz: "Asia/Shanghai", fxLink: "" };
// 两个天气来源共用一次 Vite 冷启动，避免逐个测试文件重新加载模块。
const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
after(() => server.close());

test("城市列表项的渲染与删除入口契约", async () => {
    const { SettingsCityList } = await server.ssrLoadModule("/src/features/settings/SettingsCityList.tsx"); // 生产实现。
    const expanded = renderToStaticMarkup(createElement(SettingsCityList, { cities: [city], onDelete: () => {}, onAdd: () => {} }));
    assert.match(expanded, /东城/);
    assert.match(expanded, /北京，北京市，中国/); // 行政区副标题按 adm2、adm1、country 顺序降级显示。
    assert.match(expanded, /aria-label="删除 东城"/, "删除图标带辅助技术名称");
    assert.match(expanded, /title="删除"/, "删除图标带悬停提示");
    assert.match(expanded, /settings-city__delete/);
    assert.doesNotMatch(expanded, /settings-city__empty/, "有城市项时不显示空态文案");
    assert.match(expanded, /<button[^>]*settings-city__add[^>]*>添加城市或地区<\/button>/);

    // 列表为空时同时给出空态文案与末行添加命令。
    const empty = renderToStaticMarkup(createElement(SettingsCityList, { cities: [], onDelete: () => {}, onAdd: () => {} }));
    assert.doesNotMatch(empty, /aria-label="删除/);
    assert.match(empty, /<p class="settings-city__empty">尚未添加城市或地区<\/p>/);
    assert.match(empty, /<button[^>]*settings-city__add[^>]*>添加城市或地区<\/button>/, "空列表保留添加命令");

    // 编辑弹窗的城市列表没有删除入口时显示空态文案，也不出现添加入口。
    const dialog = renderToStaticMarkup(createElement(SettingsCityList, { cities: [] }));
    assert.match(dialog, /<p class="settings-city__empty">尚未添加城市或地区<\/p>/, "为空时显示空态文案");
    assert.doesNotMatch(dialog, /settings-city__add/, "编辑弹窗的空态不重复给出添加入口");
    assert.doesNotMatch(dialog, /aria-label="删除/);

    // busy 期间禁用全部操作入口：城市项不可聚焦、删除与添加按钮均禁用。
    const busy = renderToStaticMarkup(createElement(SettingsCityList, { cities: [city], onSelect: () => {}, onDelete: () => {}, onAdd: () => {}, busy: true }));
    assert.doesNotMatch(busy, /tabindex=/, "忙时城市项不可聚焦");
    assert.match(busy, /<button[^>]*settings-city__delete[^>]*disabled/, "忙时删除按钮禁用");
    assert.match(busy, /<button[^>]*settings-city__add[^>]*disabled/, "忙时添加按钮禁用");

    // 非 busy 时恢复可操作入口。
    const idle = renderToStaticMarkup(
        createElement(SettingsCityList, { cities: [city], onSelect: () => {}, onDelete: () => {}, onAdd: () => {}, busy: false }),
    );
    assert.match(idle, /tabindex="0"/, "空闲时城市项可聚焦");
    assert.doesNotMatch(idle, /disabled/, "空闲时按钮均可用");

    // 删除图标来自图标资源，不使用文字“删除”，也不导入桥接层。
    const source = await readFile(file, "utf8");
    assert.match(source, /weather-trash\.svg/);
    assert.doesNotMatch(source, />\s*删除\s*</);
    assert.doesNotMatch(source, /from "[^"]*bridge\//);
    // 右键只打开调用方提供的菜单，删除命令由菜单触发，不再直接弹出确认框。
    assert.match(source, /onMenu\(city, \{ left: event\.clientX, top: event\.clientY \}\)/);
    assert.doesNotMatch(source, /if \(!busy\) onDelete\(city\)/, "右键不直接触发删除");
});

test("天气来源选择浮层的渲染与关闭契约", async () => {
    const { WeatherSourcePicker } = await server.ssrLoadModule("/src/features/weather/WeatherSourcePicker.tsx"); // 生产实现。
    const base = { identity: {}, availability: "available", cities: [] };
    const sources = [
        { ...base, source: "foreca", address: "https://p.foreca.example" },
        { ...base, source: "qweather", address: "https://api.qweather.example" },
    ];
    const rendered = renderToStaticMarkup(createElement(WeatherSourcePicker, { sources, onSelect: () => {}, onClose: () => {} }));
    assert.match(rendered, /role="dialog"/, "浮层声明为对话框");
    assert.match(rendered, /aria-modal="false"/, "非模态浮层不做焦点陷阱");
    assert.match(rendered, /aria-label="选择 API"/);
    assert.match(rendered, /aria-label="关闭"/, "关闭按钮带辅助技术名称");
    assert.match(rendered, /title="关闭"/, "关闭按钮带悬停提示");
    assert.ok(rendered.indexOf("Foreca") < rendered.indexOf("和风天气"), "来源按传入顺序渲染");
    assert.match(rendered, /https:\/\/p\.foreca\.example/);
    assert.match(rendered, /https:\/\/api\.qweather\.example/);
    assert.doesNotMatch(rendered, /weather-source__empty/, "有来源时不显示空态文案");

    const empty = renderToStaticMarkup(createElement(WeatherSourcePicker, { sources: [], onSelect: () => {}, onClose: () => {} }));
    assert.match(empty, /<small class="weather-source__empty">列表为空<\/small>/);

    // node SSR 无法派发事件，关闭行为以源码契约断言：Escape 监听与点击浮层外收起。
    const source = await readFile(pickerFile, "utf8");
    assert.match(source, /event\.key === "Escape"/, "Escape 关闭浮层");
    assert.match(source, /window\.addEventListener\("keydown"/, "关闭键监听在 window 上");
    assert.match(source, /onPointerDown=/);
    assert.match(source, /event\.target === event\.currentTarget/, "仅点击浮层空白处才关闭");
    assert.doesNotMatch(source, /from "[^"]*bridge\//, "轻量浮层不依赖桥接层");
});

test("天气图标资源契约", async () => {
    const names = (await readdir(resources)).filter((name) => name.endsWith(".svg") && (name.startsWith("menu-") || name.startsWith("weather-")));
    assert.ok(names.length > 0, "存在天气相关图标资源");
    for (const name of names) {
        const svg = await readFile(new URL(name, resources), "utf8");
        assert.match(svg, /viewBox="0 0 24 24"/, `${name} 使用统一画布`);
        assert.match(svg, /currentColor/, `${name} 颜色交给样式控制`);
        assert.doesNotMatch(svg, /fill="#/, `${name} 不写死填充色`);
    }
});

test("城市弹窗的搜索会话与渲染契约", async () => {
    const { WeatherCityDialog, weatherCityReducer } = await server.ssrLoadModule("/src/features/weather/WeatherCityDialog.tsx"); // 共享城市弹窗的真实实现。
    const check = { label: "当前天气", failure: null };
    const base = {
        state: { city, checks: [check], status: "全部能力验证通过", failure: false },
        busy: false,
        onSearch: async () => [],
        onSelect: () => {},
        onValidate: async () => {},
        onConfirm: async () => {},
        onClose: () => {},
    }; // 已通过验证的可用状态。
    const markup = renderToStaticMarkup(createElement(WeatherCityDialog, { ...base, title: "选择验证城市", type: "select" }));
    assert.match(markup, /<h2>选择验证城市<\/h2>/);
    assert.match(markup, /全部能力验证通过/);
    assert.match(markup, /<button[^>]*class="weather-city__primary"[^>]*>确认<\/button>/);
    assert.match(markup, /当前天气/);

    const adding = renderToStaticMarkup(createElement(WeatherCityDialog, { ...base, title: "添加城市", type: "add" }));
    assert.match(adding, /<h2>添加城市<\/h2>/);
    assert.match(adding, /<button[^>]*class="weather-city__primary"[^>]*>添加<\/button>/);

    // 未验证或忙碌时主命令不可确认，失败说明由调用方状态给出。
    const blocked = renderToStaticMarkup(
        createElement(WeatherCityDialog, { ...base, title: "添加城市", type: "add", state: { city, checks: [], status: "", failure: false }, busy: true }),
    );
    assert.match(blocked, /<button[^>]*class="weather-city__primary"[^>]*disabled=""[^>]*>添加<\/button>/);

    // 组件只使用回调，不触发桥接：源文件不导入任何桥接模块。
    const dialogFile = new URL("../weather/WeatherCityDialog.tsx", import.meta.url);
    const source = await readFile(dialogFile, "utf8");
    assert.doesNotMatch(source, /from "[^"]*bridge\//);
    assert.equal((source.match(/^import /gm) ?? []).length, 6); // 仅 React、三个图标资源、契约类型与自身样式。

    const searching = weatherCityReducer(weatherCityReducer({ query: "北京", results: [city] }, { type: "query", value: "上海" }), { type: "searching" });
    assert.deepEqual(searching.results, []); // 开始新搜索时清空旧选项。
    assert.equal(searching.selection, undefined);
    assert.equal(searching.query, "上海");
    const selected = weatherCityReducer(searching, { type: "select", city });
    assert.equal(selected.selection?.id, city.id);
    assert.equal(weatherCityReducer(selected, { type: "results", cities: [city] }).selection?.id, city.id); // 迟到结果不改变本次选择。
});
