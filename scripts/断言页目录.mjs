// 真实组件浏览器断言页（相对仓库根）。

// 断言页：页面把结果写进 <pre id="result">，以 PASS 开头为通过、FAIL 开头为失败。
export const 断言页 = [
    "src/shared/scrollingText.browser.html", // 单行文本溢出滚动：判定、动画名与时长、重新测量与关闭动效。
    "src/features/desktop-items/desktopItemImage.browser.html", // 桌面项图片缩略与等待状态。
    "src/features/settings/weatherSettings.browser.html", // 天气设置完整交互链路。
    "src/features/tile-content/collect-box/collectBox.browser.html", // 收集箱加载、选择、删除与投放。
    "src/features/tiles/contentDrop.browser.html", // 格子内容拖放。
    "src/features/tiles/tileDragPreview.browser.html", // 跨屏格子拖动预览与围栏尺寸。
    "src/features/island/islandFillet.browser.html", // 浮岛切态圆弧补面与描边。
    "src/features/island/islandLyrics.browser.html", // 浮岛歌词常态档位、无词回落、四边与展开跳转对齐。
    "src/features/island/islandMessages.browser.html", // 浮岛消息三态、关闭、阅读期暂存与通道降级。
    "src/features/island/islandTheme.browser.html", // 浮岛围栏令牌复制、主题替换/深浅切换与主题表现层隔离。
    "src/features/tile-content/music/musicLyrics.browser.html", // 桌面音乐格子换句位移、字号颜色过渡与减少动效。
    "src/features/popup-layer/popupMenu.browser.html", // 弹层菜单左右贴边、宽窄子菜单与窗口盒稳定性。
];

// 真机专用页：判定依赖真实执行环境（WebView2 渲染时序、系统 Shell 等），headless Edge 下不能成立。
// 运行器对这里的页面输出 SKIP（真机专用），不计失败；证据由 L3 真机取证的日志与用户截图覆盖。
export const 真机专用页 = [
    "src/features/tile-content/collect-box/collectEditorImages.browser.html", // 「已卸载图片延迟写 srcset」依赖 WebView2 真实渲染时序，headless 下等待超时，改由真机取证。
];

// 检查页：人工核对用的样式/菜单样张，没有 PASS/FAIL 判定，只要求能在浏览器里加载并渲染。
// 渲染判据排除 vite-error-overlay：出现构建或运行错误浮层即判失败，不以浮层元素充当渲染内容。
export const 检查页 = [
    "src/features/settings/pluginSettings.browser.html", // 插件设置页样式核对。
    "src/features/settings/pluginTaskMenu.browser.html", // 插件任务菜单各状态样张。
];
