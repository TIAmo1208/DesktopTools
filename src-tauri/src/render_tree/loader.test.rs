//! `render_tree::loader` 单测：懒加载、共享上下文与模块缓存、插件根限定、错误出口。
//!
//! 全部用临时目录里的 fixture，不触碰仓库 `plugin/` 下的真实插件。

use std::path::{Path, PathBuf};

use super::error::RenderTreeError;
use super::host::{TileRender, TileRequest};
use super::loader::PluginSession;
use super::slot::DisplaySlot;

/// 临时插件目录：用完即删。
struct Fixture
{
    root: PathBuf,
}

impl Fixture
{
    /// 创建独占的临时目录（测试并行时用名字区分）。
    fn new(name: &str) -> Self
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-render-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建临时插件目录失败");
        Self { root }
    }

    /// 写入一个 fixture 文件（自动建上级目录）。
    fn write(&self, relative: &str, content: &str) -> &Self
    {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent).expect("创建 fixture 目录失败");
        }
        std::fs::write(path, content).expect("写入 fixture 失败");
        self
    }

    /// 插件根。
    fn root(&self) -> &Path { &self.root }

    /// 打开会话。
    fn open(&self, manifest: &str) -> Result<PluginSession, RenderTreeError> { PluginSession::open(self.root(), "manifest.ini", manifest) }
}

