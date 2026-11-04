import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

// 使用现有 Vite 转换 TSX，不引入额外的组件测试工具。
test("settings switches submit independently of submenu navigation", async () => {
    const server = await createServer({ server: { middlewareMode: true }, appType: "custom" }); // 仅加载组件，不启动监听端口。
    try {
        const { SettingsSwitch } = await server.ssrLoadModule("/src/features/settings/SettingsSwitch.tsx"); // 共用开关的真实实现。
        const changes: boolean[] = []; // 记录用户开关意图。
        const props = { label: "本地时间", checked: true, disabled: false, onChange: (checked: boolean) => changes.push(checked) }; // 已启用状态。
        const control = SettingsSwitch(props); // 无内部状态的原生控件树。
        const input = control.props.children[1]; // 独立于导航按钮的输入节点。
        input.props.onChange({ currentTarget: { checked: false } });
        assert.deepEqual(changes, [false]);
        assert.equal(input.props.role, "switch");
        assert.equal(input.props.checked, true);
        assert.equal(SettingsSwitch({ ...props, disabled: true }).props.children[1].props.disabled, true);

        const { FeatureSettings } = await server.ssrLoadModule("/src/features/settings/FeatureSettings.tsx"); // 二级菜单入口。
        const markup = renderToStaticMarkup(createElement(FeatureSettings, { disabled: false, debugBuild: true, onPreferences: () => {} })); // 未加载配置的禁用入口。
        assert.equal((markup.match(/role="switch"/g) ?? []).length, 6); // 5 个常驻功能加 Debug 测试。
        assert.equal((markup.match(/class="feature-settings__hit"/g) ?? []).length, 6); // 整行命中层是独立按钮。
        assert.equal((markup.match(/data-button="row-hit"/g) ?? []).length, 6); // 结构件带语义标记，主题不参与。
        assert.equal((markup.match(/data-button="switch"/g) ?? []).length, 6);
        assert.doesNotMatch(markup, /<button[^>]*>[^<]*<input/);
        assert.match(markup, /本地时间<\/span><label class="settings-switch">/);
        assert.match(markup, /Debug 测试<\/span><label class="settings-switch">/);

        // 名称与箭头压在命中层之上，必须让指针穿透，否则点名称不进二级页。
        const css = readFileSync(new URL("./FeatureSettings.css", import.meta.url), "utf8"); // 一级列表行的真实样式。
        const passthrough = css.split("}").find((rule) => rule.includes("pointer-events: none")) ?? ""; // 带穿透声明的规则块。
        assert.match(passthrough, /__copy/);
        assert.match(passthrough, /__chevron/);
    } finally {
        await server.close();
    }
});
