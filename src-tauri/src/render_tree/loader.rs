//! 按显示位加载插件：懒加载、共享 JS 上下文与模块缓存、相对导入以插件根为界。
//!
//! 对应方案 §3.6 的加载规则：一个插件实例开一份 QuickJS 运行时与上下文，`[entries]` 里
//! 声明了哪个显示位才在真正需要时加载哪个文件；同一文件只求值一次（QuickJS 以模块名缓存，
//! 模块名就是插件根相对路径）；相对导入只允许落在插件根内，`../` 越界、绝对路径与裸模块名
//! 一律在解析阶段拒绝。
//!
//! 错误出口全部带文件（语法错误还带行列），不静默降级：文件缺失/不可读、语法错误、
//! 入口导出缺失、返回值不是渲染树，各有一条明确错误。

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value, module::Declared};

use crate::render_tree::error::RenderTreeError;
use crate::render_tree::host::{DEFAULT_ENTRY, TileRender, TileRequest, complete_tile_render};
use crate::render_tree::manifest::{ManifestMode, ManifestWarning, PluginManifest};
use crate::render_tree::slot::DisplaySlot;

/// QuickJS 堆上限（与内联宿主一致）。
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// 单个模块文件大小上限：插件脚本应当是手写代码，不是数据文件。
const MODULE_SIZE_LIMIT: u64 = 1024 * 1024;

/// 插件实例：一份运行时 + 一份上下文 + 一份清单，按显示位懒加载模块。
pub struct PluginSession
{
    root: PathBuf,
    manifest: PluginManifest,
    context: Context,
    failure: Rc<RefCell<Option<LoadFailure>>>,
    loaded: RefCell<BTreeSet<DisplaySlot>>,
}

impl PluginSession
{
    /// 打开一个插件实例：解析清单并准备运行时，但**不读取任何脚本文件**。
    pub fn open(root: impl AsRef<Path>, manifest_name: &str, manifest_text: &str) -> Result<Self, RenderTreeError>
    {
        let manifest = PluginManifest::parse(manifest_text).map_err(|error| RenderTreeError::Manifest { file: manifest_name.to_string(), line: error.line, detail: error.detail })?;
        for warning in &manifest.warnings
        {
            log_warning(warning);
        }

        let root = root.as_ref();
        let canonical = root.canonicalize().map_err(|error| RenderTreeError::Root { file: root.display().to_string(), detail: error.to_string() })?;
        if !canonical.is_dir()
        {
            return Err(RenderTreeError::Root { file: canonical.display().to_string(), detail: "不是目录".to_string() });
        }

        let failure: Rc<RefCell<Option<LoadFailure>>> = Rc::new(RefCell::new(None)); // 解析/加载阶段的诊断，供外层拼错误。
        let runtime = Runtime::new().map_err(|error| RenderTreeError::Script { detail: format!("创建 QuickJS 运行时失败：{error}") })?;
        runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
        runtime.set_loader(PluginResolver { failure: Rc::clone(&failure) }, PluginLoader { root: canonical.clone(), failure: Rc::clone(&failure) });
        let context = Context::full(&runtime).map_err(|error| RenderTreeError::Script { detail: format!("创建 QuickJS 上下文失败：{error}") })?;

        Ok(Self { root: canonical, manifest, context, failure, loaded: RefCell::new(BTreeSet::new()) })
    }

    /// 插件根（已规范化）。
    pub fn root(&self) -> &Path { &self.root }

    /// 清单解析结果。
    pub fn manifest(&self) -> &PluginManifest { &self.manifest }

    /// 已加载过的显示位（测试与诊断用；顺序固定）。
    pub fn loaded_slots(&self) -> Vec<DisplaySlot> { self.loaded.borrow().iter().copied().collect() }

    /// 该显示位声明的入口文件；未声明返回 `None`。
    pub fn entry_for(&self, slot: DisplaySlot) -> Option<&str> { self.manifest.entry_for(slot) }

    /// 懒加载一个显示位：读取并求值对应文件、检查约定的导出，不重复求值。
    pub fn load(&self, slot: DisplaySlot) -> Result<(), RenderTreeError>
    {
        let entry = self.declared_entry(slot)?;
        self.context.with(|ctx| -> Result<(), RenderTreeError> {
            let namespace = self.import(&ctx, slot, entry)?;
            // 新格式按显示位约定导出；旧单入口按迁移期的 DOM 契约导出（renderTile / settingsEntries / renderIslandHover）。
            let expected = self.expected_exports(slot);
            let present = expected.iter().any(|name| namespace.contains_key(*name).unwrap_or(false));
            if !present
            {
                return Err(RenderTreeError::EntryExport { slot, file: entry.to_string(), entry: expected.join(" / ") });
            }
            Ok(())
        })?;
        self.loaded.borrow_mut().insert(slot);
        Ok(())
    }

