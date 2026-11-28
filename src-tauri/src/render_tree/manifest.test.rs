//! `render_tree::manifest` 单测：三显示位声明、旧入口兼容、路径越界与非法声明。

use super::manifest::{ManifestMode, ManifestWarning, PluginManifest};
use super::slot::DisplaySlot;

/// 新格式：三个显示位都声明，值后带方案 §3.6 示例里的行内注释。
const NEW_FORMAT: &str = r#"
# 演示插件
[plugin]
id = plugin.demo
name = 演示
version = 1.0.0

[entries]
tile = tiles/main.js        ; 渲染树（原生）
island = island/main.js     ; 渲染树（原生）
settings = settings/main.js ; DOM（设置窗是 WebView）
"#;

/// 只声明一个显示位的新格式。
const TILE_ONLY: &str = "[plugin]\nid = plugin.demo\n\n[entries]\ntile = tiles/main.js\n";

/// 解析一段 `[entries]` 文本。
fn parse_entries(entries: &str) -> Result<PluginManifest, super::manifest::ManifestError> { PluginManifest::parse(&format!("[plugin]\nid = plugin.demo\n\n[entries]\n{entries}\n")) }

/// 新格式：三个显示位逐一取到入口文件，且没有兼容警告。
#[test]
fn parses_new_format_for_all_three_slots()
{
    let manifest = PluginManifest::parse(NEW_FORMAT).expect("应解析成功");
    assert_eq!(manifest.plugin_id.as_deref(), Some("plugin.demo"));
    assert_eq!(manifest.entry_for(DisplaySlot::Tile), Some("tiles/main.js"));
    assert_eq!(manifest.entry_for(DisplaySlot::Island), Some("island/main.js"));
    assert_eq!(manifest.entry_for(DisplaySlot::Settings), Some("settings/main.js"));
    assert_eq!(manifest.declared_slots(), DisplaySlot::ALL.to_vec());
    assert!(manifest.warnings.is_empty(), "{:?}", manifest.warnings);
    assert!(matches!(manifest.mode, ManifestMode::Entries(_)));
}

/// 未声明的显示位 = 没有这个显示。
#[test]
fn undeclared_slot_has_no_entry()
{
    let manifest = PluginManifest::parse(TILE_ONLY).expect("应解析成功");
    assert_eq!(manifest.entry_for(DisplaySlot::Tile), Some("tiles/main.js"));
    assert_eq!(manifest.entry_for(DisplaySlot::Island), None);
    assert_eq!(manifest.entry_for(DisplaySlot::Settings), None);
    assert_eq!(manifest.declared_slots(), vec![DisplaySlot::Tile]);
}

/// 旧单入口清单：可加载，但必须留下兼容警告；三个显示位都指向同一入口。
#[test]
fn legacy_entry_is_accepted_with_warning()
{
    let manifest = PluginManifest::parse("[plugin]\nid = plugin.old\nentry = index.js\n").expect("应解析成功");
    assert_eq!(manifest.mode, ManifestMode::Legacy { entry: "index.js".to_string() });
    assert_eq!(manifest.warnings, vec![ManifestWarning::LegacyEntry { entry: "index.js".to_string() }]);
    for slot in DisplaySlot::ALL
    {
        assert_eq!(manifest.entry_for(slot), Some("index.js"));
    }
    assert!(manifest.warnings[0].to_string().contains("旧单入口"));
}

/// 同时声明 `[entries]` 与 `entry`：以新格式为准并警告忽略旧入口。
#[test]
fn entries_take_precedence_over_legacy_entry()
{
    let manifest = PluginManifest::parse("[plugin]\nentry = index.js\n\n[entries]\ntile = tiles/main.js\n").expect("应解析成功");
    assert_eq!(manifest.entry_for(DisplaySlot::Tile), Some("tiles/main.js"));
    assert_eq!(manifest.entry_for(DisplaySlot::Island), None);
    assert_eq!(manifest.warnings, vec![ManifestWarning::IgnoredLegacyEntry { entry: "index.js".to_string() }]);
}

/// 非法位名直接拒绝。
#[test]
fn rejects_unknown_display_slot()
{
    let error = parse_entries("overlay = overlay/main.js").expect_err("应报错");
    assert!(error.detail.contains("未知的显示位"), "{error}");
    assert_eq!(error.line, 5);
}

/// 绝对路径、盘符与反斜杠一律拒绝。
#[test]
fn rejects_absolute_paths()
{
    for value in ["/abs/main.js", "C:/abs/main.js", "\\\\server\\share\\main.js", "C:\\tiles\\main.js"]
    {
        let error = parse_entries(&format!("tile = {value}")).expect_err("应报错");
        assert!(error.detail.contains("相对路径") || error.detail.contains("插件根"), "{value}: {error}");
    }
}

/// `..` 越出插件根一律拒绝（包括中间段）。
#[test]
fn rejects_parent_traversal()
{
    for value in ["../main.js", "a/../../main.js", "./../main.js"]
    {
        let error = parse_entries(&format!("tile = {value}")).expect_err("应报错");
        assert!(error.detail.contains("越出插件根"), "{value}: {error}");
    }
}

/// 同一显示位重复声明直接拒绝。
#[test]
fn rejects_duplicate_slot_declaration()
{
    let error = parse_entries("tile = a.js\ntile = b.js").expect_err("应报错");
    assert!(error.detail.contains("重复声明"), "{error}");
}

/// 一个文件只能实现一个显示位。
#[test]
fn rejects_shared_file_between_slots()
{
    let error = parse_entries("tile = tiles/main.js\nisland = tiles/main.js").expect_err("应报错");
    assert!(error.detail.contains("一个文件只能实现一个显示位"), "{error}");
}

/// 既没有 `[entries]` 也没有旧 `entry`：清单不完整。
#[test]
fn rejects_manifest_without_entries_or_entry()
{
    let error = PluginManifest::parse("[plugin]\nid = plugin.demo\n").expect_err("应报错");
    assert!(error.detail.contains("缺少 [entries]"), "{error}");
}

/// 空值与非脚本扩展名都拒绝。
#[test]
fn rejects_empty_value_and_non_script_extension()
{
    let empty = parse_entries("tile = ; 只有注释").expect_err("应报错");
    assert!(empty.detail.contains("入口文件为空"), "{empty}");
    let extension = parse_entries("tile = tiles/main.css").expect_err("应报错");
    assert!(extension.detail.contains(".js 或 .mjs"), "{extension}");
}

/// BOM 与 `./` 前缀都能正确处理。
#[test]
fn accepts_bom_and_dot_prefix()
{
    let manifest = PluginManifest::parse("\u{feff}[plugin]\nentry = ./index.js\n").expect("应解析成功");
    assert_eq!(manifest.entry_for(DisplaySlot::Tile), Some("./index.js"));
}
