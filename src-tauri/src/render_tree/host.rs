//! QuickJS 宿主：加载 ESM 插件模块、调用入口函数，把返回值转成校验后的节点树。
//!
//! 运行时是进程内的 rquickjs（无 JIT）。**不向插件暴露 `fetch`、`document`、`window` 等浏览器
//! 能力**：QuickJS 只带语言内建对象，本模块也不注入任何宿主全局。本文件只处理「已读入内存的
//! 模块源码」；按清单加载插件根下的文件、解析相对导入由 [`crate::render_tree::loader`] 负责，
//! 两者共用 [`complete_tile_render`] 完成契约校验与上限判定。
//!
//! 上限在两层执行：JS 值转 JSON 时先挡住病态结构（循环引用、超大对象图），解析时再按
//! 方案 §3.7 精确判定节点数与深度，超限返回 [`TileRender::Rejected`]（带兜底节点）并记 Warn 日志。

use rquickjs::{Array, Context, Ctx, Function, Module, Object, Runtime, Type, Value};

use crate::render_tree::error::RenderTreeError;
use crate::render_tree::node::{LimitReason, MAX_DEPTH, MAX_NODES, Node, TreeStats, fallback_node, parse_tree};

/// 模块默认导出的入口函数名。
pub const DEFAULT_ENTRY: &str = "default";
/// QuickJS 堆上限：限制插件内存占用，超限时求值报错。
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// JS→JSON 转换阶段的对象数上限：只挡病态结构，精确判定交给解析层。
const CONVERT_OBJECT_LIMIT: usize = MAX_NODES * 4 + 64;
/// JS→JSON 转换阶段的嵌套上限（同时兜住循环引用）。
const CONVERT_DEPTH_LIMIT: usize = 128;

/// 宿主回传给插件的一次交互事件（点击了哪个动作区）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileEvent
{
    /// 被点击节点声明的动作标识（与布局给出的 `ActionHit::action` 同源）。
    pub action: String,
}

impl TileEvent
{
    /// 构造一次点击事件。
    pub fn new(action: impl Into<String>) -> Self { Self { action: action.into() } }
}

/// 调用入口函数时传给插件的实例信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileRequest
{
    /// 插件实例标识。
    pub instance_id: String,
    /// 格子类型（插件自定义，宿主透传）。
    pub tile_type: String,
    /// 本次渲染前宿主收到、尚未消费的交互事件；插件据此在同一棵树里表达状态变化。
    pub events: Vec<TileEvent>,
}

impl TileRequest
{
    /// 构造实例信息（无待处理事件）。
    pub fn new(instance_id: impl Into<String>, tile_type: impl Into<String>) -> Self { Self { instance_id: instance_id.into(), tile_type: tile_type.into(), events: Vec::new() } }

    /// 带上待处理的交互事件。
    pub fn with_events(mut self, events: Vec<TileEvent>) -> Self { self.events = events; self }
}

/// 一次渲染的结果。
#[derive(Clone, Debug, PartialEq)]
pub enum TileRender
{
    /// 通过校验的树。
    Tree
    {
        tree: Node, stats: TreeStats
    },
    /// 超过节点数或深度上限：拒绝渲染，返回兜底节点与原因。
    Rejected
    {
        reason: LimitReason, stats: TreeStats, fallback: Node
    },
}

/// 加载模块源码、调用入口函数并返回渲染结果。
pub fn render_tile(module_source: &str, module_name: &str, entry: &str, request: &TileRequest) -> Result<TileRender, RenderTreeError>
{
    let runtime = Runtime::new().map_err(|error| RenderTreeError::Script { detail: format!("创建 QuickJS 运行时失败：{error}") })?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
    let context = Context::full(&runtime).map_err(|error| RenderTreeError::Script { detail: format!("创建 QuickJS 上下文失败：{error}") })?;
    context.with(|ctx| render_in_context(&ctx, module_source, module_name, entry, request))
}

