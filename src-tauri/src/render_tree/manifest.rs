//! 插件清单的 `[entries]` 解析：按显示位声明 `tile` / `island` / `settings`。
//!
//! 对应方案 §3.6：未声明的显示位 = 该插件没有这个显示，不加载对应文件；旧单入口
//! `entry = index.js` 在迁移期仍可识别并记警告；一个文件只能实现一个显示位。路径在解析期
//! 就按「插件根内相对路径」校验（拒绝绝对路径、反斜杠/盘符、`..` 与非脚本扩展名），
//! 越界在真正读文件之前就失败。

use std::fmt;
use std::path::{Component, Path};

use crate::render_tree::slot::DisplaySlot;

/// 清单解析错误：带行号，便于插件作者定位。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError
{
    /// 1 起的行号。
    pub line: usize,
    /// 中文原因。
    pub detail: String,
}

impl ManifestError
{
    /// 构造一条行级错误。
    fn new(line: usize, detail: impl Into<String>) -> Self { Self { line, detail: detail.into() } }
}

impl fmt::Display for ManifestError
{
    /// 输出「第 N 行：原因」。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { write!(formatter, "第 {} 行：{}", self.line, self.detail) }
}

impl std::error::Error for ManifestError {}

/// 清单兼容警告：由调用方写日志，不静默吞掉。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestWarning
{
    /// 旧单入口清单：`entry = …`，按兼容模式加载。
    LegacyEntry
    {
        entry: String
    },
    /// 同时声明了 `[entries]` 与 `entry`：以 `[entries]` 为准，忽略 `entry`。
    IgnoredLegacyEntry
    {
        entry: String
    },
}

impl fmt::Display for ManifestWarning
{
    /// 输出中文告警。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result
    {
        match self
        {
            Self::LegacyEntry { entry } => write!(formatter, "插件清单使用旧单入口 entry={entry}，按兼容模式加载；建议迁移到 [entries] 按显示位分文件"),
            Self::IgnoredLegacyEntry { entry } => write!(formatter, "插件清单同时声明了 [entries] 与 entry={entry}，以 [entries] 为准，忽略 entry"),
        }
    }
}

/// 新格式 `[entries]` 的三个显示位声明。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entries
{
    /// 格子入口（渲染树）。
    pub tile: Option<String>,
    /// 浮岛卡入口（渲染树）。
    pub island: Option<String>,
    /// 设置条目入口（DOM，在设置窗里加载）。
    pub settings: Option<String>,
}

impl Entries
{
    /// 取某个显示位的入口文件。
    pub fn get(&self, slot: DisplaySlot) -> Option<&str>
    {
        match slot
        {
            DisplaySlot::Tile => self.tile.as_deref(),
            DisplaySlot::Island => self.island.as_deref(),
            DisplaySlot::Settings => self.settings.as_deref(),
        }
    }

    /// 已声明的显示位，顺序固定为 tile / island / settings。
    pub fn declared(&self) -> Vec<DisplaySlot> { DisplaySlot::ALL.into_iter().filter(|slot| self.get(*slot).is_some()).collect() }

    /// 是否至少声明了一个显示位。
    fn is_empty(&self) -> bool { self.declared().is_empty() }

    /// 写入一个显示位；重复声明由调用方先行判错。
    fn set(&mut self, slot: DisplaySlot, value: String)
    {
        match slot
        {
            DisplaySlot::Tile => self.tile = Some(value),
            DisplaySlot::Island => self.island = Some(value),
            DisplaySlot::Settings => self.settings = Some(value),
        }
    }
}

/// 清单的显示位声明方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestMode
{
    /// 新格式：`[entries]` 按显示位分文件。
    Entries(Entries),
    /// 旧格式：单入口 `entry = index.js`，三个显示位都从同一文件加载。
    Legacy
    {
        entry: String
    },
}

/// 与渲染树加载相关的清单元数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginManifest
{
    /// 插件标识（缺省为空，仅用于日志与错误信息）。
    pub plugin_id: Option<String>,
    /// 显示位声明方式。
    pub mode: ManifestMode,
    /// 兼容警告，调用方负责写日志。
    pub warnings: Vec<ManifestWarning>,
}

