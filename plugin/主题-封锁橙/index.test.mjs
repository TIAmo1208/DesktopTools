// 封锁橙主题的纯逻辑与样式表契约：开关归一、切角量夹紧、令牌只取契约内名称、焦点语言与浮岛门控。
// 证据层为 L0（静态文本断言）；浮岛与窗口上的实际取值由 src/features/island/islandTheme.browser.html 与真机验收覆盖。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { applyThemeOptions, clearThemeOptions, normalizeBevelPixels, normalizeOptions } from "./index.js";

const 读取 = (相对路径) => readFileSync(new URL(相对路径, import.meta.url), "utf8");
const 主题样式表 = () => 读取("./theme.css");
const 主题数据 = () => JSON.parse(读取("./theme.json"));
/** 去掉注释后逐块取出选择器与声明，供样式断言使用。 */
const 规则表 = (css) => [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, 选择器, 内容]) => ({ 选择器: 选择器.replace(/\s+/g, " ").trim(), 内容 }));

test("特效开关默认全开，只有显式 false 才关闭", () => {
    assert.deepEqual(normalizeOptions(null), { bevel: true, mono: true });
    assert.deepEqual(normalizeOptions({ bevel: false }), { bevel: false, mono: true }); // 缺键按默认开启。
    assert.deepEqual(normalizeOptions({ bevel: "off", mono: 0 }), { bevel: true, mono: true }); // 非布尔值不当作关闭。
    assert.deepEqual(normalizeOptions([{ bevel: false }]), { bevel: true, mono: true }); // 数组不是设置对象。
});

test("切角像素非法回落、合法值夹紧且允许零", () => {
    assert.equal(normalizeBevelPixels(undefined), 8);
    assert.equal(normalizeBevelPixels("8"), 8); // 字符串不参与归一。
    assert.equal(normalizeBevelPixels(Number.NaN), 8);
    assert.equal(normalizeBevelPixels(-4), 0);
    assert.equal(normalizeBevelPixels(0), 0);
    assert.equal(normalizeBevelPixels(5.5), 5.5);
    assert.equal(normalizeBevelPixels(20), 12);
});

test("开关与切角变量写入文档根，清理后不留残留", () => {
    const properties = new Map();
    const root = {
        dataset: {},
        style: {
            setProperty: (name, value) => properties.set(name, value),
            removeProperty: (name) => properties.delete(name),
        },
    };
    applyThemeOptions(root, { bevel: false, bevelPixels: 5 });
    assert.deepEqual(root.dataset, { blockadeBevel: "off", blockadeMono: "on" });
    assert.equal(properties.get("--blockade-bevel"), "5px");
    clearThemeOptions(root);
    assert.deepEqual(root.dataset, {}, "停用主题后不能留下 data-blockade-* 状态");
    assert.equal(properties.has("--blockade-bevel"), false, "停用主题后不能留下切角变量");
});

test("外观四项与原生消息外观写在 theme.json", () => {
    const theme = 主题数据();
    assert.deepEqual(theme.appearance, {
        accentColor: "#ff7f00",
        desktopAccentColor: "#ffb03a",
        cornerRadius: 4, // 设计语言的面板圆角 4px。
        fontFamily: "Microsoft YaHei UI",
        fontSize: 13,
    });
    assert.equal(theme.notification.cornerStyle, "bevel");
    assert.equal(theme.notification.cornerSize, 10);
    assert.equal(theme.notification.weight, "medium");
    assert.doesNotMatch(JSON.stringify(theme), /boot/, "本主题不覆盖启动覆盖层时长，沿用内置 900ms");
});

