//! 节点数据模型与 `JSON → 树` 的校验解析。
//!
//! 插件返回的树按白名单校验：未知字段、未知节点类型、缺必填字段、裸色值与未知令牌都报错，
//! 不做静默降级。节点数与深度上限在这里执行（方案 §3.7 / ADR-0053），超限时停止构造整棵树，
//! 返回拒绝结论供宿主显示兜底句。
//!
//! 字段采用扁平写法（与阶段 0 原型一致），例如：
//! `{ "type": "text", "text": "CPU", "size": "caption", "tone": "--fence-muted" }`。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::render_tree::error::RenderTreeError;
use crate::render_tree::tokens::{ThemeToken, TokenError};

/// 单实例允许的最大节点数。
pub const MAX_NODES: usize = 5000;
/// 单实例允许的最大嵌套深度（根为 1）。
pub const MAX_DEPTH: usize = 32;
/// 单个文本节点的最大字符数，防止插件塞入无界文本。
const MAX_TEXT_CHARS: usize = 4096;
/// 单个节点 key 的最大字符数。
const MAX_KEY_CHARS: usize = 64;
/// 单条数值序列（`values`）的最大长度。
const MAX_VALUES: usize = 256;

/// 节点类型：方案 §3.3 的节点集 v1。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind
{
    Column,
    Row,
    Grid,
    Spacer,
    Divider,
    Text,
    Icon,
    Image,
    Progress,
    Ring,
    StatusDot,
    Sparkline,
    Bars,
    Badge,
    Link,
    Scroll,
    Empty,
}

impl NodeKind
{
    /// 从 JSON 里的 `type` 字段解析节点类型。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "column" => Self::Column,
            "row" => Self::Row,
            "grid" => Self::Grid,
            "spacer" => Self::Spacer,
            "divider" => Self::Divider,
            "text" => Self::Text,
            "icon" => Self::Icon,
            "image" => Self::Image,
            "progress" => Self::Progress,
            "ring" => Self::Ring,
            "status_dot" => Self::StatusDot,
            "sparkline" => Self::Sparkline,
            "bars" => Self::Bars,
            "badge" => Self::Badge,
            "link" => Self::Link,
            "scroll" => Self::Scroll,
            "empty" => Self::Empty,
            _ => return None,
        })
    }

    /// 序列化使用的名字（与 [`Self::parse`] 对称）。
    pub fn as_str(self) -> &'static str
    {
        match self
        {
            Self::Column => "column",
            Self::Row => "row",
            Self::Grid => "grid",
            Self::Spacer => "spacer",
            Self::Divider => "divider",
            Self::Text => "text",
            Self::Icon => "icon",
            Self::Image => "image",
            Self::Progress => "progress",
            Self::Ring => "ring",
            Self::StatusDot => "status_dot",
            Self::Sparkline => "sparkline",
            Self::Bars => "bars",
            Self::Badge => "badge",
            Self::Link => "link",
            Self::Scroll => "scroll",
            Self::Empty => "empty",
        }
    }

    /// 是否为容器类型：容器必须声明 `children`。
    pub fn is_container(self) -> bool { matches!(self, Self::Column | Self::Row | Self::Grid | Self::Scroll) }

    /// 是否为文本类节点：接受 `text` 与排版字段。
    pub fn is_text_like(self) -> bool { matches!(self, Self::Text | Self::Badge | Self::Link) }
}

/// 字号档位（DIP 由布局层的文本度量决定）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize
{
    Caption,
    #[default]
    Body,
    Title,
}

impl TextSize
{
    /// 从字符串解析字号档位。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "caption" => Self::Caption,
            "body" => Self::Body,
            "title" => Self::Title,
            _ => return None,
        })
    }
}

/// 字重档位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontWeight
{
    #[default]
    Normal,
    Medium,
    Semibold,
    Bold,
}

impl FontWeight
{
    /// 从字符串解析字重档位。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "normal" => Self::Normal,
            "medium" => Self::Medium,
            "semibold" => Self::Semibold,
            "bold" => Self::Bold,
            _ => return None,
        })
    }

    /// DirectWrite 使用的数值字重。
    pub fn numeric(self) -> u16
    {
        match self
        {
            Self::Normal => 400,
            Self::Medium => 500,
            Self::Semibold => 600,
            Self::Bold => 700,
        }
    }
}

/// 交叉轴对齐。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align
{
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

impl Align
{
    /// 从字符串解析对齐方式。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "start" => Self::Start,
            "center" => Self::Center,
            "end" => Self::End,
            "stretch" => Self::Stretch,
            _ => return None,
        })
    }
}

