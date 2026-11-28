//! 按实例授予开窗能力；原生 label 绑定不可替换的视图和设置归属。窗口不进入工作区注册表。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, atomic::{AtomicU64, Ordering}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{ComponentCapability, ComponentManifest, ComponentRole};
use crate::preferences::SettingsState;

pub(crate) const PROTOCOL: &str = "desktop-tools-plugin";
pub(crate) const ORIGIN: &str = "http://desktop-tools-plugin.localhost";
const PAGE: &str = "http://desktop-tools-plugin.localhost/?plugin-window=1";
const LABEL_PREFIX: &str = "plugin-window-";

/// 开窗与文件操作分开串行；原生事件、协议资产读取不等待正在建窗的 WebView。
static OPEN: Mutex<()> = Mutex::new(());
pub(super) static LIFECYCLE: Mutex<()> = Mutex::new(());
static WINDOWS: LazyLock<Mutex<HashMap<String, Binding>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_LABEL: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Size { width: u32, height: u32 }

/// 请求没有路径、URL、脚本字段；data 只是交给入口的 JSON，不参与导航或代码生成。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request { view: String, window_key: String, title: String, size: Size, min_size: Option<Size>, data: Option<Value> }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Context { plugin_id: String, instance_id: String, script: String, stylesheet: String, view: String, window_key: String, data: Option<Value> }

/// guard 只在作者的草稿关闭监听装好后启用；错误页和启动中的页面可直接退出。
struct Binding { context: Context, author: String, close_guard: bool }

fn valid_key(value: &str, limit: usize) -> bool { !value.is_empty() && value.len() <= limit && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b':')) }

impl Request {
    fn validate(&self) -> Result<(), String> {
        if !valid_key(&self.view,64) || !valid_key(&self.window_key,128) { return Err("插件视图或窗口键无效".into()); }
        if self.title.trim().is_empty() || self.title.chars().count()>128 || self.title.chars().any(char::is_control) { return Err("插件窗口标题无效".into()); }
        let valid = |size: Size| (1..=16384).contains(&size.width) && (1..=16384).contains(&size.height);
        if !valid(self.size) || self.min_size.is_some_and(|size| !valid(size) || size.width>self.size.width || size.height>self.size.height) { return Err("插件窗口尺寸无效".into()); }
        if let Some(data)=&self.data && serde_json::to_vec(data).map_err(|e|e.to_string())?.len()>65536 { return Err("插件窗口初始数据过大".into()); }
        Ok(())
    }
}

pub(crate) fn is_window(label: &str) -> bool { label.starts_with(LABEL_PREFIX) }

fn permitted_manifest(app: &AppHandle, id: &str) -> Result<ComponentManifest,String> {
    let manifest=app.state::<ComponentRuntimeState>().manifest(id).ok_or("插件清单不存在")?;
    if app.state::<SettingsState>().preferences()?.disabled_plugins.iter().any(|item| item.eq_ignore_ascii_case(id)) || manifest.role!=ComponentRole::Tile || manifest.entry!="plugin:slot" || manifest.script.is_empty() || !manifest.required_capabilities.contains(&ComponentCapability::PluginWindow) { return Err("插件未启用或未声明窗口能力".into()); }
    Ok(manifest)
}

fn instance_manifest(app:&AppHandle, instance_id:&str) -> Result<ComponentManifest,String> {
    let runtime=app.state::<ComponentRuntimeState>();
    let instances=super::component::merged_instances(&runtime,&app.state::<crate::workspace::WorkspaceState>(),&app.state::<SettingsState>()).map_err(|e|e.message)?;
    let registration=runtime.authorize_capability_on(&instances,instance_id,ComponentCapability::PluginWindow).map_err(|e|e.message)?;
    permitted_manifest(app,&registration.component_id)
}

/// 跨呈现位只按插件、视图和内容键唤回；不比较或覆盖旧请求的标题、尺寸、草稿或初始数据。
fn existing(windows:&HashMap<String,Binding>, id:&str, request:&Request) -> Option<String> {
    windows.iter().find(|(_,b)| b.context.plugin_id==id && b.context.view==request.view && b.context.window_key==request.window_key).map(|(label,_)|label.clone())
}

