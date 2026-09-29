# Desktop Tools

Desktop Tools 是面向个人效率用户、专业创作者和开发者的 Windows 桌面内容工作区，负责呈现和组织桌面项目、围栏、文件夹门户与功能格子。Windows Explorer 继续提供桌面命名空间、Shell 操作和恢复兜底。

## 功能介绍

- 组织文件、目录、快捷方式和其他 Shell 桌面项目。
- 使用围栏管理项目归属、顺序、排序、滚动、折叠和布局。
- 使用文件夹门户浏览绑定目录及其子目录，并支持真实文件拖入和拖出。
- 提供收集箱、音乐、时间和 Debug 等功能格子。
- 收集箱接收文本、HTTP(S) 链接、本地图片和受支持的 PNG/DIB 图像数据；拖入时临时展开，投到条目则追加，其他位置则新建。图片文件的复制、移动、仅链接设置位于收集箱二级设置；无原始路径的图像数据统一写入资源目录。
- 音乐格子接收单个受支持的媒体文件并交给系统默认程序打开，不保证自动播放；时钟与 Debug 拒绝内容投放。各类型白名单与格式限制见 [拖放策略](docs/adr/0017-分离系统拖放与布局投放结果.md)。
- 收集箱的设置页、桌面背景菜单和格子菜单共用创建流程，统一确定初始位置与展开尺寸。
- 收集箱格子菜单提供“移除”，只移除当前格子并保留共享收集内容；本次会话可撤销或重做，移除全部格子后仍可从设置页或桌面背景菜单重新添加。
- 统一功能格子的移动、尺寸调整、边缘吸附、折叠、标题工具和跨屏交互。
- 桌面项目拖动统一使用 Windows OLE；功能格子移动和尺寸调整使用工作区内部交互。
- 通过 Windows Shell 执行打开、重命名、删除、复制、移动、剪贴板和原生上下文菜单操作。
- 支持多显示器、DPI 坐标转换、跨屏预览和布局恢复。

桌面项目布局、围栏归属、门户配置和功能格子状态由 Desktop Tools 维护；文件事实和 Shell 恢复仍由 Windows Explorer 提供。

## 构建与运行

常用验证命令：

```text
npm run typecheck
npm run test:frontend
npm run test:rust
npm run build
npm run tauri:build
```

发布构建使用 `npm run tauri:build`。直接执行 Cargo 时沿用项目指定工具链、共享构建目录和 `src-tauri/Cargo.toml` 的 features。

Windows x64 未签名测试安装包入口为 `npm run tauri:installer`，产物目录为 `build/target/release/bundle/nsis/`。NSIS 使用当前用户安装，缺少 WebView2 时联网安装，安装包包含主程序、guardian 和许可声明；升级与卸载钩子请求正常退出并保留用户数据。实际打包及中文空格路径下的首次安装、同版本覆盖和卸载已验证；运行中恢复等剩余边界见[安装包验证](docs/validation/安装包验证.md)。此入口不执行签名或 GitHub 发布，发行约束见 [ADR-0024](docs/adr/0024-采用当前用户NSIS安装包.md)。

## 文档入口

- [前端目录结构](src/README.md)
- [Rust/Tauri 目录结构](src-tauri/README.md)
- [领域词汇](CONTEXT.md)
- [正式架构规格](docs/adr/0016-正式项目架构.md)
- [共享格子外观与公共交互决策](docs/adr/0018-共享格子外观并隔离布局撤销.md)
- [视觉规范](docs/design/视觉规范.md)

## 使用边界

- 关闭功能格子只隐藏实例并保留布局；折叠只隐藏内容，临时展开不修改保存的外观。
- 移动与缩放统一经公共外框处理，活动几何的 Escape 优先取消；开始提交后的领域操作沿用既有撤销边界。
- 多屏、混合 DPI、真实 Shell 文件投放及持续拖动性能需在 Windows 宿主验收，自动检查不能替代实机结果。
- 源码目录及入口分别见[前端目录结构](src/README.md)和[Rust/Tauri 目录结构](src-tauri/README.md)，实现与验证范围见[公共格子分层验收](docs/validation/公共格子分层验收.md)。