/// 主轴分布。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify
{
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

impl Justify
{
    /// 从字符串解析主轴分布。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "start" => Self::Start,
            "center" => Self::Center,
            "end" => Self::End,
            "space_between" => Self::SpaceBetween,
            _ => return None,
        })
    }
}

/// 弹性权重。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlexWeight
{
    pub flex: f32,
}

/// 尺寸：固定 DIP 或按剩余空间分配的弹性权重。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dimension
{
    /// 固定尺寸（DIP）。
    Fixed(f32),
    /// 弹性权重，只在父容器有剩余空间时生效。
    Flex(FlexWeight),
}

/// 样式子集（方案 §3.3）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style
{
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<Justify>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ThemeToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<ThemeToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<TextSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<FontWeight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncate_lines: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

/// 渲染树节点。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node
{
    #[serde(rename = "type")]
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<ThemeToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f32>,
    #[serde(flatten, default)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

impl Node
{
    /// 构造文本节点（内置内容与单测用）。
    pub fn text(text: impl Into<String>) -> Self { Self { kind: NodeKind::Text, key: None, text: Some(text.into()), tone: None, value: None, values: Vec::new(), style: Style::default(), children: Vec::new() } }

    /// 构造容器节点（内置内容与单测用）。
    pub fn container(kind: NodeKind, children: Vec<Node>) -> Self { Self { kind, key: None, text: None, tone: None, value: None, values: Vec::new(), style: Style::default(), children } }

    /// 统计整棵树的节点数与深度（根为 1），用于宿主自检与日志。
    pub fn measure_stats(&self) -> TreeStats
    {
        let mut stats = TreeStats::default(); // 累计节点数与最大深度。
        measure_node(self, 1, &mut stats);
        stats
    }
}

/// 递归累计节点数与深度。
fn measure_node(node: &Node, depth: usize, stats: &mut TreeStats)
{
    stats.nodes += 1;
    if depth > stats.depth
    {
        stats.depth = depth;
    }
    for child in &node.children
    {
        measure_node(child, depth + 1, stats);
    }
}

/// 树规模统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeStats
{
    pub nodes: usize,
    pub depth: usize,
}

/// 触发上限的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitReason
{
    /// 节点数超过 [`MAX_NODES`]。
    TooManyNodes
    {
        nodes: usize, limit: usize
    },
    /// 深度超过 [`MAX_DEPTH`]。
    TooDeep
    {
        depth: usize, limit: usize
    },
}

impl LimitReason
{
    /// 写进日志与兜底说明的中文原因。
    pub fn describe(self) -> String
    {
        match self
        {
            Self::TooManyNodes { nodes, limit } => format!("节点数 {nodes} 超过上限 {limit}"),
            Self::TooDeep { depth, limit } => format!("深度 {depth} 超过上限 {limit}"),
        }
    }
}

/// 解析结果：通过校验的树或上限拒绝结论。
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTree
{
    /// 通过校验的树；被上限拒绝时为 `None`。
    pub tree: Option<Node>,
    /// 已统计的节点数与深度（拒绝时是触发上限时的读数）。
    pub stats: TreeStats,
    /// 触发上限时的原因。
    pub limit: Option<LimitReason>,
}

impl ParsedTree
{
    /// 是否可以直接渲染。
    pub fn is_accepted(&self) -> bool { self.limit.is_none() && self.tree.is_some() }
}

/// 契约白名单：节点级字段。
const NODE_FIELDS: &[&str] = &["type", "key", "text", "tone", "value", "values", "children"];

/// 契约白名单：样式子集字段（扁平写在节点上）。
const STYLE_FIELDS: &[&str] = &["gap", "padding", "align", "justify", "width", "height", "minWidth", "radius", "background", "border", "borderWidth", "opacity", "size", "weight", "mono", "truncateLines", "textAlign", "columns", "visible"];

/// 超限时渲染的兜底句节点（方案 §3.7）：只含一行说明，不递归、不再触发上限。
pub fn fallback_node(reason: LimitReason) -> Node
{
    // 兜底句本身必须永远合法：文本固定、颜色只引用桌面层次要令牌。
    let mut node = Node::text(format!("该插件内容超出渲染上限（{}），已隐藏。", reason.describe()));
    node.key = Some("fallback".to_string());
    node.tone = ThemeToken::parse("--fence-muted").ok();
    node.style.truncate_lines = Some(2);
    node
}

/// 解析入口：把插件返回的 JSON 树校验成节点树，并执行节点数与深度上限。
pub fn parse_tree(value: &Value) -> Result<ParsedTree, RenderTreeError>
{
    let mut parser = Parser { stats: TreeStats::default(), limit: None }; // 单次解析的累计状态。
    let tree = parser.parse_node(value, "$", 1)?;
    Ok(ParsedTree { tree, stats: parser.stats, limit: parser.limit })
}

struct Parser
{
    stats: TreeStats,
    limit: Option<LimitReason>,
}

impl Parser
{
    /// 解析一个节点；上限触发后立即返回 `None`，让调用方丢弃整棵树。
    fn parse_node(&mut self, value: &Value, path: &str, depth: usize) -> Result<Option<Node>, RenderTreeError>
    {
        if self.limit.is_some()
        {
            return Ok(None);
        }
        let Some(object) = value.as_object()
        else
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "type", detail: "节点必须是对象".to_string() });
        };

        self.stats.nodes += 1;
        if depth > self.stats.depth
        {
            self.stats.depth = depth;
        }
        if self.stats.nodes > MAX_NODES
        {
            self.limit = Some(LimitReason::TooManyNodes { nodes: self.stats.nodes, limit: MAX_NODES });
            return Ok(None);
        }
        if depth > MAX_DEPTH
        {
            self.limit = Some(LimitReason::TooDeep { depth, limit: MAX_DEPTH });
            return Ok(None);
        }

        let kind = self.parse_kind(object, path)?;
        check_fields(object, path)?;
        check_applicability(object, kind, path)?;

        let key = optional_string(object, "key", path)?;
        if let Some(key) = &key
        {
            if key.is_empty() || key.chars().count() > MAX_KEY_CHARS
            {
                return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "key", detail: format!("长度必须在 1–{MAX_KEY_CHARS} 字符之间") });
            }
        }

        let text = optional_text(object, "text", path)?;
        let tone = optional_token(object, "tone", path)?;
        let value = optional_ratio(object, "value", path)?;
        let values = optional_values(object, "values", path)?;
        let style = parse_style(object, path)?;

        // 该节点类型的必填字段：缺失或显式 null 都按缺字段报错。
        for field in required_fields(kind)
        {
            if matches!(object.get(*field), None | Some(Value::Null))
            {
                return Err(RenderTreeError::MissingField { path: path.to_string(), field });
            }
        }
        if matches!(kind, NodeKind::Sparkline | NodeKind::Bars) && values.is_empty()
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "values", detail: "至少需要一个数值".to_string() });
        }

        let mut children = Vec::new(); // 容器子节点。
        if kind.is_container()
        {
            let Some(list) = object["children"].as_array()
            else
            {
                return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "children", detail: "必须是数组".to_string() });
            };
            for (index, child) in list.iter().enumerate()
            {
                let child_path = format!("{path}.children[{index}]"); // 子节点路径，便于定位。
                match self.parse_node(child, &child_path, depth + 1)?
                {
                    Some(node) => children.push(node),
                    None => return Ok(None),
                }
            }
        }

        Ok(Some(Node { kind, key, text, tone, value, values, style, children }))
    }

    /// 解析并校验 `type` 字段。
    fn parse_kind(&self, object: &Map<String, Value>, path: &str) -> Result<NodeKind, RenderTreeError>
    {
        let Some(value) = object.get("type")
        else
        {
            return Err(RenderTreeError::MissingField { path: path.to_string(), field: "type" });
        };
        let Some(name) = value.as_str()
        else
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "type", detail: "必须是字符串".to_string() });
        };
        NodeKind::parse(name).ok_or_else(|| RenderTreeError::UnknownNodeType { path: path.to_string(), value: name.to_string() })
    }
}

