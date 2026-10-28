import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createServer } from "vite";

// 通过已有 Vite 加载真实 TSX；只为测试公开同文件常量，不增加生产导出。
test("插件宿主保留官方显隐、入口别名和未知入口兜底，插件槽不规定内容形状", async () => {
    const server = await createServer({
        server: { middlewareMode: true },
        appType: "custom",
        plugins: [
            {
                name: "plugin-host-test-tables",
                transform(code, id) {
                    if (id.endsWith("/features/components/OfficialComponentHost.tsx")) return `${code}\nexport { pluginVisibility, pluginRenderers };`;
                },
            },
        ],
    }); // 不启动监听端口，沿用生产入口及依赖。
    try {
        const { pluginVisibility, pluginRenderers } = await server.ssrLoadModule("/src/features/components/OfficialComponentHost.tsx"); // 真实判定与渲染函数。
        for (const value of [undefined, false, true]) {
            for (const debugRegistered of [false, true]) {
                const context = {
                    preferences: { clockEnabled: value, debugEnabled: value, musicEnabled: value, collectEnabled: value, weatherEnabled: value },
                    runtime: { manifests: debugRegistered ? [{ id: "official.debug-test" }] : [] },
                }; // 每个开关与 Debug 注册是独立条件。
                for (const [id, expected] of [
                    ["official.local-clock", value !== false],
                    ["official.debug-test", value !== false && debugRegistered],
                    ["official.music-control", value === true],
                    ["official.collect-box", value !== false],
                    ["official.weather", value === true],
                ])
                    assert.equal(pluginVisibility.get(id)(context), expected);
                assert.equal(pluginVisibility.get("plugin.sample")?.(context) ?? true, true);
            }
        }
        assert.equal(pluginVisibility.get("official.debug-test")({}), false);
        assert.equal(pluginVisibility.get("official.local-clock")({}), true);
        assert.equal(pluginVisibility.get("official.collect-box")({}), true);
        assert.equal(pluginVisibility.get("official.music-control")({}), false);
        assert.equal(pluginVisibility.get("official.weather")({}), false);

        const props = { instance: { instanceId: "sample", componentId: "plugin.sample" }, registration: { entry: "plugin:slot" } }; // 内容分派只传递原有实例契约。
        for (const name of ["local-clock", "debug-test", "music-control", "collect-box", "weather"]) {
            const render = pluginRenderers.get(`official:${name}`); // 官方清单使用冒号入口。
            assert.equal(render, pluginRenderers.get(`official.${name}`));
            const tile = render(props, { debugContent: "debug content" }); // 同一 renderer 接收原有 props。
            assert.equal(tile.props.instance, props.instance);
            assert.equal(tile.props.registration, props.registration);
            assert.equal(tile.key, "sample");
            if (name === "debug-test") assert.equal(tile.props.children, "debug content");
        }
        assert.equal(pluginRenderers.get("unknown:entry"), undefined);
        assert.equal(pluginRenderers.get("constructor"), undefined);
        const pluginTile = pluginRenderers.get("plugin:slot")(props, {}); // 插件只取得容器，没有新尺寸和命中策略。
        assert.equal(pluginTile.props.minimum, undefined);
        assert.equal(pluginTile.props["data-hit-priority"], undefined);
        assert.equal(pluginTile.props.children.props["data-plugin-container"], "sample");
        assert.equal(pluginTile.props.children.props.className, undefined);
        assert.equal(pluginTile.props.children.props.children, "插件内容尚未加载");

        const source = await readFile(new URL("./OfficialComponentHost.tsx", import.meta.url), "utf8"); // 保证入口优先级与原有兜底原样保留。
        assert.match(source, /props\.registration\?\.entry \?\? instance\.componentId/);
        assert.match(source, /title="本地时间" minimum=\{\{ width: 164, height: 90 \}\}/);
        assert.match(source, /<p className="official-component__error">组件前端入口不存在<\/p>/);
    } finally {
        await server.close();
    }
});