test("样式表只声明契约内的配色令牌", () => {
    const css = 主题样式表();
    // 只认行首声明，避免把 `.settings__row--link:hover` 之类的 BEM 修饰名当成令牌。
    const 声明 = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/^\s*(--[a-z-]+)\s*:/gm)].map(([, 名称]) => 名称);
    const 允许 = {
        界面: ["--color-background", "--color-surface", "--color-subtle", "--color-border", "--color-text", "--color-muted", "--color-ready", "--color-active", "--color-action", "--color-action-hover", "--color-action-text", "--color-error", "--color-error-surface"],
        围栏: ["--fence-border", "--fence-surface", "--fence-text", "--fence-active", "--fence-warning", "--fence-error", "--fence-muted", "--fence-focus-border", "--fence-item-text-shadow"],
        菜单: ["--menu-border", "--menu-text", "--menu-surface", "--menu-hover", "--menu-shadow"],
    };
    for (const 名称 of 声明) {
        if (名称.startsWith("--blockade-")) continue; // 主题私有表现变量不属于契约。
        const 分组 = 名称.startsWith("--fence-") ? 允许.围栏 : 名称.startsWith("--menu-") ? 允许.菜单 : 允许.界面;
        assert.ok(分组.includes(名称), `${名称} 不在主题变量契约的可覆盖清单内`);
    }
    // 契约里的 13 + 9 + 5 个令牌都必须给出值。
    assert.deepEqual(new Set(声明.filter((名称) => !名称.startsWith("--blockade-"))).size, 27, "三组令牌要各自给全，禁止跨组借用");
});

