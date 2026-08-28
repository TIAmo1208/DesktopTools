# 前端边界

`src/` 负责 React 页面呈现、用户交互状态、展示模型转换和类型化 Tauri bridge。

`features/` 按产品功能组织页面与交互；`bridge/` 负责原生命令调用和事件订阅；`contracts/` 与 `src-tauri/src/contracts/` 对齐跨边界 DTO。领域规则和 Windows API 适配集中在 Rust 后端。
