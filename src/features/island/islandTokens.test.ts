// 浮岛令牌与主题表现层的契约回归：令牌集合必须跟着桌面围栏走，主题装饰必须避开浮岛。
// 证据层为 L0（静态文本断言）；浮岛页面上的实际取值与表现层隔离由 islandTheme.browser.html 覆盖。
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { ISLAND_FENCE_TOKENS } from "./islandTokens.ts";

/** 读取相对本测试文件的文本文件。 */
const 读取 = (相对路径: string) => readFileSync(fileURLToPath(new URL(相对路径, import.meta.url)), "utf8");

/** 收集 CSS 里声明的自定义属性名（只看声明，不看 var() 引用）。 */
const 声明的令牌 = (css: string) => [...css.matchAll(/(--[a-z-]+)\s*:/g)].map((匹配) => 匹配[1]!);

test("浮岛复制的围栏令牌与桌面围栏声明一致", () => {
    const 桌面围栏 = 声明的令牌(读取("../../styles/fenceTokens.css")).filter((名称) => 名称.startsWith("--fence-"));
    assert.ok(桌面围栏.length > 0, "围栏令牌样式表应声明 --fence-* 令牌");
    assert.deepEqual([...ISLAND_FENCE_TOKENS].sort(), [...new Set(桌面围栏)].sort(), "浮岛复制组必须覆盖围栏声明的全部令牌，避免插件卡在浮岛上掉色");
});

test("浮岛样式表不自带围栏令牌取值", () => {
    const 浮岛令牌 = 声明的令牌(读取("./island.css")).filter((名称) => 名称.startsWith("--fence-"));
    assert.deepEqual(浮岛令牌, [], "浮岛令牌只允许来自主题样式表与偏好写入两处来源，样式表不能长出第三份回落值");
});

test("直接加载工作区布局的断言页同时加载围栏令牌", () => {
    // DesktopWorkspace.tsx 先加载 fenceTokens.css 再加载自身样式表；直接 import CSS 的夹具必须保持同一顺序。
    const 源目录 = fileURLToPath(new URL("../..", import.meta.url));
    const 页面 = readdirSync(源目录, { recursive: true, withFileTypes: true })
        .filter((条目) => 条目.isFile() && 条目.name.endsWith(".browser.html"))
        .map((条目) => join(条目.parentPath, 条目.name))
        .filter((路径) => readFileSync(路径, "utf8").includes("DesktopWorkspace.css"));
    assert.ok(页面.length >= 3, "应能找到直接加载工作区布局的断言页");
    for (const 路径 of 页面) {
        assert.match(readFileSync(路径, "utf8"), /fenceTokens\.css/, `${relative(源目录, 路径)} 直接加载 DesktopWorkspace.css 时必须同时加载 styles/fenceTokens.css`);
    }
});

test("主题表现层必须排除浮岛", () => {
    const 插件目录 = fileURLToPath(new URL("../../../plugin/", import.meta.url));
    const 主题目录 = readdirSync(插件目录, { withFileTypes: true })
        .filter((条目) => 条目.isDirectory() && 条目.name.startsWith("主题-"))
        .map((条目) => `../../../plugin/${条目.name}/theme.css`);
    assert.ok(主题目录.length > 0, "应能在 plugin/ 下找到主题插件");
    // 控件元素按标签名匹配，避免把 data-selected 之类的属性值当成 select 元素；标题同属装饰性排版。
    const 控件元素 = /(^|[\s>+~])(button|input|select|textarea|h1)(?=$|[\s,:.\[{>+~])/;
    /** 该条选择器是否会作用到浮岛：全屏层、直接写浮岛的选择器，以及没有类或属性前缀的全局写法。 */
    const 命中浮岛 = (部分: string) => {
        if (/#root/.test(部分) || /\.island\b|\[data-island/.test(部分)) return true;
        if (!控件元素.test(部分)) return false;
        return !/^[.#\[]/.test(部分); // `.settings-switch input` 这类组件作用域规则不会命中浮岛。
    };
    for (const 路径 of 主题目录) {
        const css = 读取(路径).replace(/\/\*[\s\S]*?\*\//g, ""); // 注释不参与选择器判定。
        // 选择器文本取到最近的 { 之前；@media 里的规则同样命中，因为分块后其前导只有自身选择器。
        for (const [, 选择器] of css.matchAll(/([^{}]+)\{/g)) {
            for (const 部分 of 选择器.split(",")) {
                if (!命中浮岛(部分.trim())) continue;
                assert.match(
                    部分,
                    /:where\(html:not\(\[data-island="true"\]\)\)/,
                    `${路径} 的浮岛可见选择器缺少 :where(html:not([data-island="true"])) 门控（零权重包装，不抬高原特异性）：${部分.trim()}`,
                );
            }
        }
    }
});