pub(crate) fn open(app:&AppHandle, instance_id:String, request:Request) -> Result<(),String> {
    request.validate()?;
    let _opening=OPEN.lock().map_err(|_|"插件开窗状态不可用")?;
    let label={
        let _lifecycle=LIFECYCLE.lock().map_err(|_|"插件目录状态不可用")?;
        let manifest=instance_manifest(app,&instance_id)?;
        let mut windows=WINDOWS.lock().map_err(|_|"插件窗口状态不可用")?;
        if let Some(label)=existing(&windows,&manifest.id,&request) && let Some(window)=app.get_webview_window(&label) {
            drop(windows); drop(_lifecycle);
            return window.unminimize().and_then(|_|window.show()).and_then(|_|window.set_focus()).map_err(|e|e.to_string());
        }
        let label=format!("{LABEL_PREFIX}{}",NEXT_LABEL.fetch_add(1,Ordering::Relaxed));
        let context=Context{plugin_id:manifest.id,instance_id,script:manifest.script,stylesheet:manifest.stylesheet,view:request.view,window_key:request.window_key,data:request.data};
        windows.insert(label.clone(),Binding{context,author:manifest.author,close_guard:false});
        label
    };
    let opened=(|| {
        let url=tauri::Url::parse(PAGE).expect("固定插件页面地址有效");
        let window=WebviewWindowBuilder::new(app,&label,WebviewUrl::CustomProtocol(url)).title(request.title.trim()).inner_size(request.size.width.into(),request.size.height.into()).prevent_overflow().visible(false).on_navigation(|url|url.as_str()==PAGE).on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).build().map_err(|e|e.to_string())?;
        let setup=(|| {
            // 用实际 Windows 装饰边框和当前显示器工作区，不能把整屏或物理像素直接当客户区 DIP。
            fit_window(&window,request.size,request.min_size)?;
            if !WINDOWS.lock().map_err(|_|"插件窗口状态不可用")?.contains_key(&label) { return Err("插件在开窗期间已停用或卸载".into()); }
            window.show().and_then(|_|window.set_focus()).map_err(|e|e.to_string())
        })();
        if setup.is_err() { let _=window.destroy(); }
        setup
    })();
    if opened.is_err() { forget(&label); }
    opened
}

/// 可用工作区扣除非客户区后同时限制期望与最小尺寸，并在此工作区居中首次打开。
fn fit_window(window:&WebviewWindow, size:Size, minimum:Option<Size>) -> Result<(),String> {
    let err=|e:tauri::Error|e.to_string();
    let monitor=window.current_monitor().map_err(err)?.ok_or("找不到窗口显示器")?;
    let work=monitor.work_area();
    let scale=monitor.scale_factor();
    let outer=window.outer_size().map_err(err)?;
    let inner=window.inner_size().map_err(err)?;
    let frame=(outer.width.saturating_sub(inner.width),outer.height.saturating_sub(inner.height));
    let max=Size{width:((work.size.width.saturating_sub(frame.0) as f64/scale).floor() as u32).max(1),height:((work.size.height.saturating_sub(frame.1) as f64/scale).floor() as u32).max(1)};
    let fit=|s:Size|tauri::LogicalSize::new(s.width.min(max.width) as f64,s.height.min(max.height) as f64);
    window.set_min_size(minimum.map(fit)).map_err(err)?;
    window.set_size(fit(size)).map_err(err)?;
    let outer=window.outer_size().map_err(err)?;
    window.set_position(tauri::PhysicalPosition::new(work.position.x+(work.size.width.saturating_sub(outer.width)/2) as i32,work.position.y+(work.size.height.saturating_sub(outer.height)/2) as i32)).map_err(err)
}

fn forget(label:&str) { WINDOWS.lock().unwrap_or_else(|e|e.into_inner()).remove(label); }

pub(crate) fn window_event(app:&AppHandle, label:&str, event:&tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::Destroyed=>forget(label),
        tauri::WindowEvent::CloseRequested{api,..} if WINDOWS.lock().is_ok_and(|windows|windows.get(label).is_some_and(|b|b.close_guard)) => {
            if let Some(window)=app.get_webview_window(label) && window.eval("window.dispatchEvent(new Event('plugin-window-close-requested'))").is_ok() { api.prevent_close(); }
        }
        _=>{}
    }
}

/// 先撤销注册，再在锁外销毁（原生回调可重入）；建窗中的 label 也会在显示前复核。
pub(super) fn revoke(app:&AppHandle, matches:impl Fn(&str)->bool) {
    let labels={
        let mut windows=WINDOWS.lock().unwrap_or_else(|e|e.into_inner());
        let labels=windows.iter().filter(|(_,b)|matches(&b.context.plugin_id)).map(|(label,_)|label.clone()).collect::<Vec<_>>();
        for label in &labels { windows.remove(label); }
        labels
    };
    for label in labels { if let Some(window)=app.get_webview_window(&label) && let Err(error)=window.destroy() { crate::log_warn!("插件窗口：销毁失败，窗口={label}，错误={error}"); } }
}