test("深浅两套调色板与围栏、菜单分组各自独立", () => {
    const css = 主题样式表().replace(/\/\*[\s\S]*?\*\//g, ""); // 注释不参与选择器判定。
    const 单块 = (选择器) => 规则表(css).find((项) => 项.选择器 === 选择器)?.内容 ?? "";
    const 含 = (片段) => 规则表(css).filter((项) => 项.选择器.includes(片段)).map((项) => 项.内容).join("\n");
    const 深色界面 = 单块(':root[data-theme="dark"]');
    assert.match(深色界面, /--color-background:\s*#0d0e10/);
    assert.match(深色界面, /--color-action:\s*#ff7f00/);
    assert.match(深色界面, /--color-action-hover:\s*#f0a043/); // 设计语言的强调悬浮值。
    assert.match(深色界面, /--color-action-text:\s*#14181b/, "琥珀橙填充上的命令文字必须用深墨，白字对比度不足");
    assert.match(深色界面, /color-scheme:\s*dark/);
    const 浅色界面 = 单块(":root");
    assert.match(浅色界面, /--color-background:\s*#e6e2db/, "浅色是暖中性纸");
    assert.match(浅色界面, /--color-action:\s*#b25100/, "浅色强调必须压深到烧焦橙");
    assert.match(浅色界面, /--color-action-text:\s*#ffffff/);
    assert.match(单块(':root[data-theme="light"]'), /color-scheme:\s*light/);
    const 深色围栏 = 含(':root[data-desktop-theme="dark"] .fence-workspace');
    assert.match(深色围栏, /--fence-surface:\s*rgb\(27 28 32 \/ 52%\)/);
    assert.match(深色围栏, /--fence-item-text-shadow:\s*0 1px 2px/);
    const 浅色围栏 = 含(':root[data-desktop-theme="light"] .fence-workspace');
    assert.match(浅色围栏, /--fence-surface:\s*rgb\(252 251 248 \/ 86%\)/, "浅色主面不透明度不得低于设计语言下限");
    assert.match(浅色围栏, /--fence-item-text-shadow:\s*none/);
    assert.match(含(':root[data-theme="light"] .desktop-tile-menu'), /--menu-surface:\s*rgb\(252 251 248 \/ 94%\)/);
});

test("克制版不含全屏叠加层与常驻动画", () => {
    const css = 主题样式表();
    assert.doesNotMatch(css, /#root::after|#root::before/, "本主题不绘制全屏装饰层，避免浮岛与低配 GPU 负担");
    assert.doesNotMatch(css, /@keyframes/, "本主题不含循环动画");
    assert.doesNotMatch(css, /background-position[^;]*animation|animation[^;]*background-position/, "不允许 background-position 动画");
    assert.doesNotMatch(css, /backdrop-filter:\s*blur/, "模糊由宿主围栏持有，主题不加");
});

test("焦点语言是一条行顶 2px 橙条，没有发光文字", () => {
    const css = 主题样式表();
    const 规则 = 规则表(css);
    /** 断言某载体的伪元素条：2px 高、从左侧 scaleX 画出、只动 transform 与 opacity。 */
    const 断言条伪元素 = (选择器, 说明) => {
        const 条 = 规则.find((项) => 项.选择器 === 选择器);
        assert.ok(条, `${说明}需要 ::before 橙条`);
        assert.match(条.内容, /height:\s*2px/);
        assert.match(条.内容, /background:\s*var\(--fence-focus-border, var\(--color-focus\)\)/);
        assert.match(条.内容, /transform:\s*scaleX\(0\)/);
        assert.match(条.内容, /transform-origin:\s*left center/);
        assert.match(
            条.内容,
            /transition:\s*transform var\(--blockade-bar-duration, 200ms\) cubic-bezier\(0\.22, 0\.88, 0\.24, 1\), opacity var\(--blockade-bar-fade, 120ms\) ease/,
        );
    };
    // 四个静态条载体都改为伪元素描画：样式表里不再有 background-image 条。
    assert.doesNotMatch(css, /background-size:\s*100% 2px/, "条不得再用 background-image 静态写入");
    断言条伪元素(".desktop-tile::before", "桌面格子焦点");
    断言条伪元素(".desktop-item::before", "桌面项目");
    断言条伪元素(".folder-portal__items li::before", "门户条目");
    断言条伪元素(".official-component__collect li::before", "收集箱条目");
    // 状态规则把条画到最终态。
    const 状态 = (选择器) => 规则.find((项) => 项.选择器 === 选择器)?.内容 ?? "";
    assert.match(状态('.desktop-tile:focus-visible::before, .desktop-tile[data-keyboard-focus="true"]::before'), /transform:\s*scaleX\(1\)/);
    assert.match(状态('.desktop-item:hover::before, .desktop-item[data-selected="true"]::before'), /transform:\s*scaleX\(1\)/);
    assert.match(状态(".folder-portal__items li:hover::before, .folder-portal__items li.is-selected::before"), /transform:\s*scaleX\(1\)/);
    assert.match(状态(".official-component__collect li:hover::before, .official-component__collect .collect-box__item--selected::before"), /transform:\s*scaleX\(1\)/);
    // 定位上下文：桌面项目排除已绝对定位的变体，收集箱与门户 li 补 relative。
    assert.match(状态(".desktop-item:not(.desktop-item--unclassified):not(.desktop-item--drag-preview)"), /position:\s*relative/);
    assert.doesNotMatch(状态(".desktop-item"), /position:\s*relative/, "基础类不能无条件加 relative，会覆盖未归类项的绝对定位");
    assert.match(状态(".official-component__collect li"), /position:\s*relative/);
    assert.match(状态(".folder-portal__items li"), /position:\s*relative/);
    // 行底保持中性，橙色只出现在条上。
    const 选中项目 = 规则.find((项) => 项.选择器 === '.desktop-item[data-selected="true"]');
    assert.ok(选中项目, "桌面项目选中规则存在");
    assert.match(选中项目.内容, /background-color:\s*color-mix\(in srgb, var\(--fence-text\) 12%, transparent\)/);
    assert.doesNotMatch(选中项目.内容, /background-image|background-size/, "选中不再写背景图层，条归伪元素");
    assert.doesNotMatch(选中项目.内容, /border-color:\s*transparent/, "选中不再用橙色边框，条是唯一橙色信号");
    const 作用域选中 = 规则.find((项) => 项.选择器 === ':root[data-desktop-theme] .desktop-item[data-selected="true"]');
    assert.ok(作用域选中, "桌面主题作用域下的选中覆盖必须存在");
    assert.match(作用域选中.内容, /border-color:\s*var\(--fence-border\)/);
    assert.doesNotMatch(作用域选中.内容, /background-color:[^;]*color-focus/, "浅色桌面下选中也不得用橙色行底");
    // 菜单命令：橙条由伪元素从左侧画出，动效只动 transform 与 opacity。
    const 菜单条 = 规则.find((项) => 项.选择器.startsWith(".desktop-tile-menu button::before,"));
    assert.ok(菜单条, "菜单命令需要橙条伪元素");
    assert.match(菜单条.内容, /height:\s*2px/);
    assert.match(菜单条.内容, /transform:\s*scaleX\(0\)/);
    assert.match(菜单条.内容, /transition:\s*transform var\(--blockade-bar-duration, 200ms\) cubic-bezier\(0\.22, 0\.88, 0\.24, 1\)/);
    // 设置导航：当前页与悬停都用行顶条，不再是左侧竖条。
    const 导航当前 = 规则.find((项) => 项.选择器 === '.settings__navigation-item[aria-current="page"]::before');
    assert.ok(导航当前, "设置导航当前页标记存在");
    assert.match(导航当前.内容, /top:\s*0/);
    assert.match(导航当前.内容, /height:\s*2px/);
    assert.match(导航当前.内容, /width:\s*auto/);
    assert.doesNotMatch(导航当前.内容, /width:\s*3px|bottom:\s*13px/);
    // 桌面格子焦点：行顶条落在格子内部，不再画外扩整圈；底座强调色边框要回到中性发丝。
    assert.doesNotMatch(css, /outline:\s*2px/, "主题不再使用外扩 2px 焦点环");
    assert.match(
        状态('.desktop-tile:focus-visible, .desktop-tile[data-keyboard-focus="true"]'),
        /border-color:\s*var\(--fence-border\)/,
        "焦点态必须覆盖底座的强调色边框",
    );
    // 收集箱条目：与桌面项目、门户条目同一套条语言，不用橙色整边框。
    const 收集选中 = 规则.find((项) => 项.选择器 === ".official-component__collect .collect-box__item--selected");
    assert.ok(收集选中, "收集箱选中条目需要覆盖，否则退回组件自带的橙色整边框");
    assert.match(收集选中.内容, /border-color:\s*var\(--fence-border\)/);
    assert.doesNotMatch(收集选中.内容, /background-image|background-size/, "选中不再写背景图层，条归伪元素");
    assert.doesNotMatch(收集选中.内容, /color-accent/, "选中行底不得回退到界面强调色");
    assert.ok(
        规则.find((项) => 项.选择器 === ".official-component__collect .collect-box__item--selected:hover"),
        "选中项悬停要单独覆盖，保持选中行底",
    );
    const 收集组件样式 = 读取("../../src/features/tile-content/collect-box/CollectBox.css");
    assert.match(收集组件样式, /\.official-component__collect \.collect-box__item--selected\s*\{/, "组件选择器改名时主题覆盖要同步");
    assert.match(收集组件样式, /\.official-component__collect li:hover\s*\{/, "组件悬停选择器改名时主题覆盖要同步");
    // 反模式：发光文字。
    assert.doesNotMatch(css, /text-shadow:\s*0 0 \d+px/, "设计语言把发光文字列为反模式");
    assert.doesNotMatch(css, /data-blockade-glow/);
});

test("材质、四角刻线与输入卡聚焦按设计语言取值", () => {
    const css = 主题样式表();
    const 规则 = 规则表(css);
    const 深色格子 = 规则.find((项) => 项.选择器 === ".desktop-tile");
    assert.match(深色格子?.内容 ?? "", /inset 0 1px 0 rgb\(255 255 255 \/ 10%\)/);
    assert.match(深色格子?.内容 ?? "", /inset 0 -1px 0 rgb\(0 0 0 \/ 35%\)/);
    assert.match(深色格子?.内容 ?? "", /4px 10px 26px rgb\(0 0 0 \/ 40%\)/);
    const 浅色格子 = 规则.find((项) => 项.选择器 === ':root[data-desktop-theme="light"] .desktop-tile');
    assert.match(浅色格子?.内容 ?? "", /3px 8px 22px rgb\(28 26 22 \/ 12%\)/);
    // 四角刻线：中性灰发丝、内缩 6px、长 18px，不用橙色。
    const 刻线 = 规则.find((项) => 项.选择器 === ".desktop-tile::after");
    assert.ok(刻线, "格子需要四角刻线");
    assert.equal((刻线.内容.match(/linear-gradient\(var\(--fence-border\)/g) ?? []).length, 8);
    assert.match(刻线.内容, /inset:\s*6px/);
    assert.match(刻线.内容, /background-size:\s*18px 1px, 1px 18px/);
    assert.doesNotMatch(刻线.内容, /focus-border|desktop-accent/);
    const 菜单 = 规则.find((项) => 项.选择器.startsWith(".desktop-tile-menu,"));
    assert.ok(菜单, "菜单表面规则存在");
    assert.match(菜单.内容, /background-image:/);
    assert.match(菜单.内容, /calc\(100% - 6px\)/);
    assert.doesNotMatch(菜单.内容, /padding:|min-height:|margin:/, "菜单内距与行高归组件所有");
    // 输入卡聚焦：0.5px 描边 + 20px 淡光。
    const 输入聚焦 = 规则.find((项) => 项.选择器.startsWith(':where(html:not([data-island="true"])) input:not([type="checkbox"]):not([type="radio"]):focus-visible,'));
    assert.match(输入聚焦?.内容 ?? "", /box-shadow:\s*0 0 0 0\.5px var\(--color-focus\), 0 0 20px color-mix\(in srgb, var\(--color-focus\) 14%, transparent\)/);
});

test("切角只落左上与右下，关闭后回到直角，菜单命令保持 2px 圆角", () => {
    const css = 主题样式表().replace(/\/\*[\s\S]*?\*\//g, "");
    const 规则 = 规则表(css);
    const 切角 = 规则.find(({ 选择器 }) => 选择器.startsWith(':where(html:not([data-island="true"])) button:not([data-button="row-hit"]),'));
    const 关闭 = 规则.find(({ 选择器 }) => 选择器.startsWith(':where(html:not([data-island="true"])):root[data-blockade-bevel="off"] button:not([data-button="row-hit"]),'));
    assert.ok(切角 && 关闭, "切角与关闭态两条规则都应存在");
    assert.match(切角.内容, /border-start-start-radius:\s*var\(--blockade-bevel,\s*8px\)/);
    assert.match(切角.内容, /border-start-end-radius:\s*0/);
    assert.match(切角.内容, /border-end-start-radius:\s*0/);
    assert.match(切角.内容, /border-end-end-radius:\s*var\(--blockade-bevel,\s*8px\)/);
    assert.match(切角.内容, /corner-top-left-shape:\s*bevel/);
    assert.match(切角.内容, /corner-bottom-right-shape:\s*bevel/);
    assert.doesNotMatch(切角.内容, /corner-top-right-shape:\s*bevel|corner-bottom-left-shape:\s*bevel/);
    assert.doesNotMatch(切角.选择器, /row-hit"\]\s*\,|Toast|#root/);
    assert.match(关闭.内容, /border-radius:\s*0/);
    assert.match(关闭.内容, /corner-shape:\s*round/);
    assert.match(css, /\.desktop-tile-menu button,\s*\.folder-portal__breadcrumb-menu button\s*\{[^}]*border-radius:\s*2px;[^}]*corner-shape:\s*round;/s);
});

test("主题表现层的全局选择器都带浮岛门控", () => {
    const css = 主题样式表().replace(/\/\*[\s\S]*?\*\//g, "");
    const 控件元素 = /(^|[\s>+~])(button|input|select|textarea|h1)(?=$|[\s,:.\[{>+~])/;
    for (const [, 选择器] of css.matchAll(/([^{}]+)\{/g)) {
        for (const 部分 of 选择器.split(",")) {
            const 文本 = 部分.trim();
            if (!控件元素.test(文本) || /^[.#\[]/.test(文本)) continue; // 组件作用域规则不命中浮岛。
            assert.match(文本, /:where\(html:not\(\[data-island="true"\]\)\)/, `${文本} 缺少浮岛门控`);
        }
    }
    assert.match(css, /:where\(html:not\(\[data-island="true"\]\)\):root\[data-blockade-mono="off"\]/, "等宽数字关闭态必须与门控直接复合");
    assert.match(css, /:where\(html:not\(\[data-island="true"\]\)\) h1\s*\{/, "标题字距必须带浮岛门控");
    // 条动画的时长在动画开关与减少动效下归零，只保留最终态。
    assert.match(css, /:root\[data-interface-animations="off"\]\s*\{[^}]*--blockade-bar-duration:\s*0ms;[^}]*--blockade-bar-fade:\s*0ms;/s);
    assert.match(css, /@media \(prefers-reduced-motion: reduce\)\s*\{[^}]*--blockade-bar-duration:\s*0ms;[^}]*--blockade-bar-fade:\s*0ms;/s);
});

test("减少透明效果时撤下围栏模糊", () => {
    const css = 主题样式表();
    assert.match(css, /@media \(prefers-reduced-transparency: reduce\)\s*\{[^}]*\.desktop-tile\s*\{[^}]*background-color:\s*var\(--color-surface\);[^}]*backdrop-filter:\s*none;/s);
});