/// 在给定上下文中编译、求值模块并调用入口。
fn render_in_context<'js>(ctx: &Ctx<'js>, module_source: &str, module_name: &str, entry: &str, request: &TileRequest) -> Result<TileRender, RenderTreeError>
{
    let module = Module::declare(ctx.clone(), module_name, module_source).map_err(|error| describe_js_error(ctx, error))?;
    let (module, promise) = module.eval().map_err(|error| describe_js_error(ctx, error))?;
    // 无顶层 await 的模块在求值后立即完成；带顶层 await 的模块这里会报错，属阶段 3 未支持项。
    promise.finish::<()>().map_err(|error| describe_js_error(ctx, error))?;
    let namespace = module.namespace().map_err(|error| describe_js_error(ctx, error))?;
    let function: Function = namespace.get(entry).map_err(|_| RenderTreeError::MissingEntry { entry: entry.to_string() })?;

    let input = Object::new(ctx.clone()).map_err(|error| describe_js_error(ctx, error))?;
    input.set("instanceId", request.instance_id.as_str()).map_err(|error| describe_js_error(ctx, error))?;
    input.set("tileType", request.tile_type.as_str()).map_err(|error| describe_js_error(ctx, error))?;
    // 交互事件按数组透传：插件用纯函数表达「收到哪些点击」，不需要宿主回调。
    let events = Array::new(ctx.clone()).map_err(|error| describe_js_error(ctx, error))?;
    for (index, event) in request.events.iter().enumerate()
    {
        let item = Object::new(ctx.clone()).map_err(|error| describe_js_error(ctx, error))?;
        item.set("action", event.action.as_str()).map_err(|error| describe_js_error(ctx, error))?;
        events.set(index, item).map_err(|error| describe_js_error(ctx, error))?;
    }
    input.set("events", events).map_err(|error| describe_js_error(ctx, error))?;
    let value: Value<'js> = function.call((input,)).map_err(|error| describe_js_error(ctx, error))?;
    complete_tile_render(ctx, value, module_name, &request.instance_id)
}

/// 把入口返回值走完「JS→JSON → 契约校验 → 上限判定」，供内联源码路径与文件加载路径共用。
///
/// `module_label` 只用于日志（内联路径传模块名，文件加载路径传插件根相对路径）。
pub(crate) fn complete_tile_render<'js>(ctx: &Ctx<'js>, value: Value<'js>, module_label: &str, instance_id: &str) -> Result<TileRender, RenderTreeError>
{
    let json = match convert(ctx, value)
    {
        Ok(json) => json,
        // 转换阶段提前中止：对象图过大或嵌套过深，直接按对应上限拒绝渲染。
        Err(ConvertError::TooMany(objects)) =>
        {
            let reason = LimitReason::TooManyNodes { nodes: objects, limit: MAX_NODES }; // 对象数已超过节点上限，先按节点上限拒绝。
            let fallback = fallback_node(reason);
            crate::log_warn!("插件渲染树超限：模块={module_label} 实例={instance_id} {}(转换阶段对象数={objects})，已拒绝渲染", reason.describe());
            return Ok(TileRender::Rejected { reason, stats: TreeStats { nodes: objects, depth: 0 }, fallback });
        }
        Err(ConvertError::TooDeep(depth)) =>
        {
            let reason = LimitReason::TooDeep { depth, limit: MAX_DEPTH };
            let fallback = fallback_node(reason);
            crate::log_warn!("插件渲染树超限：模块={module_label} 实例={instance_id} {}(转换阶段深度={depth})，已拒绝渲染", reason.describe());
            return Ok(TileRender::Rejected { reason, stats: TreeStats { nodes: 0, depth }, fallback });
        }
        Err(ConvertError::Unsupported(detail)) => return Err(RenderTreeError::Script { detail }),
    };

    let parsed = parse_tree(&json)?;
    match parsed.limit
    {
        Some(reason) =>
        {
            let fallback = fallback_node(reason);
            crate::log_warn!("插件渲染树超限：模块={module_label} 实例={instance_id} {}(节点={} 深度={})，已拒绝渲染", reason.describe(), parsed.stats.nodes, parsed.stats.depth);
            Ok(TileRender::Rejected { reason, stats: parsed.stats, fallback })
        }
        None =>
        {
            let tree = parsed.tree.expect("未触发上限时解析结果必含节点树");
            Ok(TileRender::Tree { tree, stats: parsed.stats })
        }
    }
}