pub(crate) fn asset_folder(label:&str) -> Option<String> { WINDOWS.lock().ok()?.get(label)?.context.script.split_once('/').map(|(folder,_)|folder.to_owned()) }

/// Tauri 主设置页与桌面/岛的实例桥在这里再次确认身份；绝不使用调用方提供的作者名。
pub(super) fn settings_action(app:&AppHandle,id:&str,instance:Option<&str>,update:Option<(&Value,&Value)>)->Result<Value,String> {
    let _transaction=LIFECYCLE.lock().map_err(|_|"插件目录状态不可用")?;
    let manifest=match instance { Some(instance)=>instance_manifest(app,instance)?,None=>permitted_manifest(app,id)? };
    if manifest.id!=id {return Err("设置不属于当前插件实例".into());}
    settings_for(app,&manifest,update)
}

fn settings_for(app:&AppHandle,manifest:&ComponentManifest,update:Option<(&Value,&Value)>)->Result<Value,String> {
    let directory=app.state::<SettingsState>().directory()?;
    if let Some((expected,value))=update {
        let saved=crate::preferences::update_plugin_settings(&directory,&manifest.author,&manifest.id,Some(expected),value)?;
        if saved { super::plugin_settings::notify_settings_changed(app,&manifest.id); }
        Ok(Value::Bool(saved))
    } else {Ok(crate::preferences::read_plugin_settings_for_update(&directory,&manifest.author,&manifest.id)?.unwrap_or(Value::Null))}
}

#[tauri::command]
pub(crate) fn get_plugin_settings_for_update(app:AppHandle,window:WebviewWindow,plugin_id:String)->Result<Value,String> {
    if window.label()!="main" {return Err("只允许设置窗口调用".into());}
    settings_action(&app,&plugin_id,None,None)
}

#[tauri::command]
pub(crate) fn compare_and_swap_plugin_settings(app:AppHandle,window:WebviewWindow,plugin_id:String,expected:Value,value:Value)->Result<Value,String> {
    if window.label()!="main" {return Err("只允许设置窗口调用".into());}
    settings_action(&app,&plugin_id,None,Some((&expected,&value)))
}

/// 专用窄通道：label 只来自 Tauri 协议上下文，不来自 query、HTTP 参数或页面自报身份。
pub(crate) fn handler(app:AppHandle,label:String)->crate::command_bridge::NativeCommandHandler {
    Arc::new(move |cmd,args|dispatch(&app,&label,cmd,&args).map_err(|message|json!({"code":"plugin-window-error","message":message})))
}

fn dispatch(app:&AppHandle,label:&str,cmd:&str,args:&Value)->Result<Value,String> {
    if cmd=="close_plugin_window" { if !is_window(label) {return Err("不是插件窗口".into());} app.get_webview_window(label).ok_or("窗口已关闭")?.destroy().map_err(|e|e.to_string())?; return Ok(Value::Null); }
    let _transaction=LIFECYCLE.lock().map_err(|_|"插件目录状态不可用")?;
    let mut windows=WINDOWS.lock().map_err(|_|"插件窗口状态不可用")?;
    let binding=windows.get_mut(label).ok_or("未注册的插件窗口")?;
    let manifest=permitted_manifest(app,&binding.context.plugin_id)?;
    if manifest.author!=binding.author || manifest.script!=binding.context.script { return Err("窗口入口已失效".into()); }
    match cmd {
        "get_plugin_window_context"=>serde_json::to_value(&binding.context).map_err(|e|e.to_string()),
        "set_plugin_window_close_guard"=> { binding.close_guard=args["enabled"].as_bool().ok_or("关闭确认参数无效")?; Ok(Value::Null) },
        "get_preferences"=>serde_json::to_value(app.state::<SettingsState>().preferences()?).map_err(|e|e.to_string()),
        "get_windows_accent_color"=>app.state::<crate::platform::windows::settings::WindowsAccentMonitor>().color().map(Value::String),
        "get_plugin_settings"|"get_plugin_settings_for_update"|"compare_and_swap_plugin_settings"=>{
            if args["pluginId"].as_str()!=Some(manifest.id.as_str()) {return Err("设置不属于当前插件窗口".into());}
            let update=if cmd=="compare_and_swap_plugin_settings" { Some((args.get("expected").ok_or("缺少设置基线")?,args.get("value").ok_or("缺少新设置")?)) } else {None};
            drop(windows);
            settings_for(app,&manifest,update)
        },
        _=>Err("该命令不适用于插件窗口".into())
    }
}

#[cfg(test)]
#[path="plugin_window.test.rs"]
mod tests;
