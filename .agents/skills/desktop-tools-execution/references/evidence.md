# evidence：按层取证

先读 `docs/validation/验收流程.md` 的分层表；本页只给执行顺序与模板。

## 从低到高

| 层 | 命令 | 能证明 |
| --- | --- | --- |
| L0 单元/契约 | `node --test <受影响文件>`；提交前 `npm test` | 纯函数、契约、几何 |
| L1 组件/UI 数值 | `npm run dev` 后 `npm run test:browser` 或 `--页 <子串>` | 真实 React 组件 + 真实 CSS 的计算样式与布局 |
| L2 宿主原生 | `npm run test:rust` | Windows API 返回值、尺寸、命中区域 |
| L3-探针 | 开发宿主开 9223 后 `node scripts/取证探针.mjs --清单 <清单.mjs>` | 真实 WebView2 里的计算样式、计数、落盘偏好 |
| L3 真机 | `.\scripts\后台启动程序.ps1 -Theme <主题>` | 真实宿主日志、退出码、进程状态 |
| L4 性能 | 暂无脚本化入口 | 按待验证处理 |

能表达成返回值、尺寸、计数或计算样式的现象，必须在 L0–L2 或 L3-探针收口，不用截图。

## 八项只能人眼

设计稿还原、透明桌面与 DWM 合成、原生拖放手感、系统缩略图内容策略、真实 Toast 样式、多显示器 DPI 组合、音乐频谱柱形、多类型格子菜单落位。

这八项由用户实际操作后确认；结果写进 `build/validation/记录/`，标注「人眼确认」与时间。Agent 不写脚本反复采集同一视图来「核对还原度」。

## 探针清单

```js
// .scratch/<问题>/清单.mjs
export default {
    检查: [{ 名称: "主题行文案", 表达式: `document.querySelector("#interface-theme-plugin")?.value ?? null` }],
    截图: [{ 名称: "外观页主题行", 选择器: ".appearance-settings" }],
};
```

进宿主前把要核对的点一次列完，一轮跑完再读结果；不要「截一张—看一眼—再截一张」。

## 样式表缓存陷阱

WebView2 按 URL 缓存 CSS。文件已同步但 URL 未变时：

- `fetch(href, { cache: "no-store" })` 能拿到新文本；
- 计算样式仍可能来自旧 CSSOM。

核对顺序：先确认 `build/target/debug/plugin` 里的文件已更新、`link[data-theme-plugin]` 的 href 已变化，再读计算样式。两者不一致时先怀疑缓存与同步，不要反复改 CSS 试错。

## 禁止

- 无界重试、超时后重启碰运气。
- 用低层结论冒充高层（L0 通过 ≠ 宿主传参正确；L1 PASS ≠ 真机通过）。
- 把「准备完成」「未执行」「超时」写成通过。
- 把 L3 真机层并入 `npm test`。