/// JS→JSON 转换阶段的中止原因。
#[derive(Clone, Debug, PartialEq, Eq)]
enum ConvertError
{
    /// 对象数超过保护上限。
    TooMany(usize),
    /// 嵌套深度超过保护上限（含循环引用）。
    TooDeep(usize),
    /// 出现不能表达为 JSON 的类型（函数、Symbol、BigInt 等）。
    Unsupported(String),
}

/// 把 JS 值递归转成 JSON 值，同时统计对象数与嵌套深度。
fn convert<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<serde_json::Value, ConvertError>
{
    let mut converter = Converter { objects: 0, depth: 0 }; // 单次转换的累计状态。
    converter.value(ctx, value, 1)
}

struct Converter
{
    objects: usize,
    depth: usize,
}

impl Converter
{
    /// 转换一个值；数组与对象递归处理。
    fn value<'js>(&mut self, ctx: &Ctx<'js>, value: Value<'js>, depth: usize) -> Result<serde_json::Value, ConvertError>
    {
        if depth > CONVERT_DEPTH_LIMIT
        {
            return Err(ConvertError::TooDeep(depth));
        }
        if depth > self.depth
        {
            self.depth = depth;
        }
        match value.type_of()
        {
            Type::Uninitialized | Type::Undefined | Type::Null => Ok(serde_json::Value::Null),
            Type::Bool => Ok(serde_json::Value::Bool(value.as_bool().unwrap_or(false))),
            Type::Int | Type::Float =>
            {
                let number = value.as_number().unwrap_or(0.0); // JS 数字统一是双精度。
                Ok(serde_json::Number::from_f64(number).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null))
            }
            Type::String => Ok(serde_json::Value::String(rquickjs::FromJs::from_js(ctx, value).map_err(|error| ConvertError::Unsupported(format!("字符串转换失败：{error}")))?)),
            Type::Array =>
            {
                self.count_object()?;
                let array = Array::from_value(value).map_err(|error| ConvertError::Unsupported(format!("数组转换失败：{error}")))?;
                let mut items = Vec::with_capacity(array.len()); // 逐项转换。
                for index in 0..array.len()
                {
                    let item: Value = array.get(index).map_err(|error| ConvertError::Unsupported(format!("数组元素读取失败：{error}")))?;
                    items.push(self.value(ctx, item, depth + 1)?);
                }
                Ok(serde_json::Value::Array(items))
            }
            Type::Object | Type::Exception =>
            {
                self.count_object()?;
                let object = Object::from_value(value).map_err(|error| ConvertError::Unsupported(format!("对象转换失败：{error}")))?;
                let mut map = serde_json::Map::new(); // 逐属性转换。
                for property in object.props::<String, Value>()
                {
                    let (name, item) = property.map_err(|error| ConvertError::Unsupported(format!("对象属性读取失败：{error}")))?;
                    map.insert(name, self.value(ctx, item, depth + 1)?);
                }
                Ok(serde_json::Value::Object(map))
            }
            other => Err(ConvertError::Unsupported(format!("入口返回值含不支持的类型：{}", other.as_str()))),
        }
    }

    /// 累计对象数并执行保护上限。
    fn count_object(&mut self) -> Result<(), ConvertError>
    {
        self.objects += 1;
        if self.objects > CONVERT_OBJECT_LIMIT
        {
            return Err(ConvertError::TooMany(self.objects));
        }
        Ok(())
    }
}

/// 把 rquickjs 错误转成带异常信息的契约错误。
fn describe_js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> RenderTreeError
{
    if !error.is_exception()
    {
        return RenderTreeError::Script { detail: error.to_string() };
    }
    let caught = ctx.catch(); // 取走待处理异常，避免影响后续求值。
    // Error 对象的 message 是不可枚举属性，先用 Exception 接口直读，再退回通用转换。
    if let Ok(object) = Object::from_value(caught.clone())
    {
        if let Some(exception) = rquickjs::Exception::from_object(object)
        {
            if let Some(message) = exception.message()
            {
                return RenderTreeError::Script { detail: message };
            }
        }
    }
    let detail = match convert(ctx, caught)
    {
        Ok(serde_json::Value::String(text)) => text,
        Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map).to_string(),
        Ok(other) => other.to_string(),
        Err(_) => error.to_string(),
    };
    RenderTreeError::Script { detail }
}