/// 节点类型必须声明、且不得多写的字段。
fn required_fields(kind: NodeKind) -> &'static [&'static str]
{
    match kind
    {
        NodeKind::Text | NodeKind::Badge | NodeKind::Link => &["text"],
        NodeKind::Progress | NodeKind::Ring => &["value"],
        NodeKind::Sparkline | NodeKind::Bars => &["values"],
        NodeKind::StatusDot => &["tone"],
        NodeKind::Column | NodeKind::Row | NodeKind::Grid | NodeKind::Scroll => &["children"],
        _ => &[],
    }
}

/// 拒绝白名单之外的字段，并给出字段名（未知键不忽略）。
fn check_fields(object: &Map<String, Value>, path: &str) -> Result<(), RenderTreeError>
{
    for field in object.keys()
    {
        if NODE_FIELDS.contains(&field.as_str()) || STYLE_FIELDS.contains(&field.as_str())
        {
            continue;
        }
        return Err(RenderTreeError::UnknownField { path: path.to_string(), field: field.clone() });
    }
    Ok(())
}

/// 拒绝用错节点类型的字段（例如给文本节点写 `children`），避免"写了却不生效"。
fn check_applicability(object: &Map<String, Value>, kind: NodeKind, path: &str) -> Result<(), RenderTreeError>
{
    let forbid = |field: &'static str| -> Result<(), RenderTreeError> {
        if object.contains_key(field)
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("{} 节点不接受该字段", kind.as_str()) });
        }
        Ok(())
    };

    if !kind.is_container()
    {
        for field in ["children", "gap", "padding", "align", "justify", "columns"]
        {
            forbid(field)?;
        }
    }
    if kind != NodeKind::Grid
    {
        forbid("columns")?;
    }
    if !kind.is_text_like()
    {
        for field in ["text", "size", "weight", "mono", "truncateLines", "textAlign"]
        {
            forbid(field)?;
        }
    }
    if !matches!(kind, NodeKind::Sparkline | NodeKind::Bars)
    {
        forbid("values")?;
    }
    if !matches!(kind, NodeKind::Progress | NodeKind::Ring)
    {
        forbid("value")?;
    }
    Ok(())
}