    /// 按显示位渲染渲染树；`settings` 是 DOM 显示位，按树渲染会被拒绝。
    pub fn render(&self, slot: DisplaySlot, request: &TileRequest) -> Result<TileRender, RenderTreeError>
    {
        let entry = self.declared_entry(slot)?;
        if !slot.is_render_tree()
        {
            return Err(RenderTreeError::SlotNotTree { slot, file: entry.to_string() });
        }
        let outcome = self.context.with(|ctx| -> Result<TileRender, RenderTreeError> {
            let namespace = self.import(&ctx, slot, entry)?;
            let function: Function = namespace.get(DEFAULT_ENTRY).map_err(|_| RenderTreeError::EntryExport { slot, file: entry.to_string(), entry: DEFAULT_ENTRY.to_string() })?;
            let input = Object::new(ctx.clone()).map_err(|error| self.entry_error(&ctx, slot, entry, error))?;
            input.set("instanceId", request.instance_id.as_str()).map_err(|error| self.entry_error(&ctx, slot, entry, error))?;
            input.set("tileType", request.tile_type.as_str()).map_err(|error| self.entry_error(&ctx, slot, entry, error))?;
            let value: Value = function.call((input,)).map_err(|error| self.entry_error(&ctx, slot, entry, error))?;
            complete_tile_render(&ctx, value, entry, &request.instance_id).map_err(|error| RenderTreeError::EntryValue { slot, file: entry.to_string(), detail: error.to_string() })
        })?;
        self.loaded.borrow_mut().insert(slot);
        Ok(outcome)
    }

