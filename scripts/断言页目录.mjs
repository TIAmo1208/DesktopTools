// 真实组件浏览器断言页（相对仓库根）。

// 断言页：页面把结果写进 <pre id="result">，以 PASS 开头为通过、FAIL 开头为失败。
export const 断言页 = [
    "src/features/desktop-items/desktopItemImage.browser.html", // 桌面项图片缩略与等待状态。
    "src/features/settings/weatherSettings.browser.html", // 天气设置完整交互链路。
    "src/features/tile-content/collect-box/collectBox.browser.html", // 收集箱加载、选择、删除与投放。
    "src/features/tiles/contentDrop.browser.html", // 格子内容拖放。
    "src/features/tiles/tileDragPreview.browser.html", // 跨屏格子拖动预览与围栏尺寸。
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
    "src/features/settings/settingsSamples.browser.html", // 设置页骨架、各类设置行与控件状态样张。
];