/// 读取可选字符串字段。
fn optional_string(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<String>, RenderTreeError>
{
    match object.get(field)
    {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是字符串".to_string() }),
    }
}

/// 读取可选文本，并限制长度。
fn optional_text(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<String>, RenderTreeError>
{
    let text = optional_string(object, field, path)?; // 先按字符串读。
    if let Some(text) = &text
    {
        if text.chars().count() > MAX_TEXT_CHARS
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("文本不能超过 {MAX_TEXT_CHARS} 字符") });
        }
    }
    Ok(text)
}

/// 读取可选布尔字段。
fn optional_bool(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<bool>, RenderTreeError>
{
    match object.get(field)
    {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是布尔值".to_string() }),
    }
}

/// 读取可选数值字段。
fn optional_f32(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<f32>, RenderTreeError>
{
    match object.get(field)
    {
        None | Some(Value::Null) => Ok(None),
        Some(value) =>
        {
            let Some(number) = value.as_f64()
            else
            {
                return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是数字".to_string() });
            };
            if !number.is_finite()
            {
                return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是有限数字".to_string() });
            }
            Ok(Some(number as f32))
        }
    }
}

/// 读取可选数值字段并检查区间。
fn optional_f32_in(object: &Map<String, Value>, field: &'static str, path: &str, min: f32, max: f32) -> Result<Option<f32>, RenderTreeError>
{
    let Some(number) = optional_f32(object, field, path)?
    else
    {
        return Ok(None);
    };
    if number < min || number > max
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("取值必须在 {min}–{max} 之间") });
    }
    Ok(Some(number))
}

/// 读取 0–1 的归一化比值。
fn optional_ratio(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<f32>, RenderTreeError> { optional_f32_in(object, field, path, 0.0, 1.0) }

/// 读取可选令牌字段，把令牌错误映射成带路径的契约错误。
fn optional_token(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<ThemeToken>, RenderTreeError>
{
    let Some(raw) = optional_string(object, field, path)?
    else
    {
        return Ok(None);
    };
    ThemeToken::parse(&raw).map(Some).map_err(|error| match error
    {
        TokenError::BareColor(value) => RenderTreeError::BareColor { path: path.to_string(), field, value },
        TokenError::Unknown(value) => RenderTreeError::UnknownToken { path: path.to_string(), field, value },
    })
}

/// 读取可选数值序列（`bars` / `sparkline` 的数据）。
fn optional_values(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Vec<f32>, RenderTreeError>
{
    let Some(value) = object.get(field)
    else
    {
        return Ok(Vec::new());
    };
    let Some(array) = value.as_array()
    else
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是数字数组".to_string() });
    };
    if array.len() > MAX_VALUES
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("最多 {MAX_VALUES} 个数值") });
    }
    let mut values = Vec::with_capacity(array.len()); // 归一化后的序列。
    for item in array
    {
        let Some(number) = item.as_f64()
        else
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "数组元素必须是数字".to_string() });
        };
        if !number.is_finite() || !(0.0..=1.0).contains(&number)
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "数组元素必须在 0–1 之间".to_string() });
        }
        values.push(number as f32);
    }
    Ok(values)
}