    /// 该显示位在清单模式下的约定导出名。
    fn expected_exports(&self, slot: DisplaySlot) -> &'static [&'static str]
    {
        match (&self.manifest.mode, slot)
        {
            (ManifestMode::Entries(_), DisplaySlot::Tile | DisplaySlot::Island) => &[DEFAULT_ENTRY],
            (ManifestMode::Entries(_), DisplaySlot::Settings) => &["render", "settingsEntries"],
            (ManifestMode::Legacy { .. }, DisplaySlot::Tile) => &["renderTile", DEFAULT_ENTRY],
            (ManifestMode::Legacy { .. }, DisplaySlot::Island) => &["renderIslandHover", DEFAULT_ENTRY],
            (ManifestMode::Legacy { .. }, DisplaySlot::Settings) => &["settingsEntries", "render", DEFAULT_ENTRY],
        }
    }

    /// 取已声明的入口；未声明按 §3.6 视为「没有这个显示」。
    fn declared_entry(&self, slot: DisplaySlot) -> Result<&str, RenderTreeError> { self.manifest.entry_for(slot).ok_or(RenderTreeError::SlotNotDeclared { slot }) }

    /// 导入并求值某个显示位的模块；失败时把加载器记录的诊断翻成带文件/行列的错误。
    fn import<'js>(&self, ctx: &Ctx<'js>, slot: DisplaySlot, entry: &str) -> Result<Object<'js>, RenderTreeError>
    {
        *self.failure.borrow_mut() = None; // 每轮只保留本轮的第一条诊断。
        let promise = Module::import(ctx, entry).map_err(|error| self.map_failure(ctx, slot, entry, error))?;
        promise.finish::<Object>().map_err(|error| self.map_failure(ctx, slot, entry, error))
    }

    /// 把 rquickjs 错误与加载器记录合并成明确错误。
    fn map_failure(&self, ctx: &Ctx<'_>, slot: DisplaySlot, entry: &str, error: rquickjs::Error) -> RenderTreeError
    {
        if let Some(failure) = self.failure.borrow_mut().take()
        {
            return match failure.kind
            {
                FailureKind::Resolve => RenderTreeError::EntryImport { slot, file: failure.file, detail: failure.detail },
                FailureKind::Missing => RenderTreeError::EntryFile { slot, file: failure.file, detail: failure.detail },
                FailureKind::Syntax => RenderTreeError::EntrySyntax { slot, file: failure.file, line: failure.line.max(1), column: failure.column.max(1), detail: failure.detail },
            };
        }
        self.entry_error(ctx, slot, entry, error)
    }

    /// 运行期错误：带文件与消息（QuickJS 的栈里通常含行列）。
    fn entry_error(&self, ctx: &Ctx<'_>, slot: DisplaySlot, file: &str, error: rquickjs::Error) -> RenderTreeError { RenderTreeError::EntryValue { slot, file: file.to_string(), detail: describe_runtime_error(ctx, error) } }
}

/// 写兼容警告；日志系统未初始化时为空操作。
fn log_warning(warning: &ManifestWarning)
{
    match warning
    {
        ManifestWarning::LegacyEntry { entry } => crate::log_warn!("插件清单：{warning}（入口={entry}）"),
        ManifestWarning::IgnoredLegacyEntry { entry } => crate::log_warn!("插件清单：{warning}（入口={entry}）"),
    }
}

/// 加载器记录的诊断种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureKind
{
    /// 模块说明符不合法：越界、绝对路径、裸模块名。
    Resolve,
    /// 文件缺失或不可读。
    Missing,
    /// 模块源码语法错误。
    Syntax,
}

/// 一条加载诊断。
#[derive(Clone, Debug)]
struct LoadFailure
{
    kind: FailureKind,
    file: String,
    line: u32,
    column: u32,
    detail: String,
}

/// 模块解析器：把说明符归一化成插件根相对路径，越界即拒绝。
struct PluginResolver
{
    failure: Rc<RefCell<Option<LoadFailure>>>,
}

impl Resolver for PluginResolver
{
    /// 解析 `import` 说明符；入口模块的 `base` 为空，其余按相对路径处理。
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str, _attributes: Option<ImportAttributes<'js>>) -> rquickjs::Result<String>
    {
        match normalize_specifier(base, name)
        {
            Ok(path) => Ok(path),
            Err(detail) =>
            {
                let file = if base.is_empty() { name.to_string() } else { base.to_string() }; // 报出导入方，便于定位。
                *self.failure.borrow_mut() = Some(LoadFailure { kind: FailureKind::Resolve, file, line: 0, column: 0, detail: detail.clone() });
                Err(rquickjs::Error::new_resolving_message(base.to_string(), name.to_string(), detail))
            }
        }
    }
}

/// 模块加载器：只读插件根内的普通文件，逐级拒绝符号链接。
struct PluginLoader
{
    root: PathBuf,
    failure: Rc<RefCell<Option<LoadFailure>>>,
}

impl Loader for PluginLoader
{
    /// 读取并编译一个模块；失败时把原因记进共享诊断。
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _attributes: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, Declared>>
    {
        let fail = |failure: LoadFailure| -> rquickjs::Result<Module<'js, Declared>> {
            let detail = failure.detail.clone(); // 先留下要返回给 QuickJS 的说明。
            *self.failure.borrow_mut() = Some(failure);
            Err(rquickjs::Error::new_loading_message(name.to_string(), detail))
        };

        let relative = match safe_relative(name)
        {
            Ok(relative) => relative,
            Err(detail) => return fail(LoadFailure { kind: FailureKind::Resolve, file: name.to_string(), line: 0, column: 0, detail }),
        };
        let path = self.root.join(relative);
        if let Err(detail) = check_regular_file(&self.root, &path)
        {
            return fail(LoadFailure { kind: FailureKind::Missing, file: name.to_string(), line: 0, column: 0, detail });
        }
        let metadata = match std::fs::metadata(&path)
        {
            Ok(metadata) => metadata,
            Err(error) => return fail(LoadFailure { kind: FailureKind::Missing, file: name.to_string(), line: 0, column: 0, detail: format!("读取失败：{error}") }),
        };
        if metadata.len() > MODULE_SIZE_LIMIT
        {
            return fail(LoadFailure { kind: FailureKind::Missing, file: name.to_string(), line: 0, column: 0, detail: format!("文件超过 {MODULE_SIZE_LIMIT} 字节") });
        }
        let source = match std::fs::read_to_string(&path)
        {
            Ok(source) => source,
            Err(error) => return fail(LoadFailure { kind: FailureKind::Missing, file: name.to_string(), line: 0, column: 0, detail: format!("不是可读的 UTF-8 文本：{error}") }),
        };
        match Module::declare(ctx.clone(), name, source)
        {
            Ok(module) => Ok(module),
            Err(error) =>
            {
                // 导入说明符的解析错误会在编译期先于语法错误出现，保留解析器记下的原因。
                if self.failure.borrow().is_some()
                {
                    return Err(error);
                }
                let (line, column, detail) = describe_syntax_error(ctx, error);
                fail(LoadFailure { kind: FailureKind::Syntax, file: name.to_string(), line, column, detail })
            }
        }
    }
}

