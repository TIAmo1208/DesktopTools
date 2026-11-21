import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
    islandNavigationFailure,
    islandNavigationInstanceId,
    islandNavigationLabel,
    islandNavigationUri,
    openIslandNavigation,
} from "./islandNavigation.ts";

test("只把非空地址视为已声明导航，实例身份用岛合成实例", () => {
    assert.equal(islandNavigationUri("dsh://open"), "dsh://open");
    assert.equal(islandNavigationUri("  https://example.com/x  "), "https://example.com/x");
    for (const empty of [undefined, null, "", "   ", 1, true])
        assert.equal(islandNavigationUri(empty), undefined);
    assert.equal(islandNavigationUri("DSH://open"), "DSH://open"); // 协议前缀大小写不敏感，与宿主一致。
    for (const invalid of ["file:///C:/secret", "javascript:alert(1)", "http://example.com", "httpsx://example.com", "dsh:/\u0000bad", "dsh:/\u0085bad", `dsh:${"x".repeat(509)}`])
        assert.equal(islandNavigationUri(invalid), undefined); // 白名单/控制字符/超长在前端就被拒。
    assert.equal(islandNavigationInstanceId("plugin.dsh-status"), "island.plugin.dsh-status");
    assert.equal(islandNavigationLabel("DSH 接入", "dsh://open"), "DSH 接入：打开 dsh://open");
});

test("打开失败保留宿主原因，成功与空地址分别返回", async () => {
    const calls: string[] = [];
    const open = async (instanceId: string, uri: string) => void calls.push(`${instanceId}|${uri}`);
    assert.deepEqual(await openIslandNavigation(open, "plugin.dsh-status", "dsh://open"), { ok: true });
    assert.deepEqual(calls, ["island.plugin.dsh-status|dsh://open"]); // 带岛实例身份调用 open_uri。
    const rejected = await openIslandNavigation(async () => {
        throw "只允许打开 dsh: 或 https: 链接";
    }, "plugin.dsh-status", "dsh://open");
    assert.deepEqual(rejected, { ok: false, message: "无法打开链接：只允许打开 dsh: 或 https: 链接" }); // 宿主拒绝时原样提示。
    assert.deepEqual(await openIslandNavigation(open, "plugin.dsh-status", "   "), { ok: false, message: "无法打开链接：地址为空。" });
    const rejectedBeforeCommand = await openIslandNavigation(async () => {
        throw new Error("不应调用命令");
    }, "plugin.dsh-status", "file:///C:/secret");
    assert.deepEqual(rejectedBeforeCommand, { ok: false, message: "无法打开链接：地址为空。" }); // 非法地址不发命令。
    assert.equal(islandNavigationFailure(new Error("打开失败")), "无法打开链接：打开失败");
});

test("浮岛渲染只在声明了 uri 时启用点击/键盘，其它锚点与条目保持只读", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const navigable = islandNavigationUri\(anchor\.uri\) !== undefined && anchor\.componentId !== undefined/);
    assert.match(source, /role=\{navigable \? "button" : undefined\}/); // 只读项不改变语义。
    assert.match(source, /tabIndex=\{navigable \? 0 : undefined\}/);
    assert.match(source, /onKeyDown=\{navigable \? \(event\) => \{/); // Enter/Space 键盘激活。
    assert.match(source, /openNavigationTarget\(anchor\.componentId, anchor\.uri\)/);
    assert.match(source, /const navigable = islandNavigationUri\(notice\.uri\) !== undefined && notice\.componentId !== undefined/); // 消息条目按 uri 判定。
    assert.match(source, /onOpen\(notice\.componentId, notice\.uri\)/); // 消息条目按来源实例打开。
    assert.match(source, /onOpen=\{openNavigationTarget\}/);
    assert.match(source, /navigationFeedback \? <p className="island-navigation-feedback" role="status">/); // 页内提示，不弹 Toast。
    assert.match(source, /!measuring && navigationFeedback \? <p className="island-navigation-feedback" role="status">/); // hover 详情也有可见反馈。
    assert.match(source, /mode === "collapsed" && navigationFeedback \? ` \$\{navigationFeedback\}`/); // 胶囊只播报，不画文字。
    assert.match(source, /navigationRequest\.current \+= 1; \/\/ 转态即让旧请求失效/); // 转态清理迟到结果。
    assert.match(source, /event\.preventDefault\(\);[^}]*?event\.stopPropagation\(\);[^}]*?if \(event\.repeat\) return;/s); // 先阻断默认与冒泡，再对 repeat 早退。
    assert.doesNotMatch(source, /if \(event\.repeat\) return;[^}]*?event\.preventDefault\(\)/s); // 不允许 repeat 先漏给父级。
    assert.match(source, /preventDefault\(\); \/\/ 抢在父级 Enter 焦点导航之前/); // 可点锚点绕过父级 Enter。
});

test("hover 摘要行与消息正文复用单行溢出滚动，离开/减少动效不动", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /<ScrollingText text=\{body\.lines\[0\] \?\? ""\} title=\{body\.lines\[0\] \?\? ""\} enabled=\{!measuring && animationsEnabled\} \/>/); // 插件摘要行。
    assert.match(source, /<ScrollingText text=\{text\} title=\{text\} enabled=\{!measuring && animationsEnabled && notice\.key === active\} \/>/); // 只有当前条目滚动。
    assert.match(source, /onPointerEnter=\{\(\) => setActiveKey\(notice\.key\)\} onFocus=\{\(\) => setActiveKey\(notice\.key\)\}/); // 切换/聚焦换活动行。
    const css = readFileSync(new URL("../../shared/ScrollingText.css", import.meta.url), "utf8");
    assert.match(css, /max-inline-size: 100%;/); // 不超出容器。
    const component = readFileSync(new URL("../../shared/ScrollingText.tsx", import.meta.url), "utf8");
    assert.match(component, /animation\.current\?\.cancel\(\);/); // 禁用/变短后取消动画。
    assert.match(component, /matchMedia\("\(prefers-reduced-motion: reduce\)"\)/); // 减少动效不动。
});