impl Drop for Fixture
{
    /// 删除临时目录。
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

/// 新格式三显示位清单。
const THREE_SLOTS: &str = "[plugin]\nid = plugin.demo\n\n[entries]\ntile = tile/main.js\nisland = island/main.js\nsettings = settings/main.js\n";

/// 只有格子的清单。
const TILE_ONLY: &str = "[plugin]\nid = plugin.demo\n\n[entries]\ntile = tile/main.js\n";

/// 渲染请求。
fn request() -> TileRequest { TileRequest::new("tile-1", "status") }

/// 取树里的文本。
fn text_of(render: &TileRender) -> String
{
    match render
    {
        TileRender::Tree { tree, .. } => tree.text.clone().unwrap_or_default(),
        other => panic!("应渲染出树：{other:?}"),
    }
}

/// 打开会话不读脚本；只加载请求的显示位，缺失文件只在真加载时暴露。
#[test]
fn loads_only_requested_slot_lazily()
{
    let fixture = Fixture::new("lazy");
    fixture.write("tile/main.js", "export default function render() { return { type: 'text', text: 'tile' }; }\n");
    let session = fixture.open(THREE_SLOTS).expect("清单应通过");

    assert!(session.loaded_slots().is_empty(), "打开会话不应加载任何模块");
    session.load(DisplaySlot::Tile).expect("格子应加载成功");
    assert_eq!(session.loaded_slots(), vec![DisplaySlot::Tile]);
    assert_eq!(text_of(&session.render(DisplaySlot::Tile, &request()).expect("渲染应成功")), "tile");

    // island/main.js 在清单里声明但没有落盘：直到请求才报错，且带文件。
    match session.load(DisplaySlot::Island).expect_err("缺失文件应报错")
    {
        RenderTreeError::EntryFile { slot, file, detail } =>
        {
            assert_eq!(slot, DisplaySlot::Island);
            assert_eq!(file, "island/main.js");
            assert!(detail.contains("文件不存在"), "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
    assert_eq!(session.loaded_slots(), vec![DisplaySlot::Tile], "失败的加载不应记入已加载");
}

/// 同一实例共享 JS 上下文，且同一模块只求值一次。
#[test]
fn shares_context_and_evaluates_once()
{
    let fixture = Fixture::new("shared");
    fixture.write("tile/main.js", "globalThis.__tileInit = (globalThis.__tileInit ?? 0) + 1;\nglobalThis.__marker = 'shared' + globalThis.__tileInit;\nexport default function render() { return { type: 'text', text: 'tile:' + globalThis.__tileInit + ':' + globalThis.__marker }; }\n");
    fixture.write("island/main.js", "export default function render() { return { type: 'text', text: 'island:' + (globalThis.__marker ?? 'none') + ':' + (globalThis.__tileInit ?? 0) }; }\n");
    let session = fixture.open(THREE_SLOTS).expect("清单应通过");

    let first = text_of(&session.render(DisplaySlot::Tile, &request()).expect("渲染应成功"));
    let second = text_of(&session.render(DisplaySlot::Tile, &request()).expect("渲染应成功"));
    assert_eq!(first, "tile:1:shared1", "模块应只求值一次");
    assert_eq!(second, first);

    let island = text_of(&session.render(DisplaySlot::Island, &request()).expect("渲染应成功"));
    assert_eq!(island, "island:shared1:1", "浮岛应看到同一上下文里的全局状态");
}

/// 相对导入按插件根解析，且同一公共模块跨显示位只求值一次。
#[test]
fn resolves_relative_import_once_across_slots()
{
    let fixture = Fixture::new("import");
    fixture.write("shared/util.js", "globalThis.__utilInit = (globalThis.__utilInit ?? 0) + 1;\nexport function label() { return 'util' + globalThis.__utilInit; }\n");
    fixture.write("tile/main.js", "import { label } from '../shared/util.js';\nexport default function render() { return { type: 'text', text: label() }; }\n");
    fixture.write("island/main.js", "import { label } from '../shared/util.js';\nexport default function render() { return { type: 'text', text: label() + '/island' }; }\n");
    let session = fixture.open(THREE_SLOTS).expect("清单应通过");

    assert_eq!(text_of(&session.render(DisplaySlot::Tile, &request()).expect("渲染应成功")), "util1");
    assert_eq!(text_of(&session.render(DisplaySlot::Island, &request()).expect("渲染应成功")), "util1/island", "公共模块应被缓存复用");
}

/// `../` 越出插件根被拒绝；越界目标即使存在也不会被读取。
#[test]
fn rejects_import_escaping_plugin_root()
{
    let fixture = Fixture::new("escape");
    fixture.write("outside.js", "export const value = ; // 故意语法错误：若被读取会先报语法错误\n");
    fixture.write("tile/main.js", "import { value } from '../../outside.js';\nexport default function render() { return { type: 'text', text: String(value) }; }\n");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");

    match session.load(DisplaySlot::Tile).expect_err("越界导入应报错")
    {
        RenderTreeError::EntryImport { slot, file, detail } =>
        {
            assert_eq!(slot, DisplaySlot::Tile);
            assert_eq!(file, "tile/main.js");
            assert!(detail.contains("越出插件根"), "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 裸模块名（没有 ./ 前缀）被拒绝：插件不能从 node_modules 或网络路径取模块。
#[test]
fn rejects_bare_module_specifier()
{
    let fixture = Fixture::new("bare");
    fixture.write("tile/main.js", "import 'lodash';\nexport default function render() { return { type: 'text', text: 'x' }; }\n");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");

    match session.load(DisplaySlot::Tile).expect_err("裸模块名应报错")
    {
        RenderTreeError::EntryImport { detail, .. } => assert!(detail.contains("只允许相对路径"), "{detail}"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 语法错误带文件与行列。
#[test]
fn reports_syntax_error_with_file_and_position()
{
    let fixture = Fixture::new("syntax");
    fixture.write("tile/main.js", "export default function render() {\n    const value = ;\n}\n");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");

    match session.load(DisplaySlot::Tile).expect_err("语法错误应报错")
    {
        RenderTreeError::EntrySyntax { slot, file, line, column, detail } =>
        {
            assert_eq!(slot, DisplaySlot::Tile);
            assert_eq!(file, "tile/main.js");
            assert_eq!(line, 2, "{detail}");
            assert!(column >= 1, "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 入口导出缺失：带文件与导出名。
#[test]
fn reports_missing_export_with_file()
{
    let fixture = Fixture::new("export");
    fixture.write("tile/main.js", "export const other = 1;\n");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");

    match session.load(DisplaySlot::Tile).expect_err("缺入口导出应报错")
    {
        RenderTreeError::EntryExport { slot, file, entry } =>
        {
            assert_eq!(slot, DisplaySlot::Tile);
            assert_eq!(file, "tile/main.js");
            assert_eq!(entry, "default");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 返回非树：带文件与字段路径，说明具体哪里不合法。
#[test]
fn reports_non_tree_return()
{
    let fixture = Fixture::new("nontree");
    fixture.write("tile/main.js", "export default function render() { return { type: 'canvas' }; }\n");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");

    match session.render(DisplaySlot::Tile, &request()).expect_err("非树返回值应报错")
    {
        RenderTreeError::EntryValue { slot, file, detail } =>
        {
            assert_eq!(slot, DisplaySlot::Tile);
            assert_eq!(file, "tile/main.js");
            assert!(detail.contains("未知的渲染树节点类型"), "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 设置位是 DOM 契约：可以加载，但不能按渲染树渲染。
#[test]
fn settings_slot_loads_but_is_not_a_tree()
{
    let fixture = Fixture::new("settings");
    fixture.write("settings/main.js", "export function render() { return null; }\n");
    let session = fixture.open(THREE_SLOTS).expect("清单应通过");

    session.load(DisplaySlot::Settings).expect("设置位应加载成功");
    match session.render(DisplaySlot::Settings, &request()).expect_err("设置位不能按树渲染")
    {
        RenderTreeError::SlotNotTree { slot, file } =>
        {
            assert_eq!(slot, DisplaySlot::Settings);
            assert_eq!(file, "settings/main.js");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 未声明的显示位：不加载，明确报未声明。
#[test]
fn undeclared_slot_is_not_loaded()
{
    let fixture = Fixture::new("undeclared");
    let session = fixture.open(TILE_ONLY).expect("清单应通过");
    assert_eq!(session.load(DisplaySlot::Island).expect_err("未声明应报错"), RenderTreeError::SlotNotDeclared { slot: DisplaySlot::Island });
    assert_eq!(session.render(DisplaySlot::Settings, &request()).expect_err("未声明应报错"), RenderTreeError::SlotNotDeclared { slot: DisplaySlot::Settings });
    assert!(session.loaded_slots().is_empty());
}

/// 旧单入口：可加载并带兼容警告；没有渲染树导出时按树渲染会明确失败。
#[test]
fn legacy_entry_loads_with_warning_but_has_no_tree()
{
    let fixture = Fixture::new("legacy");
    fixture.write("index.js", "export function renderTile() {}\nexport const settingsEntries = [];\n");
    let session = fixture.open("[plugin]\nid = plugin.old\nentry = index.js\n").expect("旧清单应通过");
    assert_eq!(session.manifest().warnings.len(), 1);
    assert!(session.manifest().warnings[0].to_string().contains("旧单入口"));

    session.load(DisplaySlot::Tile).expect("旧入口的 renderTile 应被接受");
    session.load(DisplaySlot::Settings).expect("旧入口的 settingsEntries 应被接受");
    match session.render(DisplaySlot::Tile, &request()).expect_err("旧入口没有渲染树导出")
    {
        RenderTreeError::EntryExport { entry, .. } => assert_eq!(entry, "default"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 打开会话只解析清单，不编译模块：语法错误在加载时才暴露。
#[test]
fn open_does_not_compile_modules()
{
    let fixture = Fixture::new("open");
    fixture.write("tile/main.js", "export default function render( {\n");
    let session = fixture.open(TILE_ONLY).expect("清单解析不应触发模块编译");
    assert!(session.loaded_slots().is_empty());
    assert!(matches!(session.load(DisplaySlot::Tile).expect_err("加载时才报语法错误"), RenderTreeError::EntrySyntax { .. }));
}

/// 插件根不存在时报明确错误。
#[test]
fn reports_missing_plugin_root()
{
    let fixture = Fixture::new("root");
    let missing = fixture.root().join("not-a-plugin");
    let error = match PluginSession::open(&missing, "manifest.ini", TILE_ONLY)
    {
        Ok(_) => panic!("根目录不存在应报错"),
        Err(error) => error,
    };
    match error
    {
        RenderTreeError::Root { file, detail } =>
        {
            assert!(file.contains("not-a-plugin"), "{file}");
            assert!(!detail.is_empty());
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}
