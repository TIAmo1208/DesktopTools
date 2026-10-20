import assert from "node:assert/strict";
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
        assert.equal((markup.match(/class="feature-settings__open"/g) ?? []).length, 6);
        assert.doesNotMatch(markup, /<button[^>]*>[^<]*<input/);
        assert.match(markup, /本地时间<\/button><label class="settings-switch">/);
        assert.match(markup, /Debug 测试<\/button><label class="settings-switch">/);
    } finally {
        await server.close();
    }
});