impl PluginManifest
{
    /// 解析清单文本。
    pub fn parse(text: &str) -> Result<Self, ManifestError>
    {
        let lines = raw_lines(text)?; // 先切成「节 + 键 + 值 + 行号」。
        let plugin_id = lines.iter().find(|line| line.section == "plugin" && line.key == "id").map(|line| line.value.clone());
        let legacy = lines.iter().find(|line| line.section == "plugin" && line.key == "entry").cloned();

        // [entries] 节：键必须是三个显示位之一，值必须指向插件根内的脚本。
        let mut entries = Entries::default();
        let mut declared: Vec<(String, DisplaySlot)> = Vec::new(); // 已声明文件，用于「一个文件只能实现一个显示位」。
        for line in lines.iter().filter(|line| line.section == "entries")
        {
            let Some(slot) = DisplaySlot::parse(&line.key)
            else
            {
                return Err(ManifestError::new(line.line, format!("未知的显示位 {}；[entries] 只接受 tile / island / settings", line.key)));
            };
            if entries.get(slot).is_some()
            {
                return Err(ManifestError::new(line.line, format!("显示位 {} 重复声明", slot)));
            }
            if line.value.is_empty()
            {
                return Err(ManifestError::new(line.line, format!("显示位 {} 的入口文件为空", slot)));
            }
            validate_entry_path(slot, &line.value, line.line)?;
            if let Some((file, other)) = declared.iter().find(|(file, _)| file.eq_ignore_ascii_case(&line.value))
            {
                return Err(ManifestError::new(line.line, format!("显示位 {slot} 与 {other} 指向同一文件 {file}；一个文件只能实现一个显示位")));
            }
            declared.push((line.value.clone(), slot));
            entries.set(slot, line.value.clone());
        }

        if !entries.is_empty()
        {
            let mut warnings = Vec::new();
            if let Some(legacy) = &legacy
            {
                warnings.push(ManifestWarning::IgnoredLegacyEntry { entry: legacy.value.clone() });
            }
            return Ok(Self { plugin_id, mode: ManifestMode::Entries(entries), warnings });
        }

        // 没有 [entries] 时退回旧单入口；两者都没有则视为清单不完整。
        let Some(legacy) = legacy
        else
        {
            return Err(ManifestError::new(1, "缺少 [entries] 节，也没有旧格式 entry = …"));
        };
        if legacy.value.is_empty()
        {
            return Err(ManifestError::new(legacy.line, "旧格式 entry 为空"));
        }
        validate_entry_path(DisplaySlot::Tile, &legacy.value, legacy.line)?;
        let warning = ManifestWarning::LegacyEntry { entry: legacy.value.clone() };
        Ok(Self { plugin_id, mode: ManifestMode::Legacy { entry: legacy.value }, warnings: vec![warning] })
    }

    /// 取某个显示位的入口文件；未声明返回 `None`（该插件没有这个显示）。
    pub fn entry_for(&self, slot: DisplaySlot) -> Option<&str>
    {
        match &self.mode
        {
            ManifestMode::Entries(entries) => entries.get(slot),
            ManifestMode::Legacy { entry } => Some(entry), // 旧单入口同时承担三个显示位。
        }
    }

    /// 已声明的显示位。
    pub fn declared_slots(&self) -> Vec<DisplaySlot>
    {
        match &self.mode
        {
            ManifestMode::Entries(entries) => entries.declared(),
            ManifestMode::Legacy { .. } => DisplaySlot::ALL.to_vec(),
        }
    }

    /// 日志用的模式描述。
    pub fn describe_mode(&self) -> String
    {
        match &self.mode
        {
            ManifestMode::Entries(entries) => format!("[entries] 已声明 {}", entries.declared().iter().map(|slot| slot.as_str()).collect::<Vec<_>>().join(", ")),
            ManifestMode::Legacy { entry } => format!("旧单入口 {entry}"),
        }
    }
}

/// 清单里的一行有效内容。
#[derive(Clone, Debug)]
struct RawLine
{
    section: String,
    key: String,
    value: String,
    line: usize,
}

/// 把清单切成「节 + 键 + 值」；跳过空行与注释，去掉值后的行内注释。
fn raw_lines(text: &str) -> Result<Vec<RawLine>, ManifestError>
{
    let mut section = String::new(); // 当前节名（小写）。
    let mut lines = Vec::new();
    for (index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate()
    {
        let number = index + 1; // 1 起行号。
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';')
        {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|value| value.strip_suffix(']'))
        {
            section = name.trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=')
        else
        {
            return Err(ManifestError::new(number, "清单行格式无效，应为 键 = 值"));
        };
        let value = strip_inline_comment(value).trim().to_string(); // 方案 §3.6 的示例在值后写 `; 说明`。
        lines.push(RawLine { section: section.clone(), key: key.trim().to_string(), value, line: number });
    }
    Ok(lines)
}

/// 去掉值后面的行内注释：仅认「空白 + ; 或 #」，避免误伤值里的字符。
fn strip_inline_comment(value: &str) -> &str
{
    let bytes = value.as_bytes();
    for index in 0..bytes.len()
    {
        let is_marker = bytes[index] == b';' || bytes[index] == b'#';
        let preceded_by_space = index > 0 && (bytes[index - 1] == b' ' || bytes[index - 1] == b'\t');
        if is_marker && preceded_by_space
        {
            return &value[..index - 1];
        }
    }
    value
}

/// 校验入口路径：插件根内的相对脚本路径。
fn validate_entry_path(slot: DisplaySlot, value: &str, line: usize) -> Result<(), ManifestError>
{
    let mut relative = value;
    while let Some(rest) = relative.strip_prefix("./")
    {
        relative = rest; // 允许写 ./tiles/main.js，但不允许靠 . / .. 跳出插件根。
    }
    if relative.is_empty() || value.contains(['\\', ':', '%', '\0'])
    {
        return Err(ManifestError::new(line, format!("显示位 {slot} 的入口必须是插件根内、用 / 分隔的相对路径：{value}")));
    }
    let path = Path::new(relative);
    if path.is_absolute() || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ManifestError::new(line, format!("显示位 {slot} 的入口不能用绝对路径或 .. 越出插件根：{value}")));
    }
    match path.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase).as_deref()
    {
        Some("js") | Some("mjs") =>
        {}
        _ => return Err(ManifestError::new(line, format!("显示位 {slot} 的入口必须是 .js 或 .mjs 文件：{value}"))),
    }
    Ok(())
}
