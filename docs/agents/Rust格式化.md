# Rust 格式化

正式 Rust 工程使用 `src-tauri/rustfmt.toml`，由固定版本 `nightly-2026-09-08` 的 rustfmt 执行。编译、测试与发布继续使用原有 stable 工具链，不设置项目级 nightly override。

```powershell
rustup toolchain install nightly-2026-09-08 --profile minimal --component rustfmt
npm run format:rust
npm run format:rust:check
```

`npm run format` 同时执行原有前端格式化和固定版本 Rust 格式化。不要使用未指定工具链的 `cargo fmt`，stable rustfmt 会忽略 Allman 等不稳定配置。

## 风格映射

| C++ 参考风格 | Rust 规则 |
| --- | --- |
| Allman | 声明、函数及控制流大括号另起一行；允许短函数单行 |
| 4 空格、禁用 Tab | `tab_spaces = 4`、`hard_tabs = false` |
| 连续空行最多一行 | `blank_lines_upper_bound = 1` |
| 不限行宽 | 无完全等价配置；`max_width = 1000` 近似，超长行仍会折行 |
| 短条件单行 | 保留 rustfmt 支持的短 if/else、let/else；独立 if 提前返回不保证单行 |
| 连续声明、赋值不对齐 | 不启用结构字段和枚举判别值对齐 |
| 二元运算符放在续行开头 | `binop_separator = "Front"` |
| 尾注释对齐 | rustfmt 没有 clang-format 的通用等价配置，以其原生排版为准 |

Rust 的引用、生命周期、match、模块和结构体不套用 C++ 指针、访问权限、构造函数或 Objective-C 专属规则。

## 注释与校验

格式化不生成注释，也不替代 AGENTS.md 的职责、接口、变量及关键逻辑说明要求。保留现有中文说明和技术标识；本配置不重排注释正文、不格式化文档代码块。新增或修改逻辑的说明需人工结合调用链编写，不能机械填入“变量值”等无信息注释。

批量格式化后执行 `npm run format:rust:check`、`cargo +stable check --manifest-path src-tauri/Cargo.toml` 和 `git diff --check`，并核对 UTF-8、BOM 与源码注释。