/// 读取可选尺寸字段：数字为固定值，`{"flex": n}` 为弹性权重。
fn optional_dimension(object: &Map<String, Value>, field: &'static str, path: &str) -> Result<Option<Dimension>, RenderTreeError>
{
    let Some(value) = object.get(field)
    else
    {
        return Ok(None);
    };
    if let Some(number) = value.as_f64()
    {
        if !number.is_finite() || number < 0.0
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "固定尺寸必须是非负有限数字".to_string() });
        }
        return Ok(Some(Dimension::Fixed(number as f32)));
    }
    if let Some(map) = value.as_object()
    {
        let flex = map.get("flex").and_then(Value::as_f64);
        if map.len() != 1 || flex.is_none()
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "尺寸对象只接受 {\"flex\": 权重}".to_string() });
        }
        let flex = flex.expect("已确认存在");
        if !flex.is_finite() || flex <= 0.0
        {
            return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "弹性权重必须是正的有限数字".to_string() });
        }
        return Ok(Some(Dimension::Flex(FlexWeight { flex: flex as f32 })));
    }
    Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "尺寸只能是数字或 {\"flex\": 权重}".to_string() })
}

/// 读取可选枚举字段。
fn optional_enum<T>(object: &Map<String, Value>, field: &'static str, path: &str, parse: fn(&str) -> Option<T>) -> Result<Option<T>, RenderTreeError>
{
    let Some(raw) = object.get(field)
    else
    {
        return Ok(None);
    };
    let Some(name) = raw.as_str()
    else
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是字符串".to_string() });
    };
    match parse(name)
    {
        Some(value) => Ok(Some(value)),
        None => Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("\"{name}\" 不是合法取值") }),
    }
}

/// 读取可选无符号短整型字段并检查区间。
fn optional_u16_in(object: &Map<String, Value>, field: &'static str, path: &str, min: u16, max: u16) -> Result<Option<u16>, RenderTreeError>
{
    let Some(value) = object.get(field)
    else
    {
        return Ok(None);
    };
    // JS 数字统一按 f64 传入，这里接受「整数值的浮点数」，拒绝 2.5 这类取值。
    let Some(number) = value.as_f64()
    else
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: "必须是非负整数".to_string() });
    };
    if !number.is_finite() || number.fract() != 0.0 || number < min as f64 || number > max as f64
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field, detail: format!("取值必须是 {min}–{max} 之间的整数") });
    }
    Ok(Some(number as u16))
}

/// 解析样式子集（扁平字段）。
fn parse_style(object: &Map<String, Value>, path: &str) -> Result<Style, RenderTreeError>
{
    let mut style = Style::default(); // 逐字段读取，缺省保持 None。
    style.gap = optional_f32_in(object, "gap", path, 0.0, 512.0)?;
    style.padding = optional_f32_in(object, "padding", path, 0.0, 512.0)?;
    style.align = optional_enum(object, "align", path, Align::parse)?;
    style.justify = optional_enum(object, "justify", path, Justify::parse)?;
    style.width = optional_dimension(object, "width", path)?;
    style.height = optional_dimension(object, "height", path)?;
    style.min_width = optional_f32_in(object, "minWidth", path, 0.0, 100_000.0)?;
    style.radius = optional_f32_in(object, "radius", path, 0.0, 1024.0)?;
    style.background = optional_token(object, "background", path)?;
    style.border = optional_token(object, "border", path)?;
    style.border_width = optional_f32_in(object, "borderWidth", path, 0.0, 64.0)?;
    style.opacity = optional_ratio(object, "opacity", path)?;
    style.size = optional_enum(object, "size", path, TextSize::parse)?;
    style.weight = optional_enum(object, "weight", path, FontWeight::parse)?;
    style.mono = optional_bool(object, "mono", path)?;
    style.truncate_lines = optional_u16_in(object, "truncateLines", path, 1, 32)?;
    style.text_align = optional_enum(object, "textAlign", path, Align::parse)?;
    style.columns = optional_u16_in(object, "columns", path, 1, 12)?;
    style.visible = optional_bool(object, "visible", path)?;

    if style.text_align == Some(Align::Stretch)
    {
        return Err(RenderTreeError::InvalidValue { path: path.to_string(), field: "textAlign", detail: "文本对齐只接受 start / center / end".to_string() });
    }
    Ok(style)
}
