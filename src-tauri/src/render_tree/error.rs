//! 渲染树的错误类型：契约校验失败与宿主执行失败都给出明确原因，不做静默降级。

use std::fmt;

use crate::render_tree::slot::DisplaySlot;

/// 渲染树解析或宿主执行失败的原因。
///
/// 字段里的 `path` 是从根开始的节点路径（根为 `$`，子节点写作 `$.children[2]`），
/// 便于插件作者直接定位到出错的那一个字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderTreeError
{
    /// 颜色位置传入了裸色值（十六进制、`rgb()`/`hsl()` 或颜色关键字）。
    BareColor
    {
        path: String, field: &'static str, value: String
    },
    /// 令牌名不在桌面层白名单里（含界面调色板的 `--color-*`）。
    UnknownToken
    {
        path: String, field: &'static str, value: String
    },
    /// 该节点类型缺少必填字段。
    MissingField
    {
        path: String, field: &'static str
    },
    /// 节点类型不在节点集 v1 内。
    UnknownNodeType
    {
        path: String, value: String
    },
    /// 契约白名单之外的字段；未知键直接拒绝，不忽略。
    UnknownField
    {
        path: String, field: String
    },
    /// 字段取值不合法（类型、范围或适用节点类型）。
    InvalidValue
    {
        path: String, field: &'static str, detail: String
    },
    /// 节点数量超过单实例上限。
    TooManyNodes
    {
        nodes: usize, limit: usize
    },
    /// 嵌套深度超过单实例上限。
    TooDeep
    {
        depth: usize, limit: usize
    },
    /// 插件模块未导出约定的入口函数。
    MissingEntry
    {
        entry: String
    },
    /// 模块编译、求值或入口调用抛出异常。
    Script
    {
        detail: String
    },
    /// 插件清单格式错误：文件与行号。
    Manifest
    {
        file: String, line: usize, detail: String
    },
    /// 插件根不可用（不存在、不是目录或不可读）。
    Root
    {
        file: String, detail: String
    },
    /// 相对导入越界、使用了裸模块名或绝对路径。
    EntryImport
    {
        slot: DisplaySlot, file: String, detail: String
    },
    /// 显示位文件不可用：缺失、越界、不是普通文件或超过大小上限。
    EntryFile
    {
        slot: DisplaySlot, file: String, detail: String
    },
    /// 显示位模块语法错误：文件与行列。
    EntrySyntax
    {
        slot: DisplaySlot, file: String, line: u32, column: u32, detail: String
    },
    /// 显示位模块缺少约定的导出。
    EntryExport
    {
        slot: DisplaySlot, file: String, entry: String
    },
    /// 显示位返回值不符合渲染树契约：文件 + 字段路径。
    EntryValue
    {
        slot: DisplaySlot, file: String, detail: String
    },
    /// 请求的显示位不是渲染树（`settings` 是 DOM 显示位）。
    SlotNotTree
    {
        slot: DisplaySlot, file: String
    },
    /// 请求的显示位没有在清单里声明。
    SlotNotDeclared
    {
        slot: DisplaySlot
    },
    /// 令牌不在颜色表里：适配层无法把令牌映射成具体颜色。
    PaletteMiss
    {
        path: String, token: String
    },
    /// 适配层遇到不支持的图元/节点：明确报错，不静默跳过。
    UnsupportedPrimitive
    {
        path: String, detail: String
    },
}

impl fmt::Display for RenderTreeError
{
    /// 输出带路径的中文错误说明。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result
    {
        match self
        {
            Self::BareColor { path, field, value } => write!(formatter, "渲染树不接受裸色值：{path}.{field} = \"{value}\"，请改引用桌面层 --fence-* 令牌名"),
            Self::UnknownToken { path, field, value } => write!(formatter, "渲染树令牌不在桌面层白名单：{path}.{field} = \"{value}\""),
            Self::MissingField { path, field } => write!(formatter, "渲染树缺少必填字段：{path}.{field}"),
            Self::UnknownNodeType { path, value } => write!(formatter, "未知的渲染树节点类型：{path}.type = \"{value}\""),
            Self::UnknownField { path, field } => write!(formatter, "渲染树不接受字段：{path}.{field}"),
            Self::InvalidValue { path, field, detail } => write!(formatter, "渲染树字段取值不合法：{path}.{field}（{detail}）"),
            Self::TooManyNodes { nodes, limit } => write!(formatter, "渲染树节点数 {nodes} 超过上限 {limit}"),
            Self::TooDeep { depth, limit } => write!(formatter, "渲染树深度 {depth} 超过上限 {limit}"),
            Self::MissingEntry { entry } => write!(formatter, "插件模块没有导出入口函数 {entry}"),
            Self::Script { detail } => write!(formatter, "插件执行失败：{detail}"),
            Self::Manifest { file, line, detail } => write!(formatter, "插件清单错误：{file}:{line}：{detail}"),
            Self::Root { file, detail } => write!(formatter, "插件根不可用：{file}（{detail}）"),
            Self::EntryImport { slot, file, detail } => write!(formatter, "显示位 {slot} 的模块导入不合法：{file}：{detail}"),
            Self::EntryFile { slot, file, detail } => write!(formatter, "显示位 {slot} 的文件不可用：{file}（{detail}）"),
            Self::EntrySyntax { slot, file, line, column, detail } => write!(formatter, "显示位 {slot} 的模块语法错误：{file}:{line}:{column}：{detail}"),
            Self::EntryExport { slot, file, entry } => write!(formatter, "显示位 {slot} 的模块没有导出 {entry}：{file}"),
            Self::EntryValue { slot, file, detail } => write!(formatter, "显示位 {slot} 的返回值不是合法渲染树：{file}：{detail}"),
            Self::SlotNotTree { slot, file } => write!(formatter, "显示位 {slot} 不是渲染树（DOM 显示位），不能按树渲染：{file}"),
            Self::SlotNotDeclared { slot } => write!(formatter, "插件清单没有声明显示位 {slot}"),
            Self::PaletteMiss { path, token } => write!(formatter, "颜色表缺少令牌：{path} 引用了 {token}，请先向令牌表注入该令牌的颜色"),
            Self::UnsupportedPrimitive { path, detail } => write!(formatter, "渲染树适配层不支持该图元：{path}（{detail}）"),
        }
    }
}

impl std::error::Error for RenderTreeError {}