/// 归一化导入说明符：入口按根相对路径，其余只允许相对路径，结果必须仍在插件根内。
fn normalize_specifier(base: &str, name: &str) -> Result<String, String>
{
    if name.is_empty()
    {
        return Err("导入路径为空".to_string());
    }
    if name.contains(['\\', ':', '%', '\0']) || name.starts_with('/') || name.contains("://")
    {
        return Err(format!("只允许使用 / 分隔的插件根内相对路径：{name}"));
    }
    let relative = name.starts_with('.'); // 入口用根相对路径，其余用相对路径。
    if !relative && !base.is_empty()
    {
        return Err(format!("插件的模块导入只允许相对路径（./ 或 ../）：{name}"));
    }

    let mut parts: Vec<&str> = Vec::new(); // 已确认的路径段。
    if relative && !base.is_empty()
    {
        if let Some((directory, _)) = base.rsplit_once('/')
        {
            parts.extend(directory.split('/').filter(|part| !part.is_empty()));
        }
    }
    for part in name.split('/')
    {
        match part
        {
            "" | "." => continue,
            ".." =>
            {
                if parts.pop().is_none()
                {
                    return Err(format!("导入路径越出插件根：{name}"));
                }
            }
            part =>
            {
                if part.contains(['\\', ':', '%', '\0'])
                {
                    return Err(format!("导入路径含非法字符：{name}"));
                }
                parts.push(part);
            }
        }
    }
    if parts.is_empty()
    {
        return Err(format!("导入路径没有落到具体文件：{name}"));
    }
    let extension = parts.last().and_then(|file| file.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()));
    if !matches!(extension.as_deref(), Some("js") | Some("mjs"))
    {
        return Err(format!("只能导入 .js 或 .mjs 模块：{name}"));
    }
    Ok(parts.join("/"))
}

/// 再次确认模块名是安全的根相对路径（加载器可能被 QuickJS 直接调用）。
fn safe_relative(name: &str) -> Result<&str, String>
{
    if name.is_empty() || name.contains(['\\', ':', '%', '\0'])
    {
        return Err(format!("模块名不是插件根内相对路径：{name}"));
    }
    let path = Path::new(name);
    if path.is_absolute() || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("模块名不能用绝对路径或 ..：{name}"));
    }
    Ok(name)
}

/// 确认模块路径是插件根下的普通文件，且沿途没有符号链接或重解析点。
fn check_regular_file(root: &Path, path: &Path) -> Result<(), String>
{
    if !path.starts_with(root)
    {
        return Err("模块路径越出插件根".to_string());
    }
    for ancestor in path.ancestors().take_while(|ancestor| ancestor.starts_with(root))
    {
        let metadata = match std::fs::symlink_metadata(ancestor)
        {
            Ok(metadata) => metadata,
            Err(error) if ancestor == path && error.kind() == std::io::ErrorKind::NotFound => return Err("文件不存在".to_string()),
            Err(error) if ancestor != path && error.kind() == std::io::ErrorKind::NotFound => return Err("上级目录不存在".to_string()),
            Err(error) => return Err(format!("读取文件属性失败：{error}")),
        };
        if metadata.file_type().is_symlink()
        {
            return Err("模块不能是符号链接".to_string());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0
            {
                return Err("模块不能是重解析点".to_string());
            }
        }
    }
    Ok(())
}

/// 读取语法错误的消息与行列。
///
/// QuickJS 编译期的语法错误不带 `lineNumber` 属性，但会把位置写进 `stack`
/// （`js_parse_error` 调 `build_backtrace`，形如 `at tile/main.js:2:13`），这里从栈里取第一条位置。
fn describe_syntax_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> (u32, u32, String)
{
    let caught = ctx.catch(); // 取走异常，避免影响后续求值。
    let object = Object::from_value(caught.clone()).ok();
    let message = object.as_ref().and_then(|object| object.get::<_, Option<String>>("message").ok().flatten()).or_else(|| rquickjs::FromJs::from_js(ctx, caught).ok()).unwrap_or_else(|| error.to_string());
    let stack = object.as_ref().and_then(|object| object.get::<_, Option<String>>("stack").ok().flatten()).unwrap_or_default();
    let (line, column) = position_from_stack(&stack).unwrap_or((0, 0));
    (line, column, message)
}

/// 从 QuickJS 的栈文本里取第一个 `文件:行:列`。
fn position_from_stack(stack: &str) -> Option<(u32, u32)>
{
    for line in stack.lines()
    {
        let mut parts = line.trim().rsplit(':');
        let column = parts.next()?.trim().parse::<u32>().ok()?;
        let line_number = parts.next()?.trim().parse::<u32>().ok()?;
        if column > 0 && line_number > 0
        {
            return Some((line_number, column));
        }
    }
    None
}

/// 运行期错误的消息与栈（栈里通常带文件与行列）。
fn describe_runtime_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> String
{
    if !error.is_exception()
    {
        return error.to_string();
    }
    let caught = ctx.catch();
    if let Ok(object) = Object::from_value(caught)
    {
        if let Some(exception) = rquickjs::Exception::from_object(object)
        {
            let message = exception.message().unwrap_or_default(); // 异常消息。
            let stack = exception.stack().unwrap_or_default(); // 栈里带文件与行列。
            return if stack.is_empty() { message } else { format!("{message}（{stack}）") };
        }
    }
    error.to_string()
}
