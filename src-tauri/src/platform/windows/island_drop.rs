use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, OleInitialize, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
use windows::core::BOOL;

use super::drop_content;
use super::drop_target::{self, DropPhase};

/// 浮岛只接收收集箱内容，不持有工作区布局或投放预览。
pub(crate) struct IslandDropTargets
{
    targets: Vec<(isize, IDropTarget)>, // 根窗口与 WebView2 后代窗口的已注册目标。
    ole: bool,                          // 当前线程是否由浮岛配对初始化 OLE。
}

/// 当前 OLE 会话在进入时冻结的源内容摘要。
struct IslandDropSession
{
    collect_names: Option<Vec<String>>, // 本程序收集项身份；外部对象不能伪造。
    offer: drop_content::ContentOffer,  // 进入时固定的可接收类型。
}

/// 浮岛窗口和 WebView2 子窗口共同注册 Shell 投放目标。
pub(crate) fn register(hwnd: HWND, port: Arc<dyn crate::drop_bridge::IslandDropPort>) -> Result<IslandDropTargets, String>
{
    let ole = initialize_ole()?; // 先确保当前 UI 线程可注册 OLE 目标。
    let mut windows = vec![hwnd]; // 根窗口始终参与，即使没有子窗口。
    unsafe extern "system" fn collect_child(window: HWND, parameter: LPARAM) -> BOOL
    {
        let windows = unsafe { &mut *(parameter.0 as *mut Vec<HWND>) }; // 调用方持有的窗口集合。
        windows.push(window);
        BOOL(1)
    }
    let _ = unsafe { EnumChildWindows(Some(hwnd), Some(collect_child), LPARAM((&mut windows as *mut Vec<HWND>) as isize)) };
    let mut targets = Vec::new(); // 必须保持到浮岛销毁的 COM 投放目标。
    for target_window in windows
    {
        let port = port.clone(); // 每个窗口共用同一应用层端口。
        let session = Rc::new(RefCell::new(None::<IslandDropSession>)); // 本次原生会话的进入事实。
        let accepted = Rc::new(Cell::new(false)); // 是否允许当前源进入接收路径。
        let handler_port = port.clone();
        let handler_session = session.clone();
        let handler_accepted = accepted.clone();
        let target = drop_target::create_drop_target(
            Some(target_window),
            Rc::new(move |phase| match phase
            {
                DropPhase::Enter(data, keys, _point, allowed) =>
                {
                    *handler_session.borrow_mut() = None;
                    handler_accepted.set(false);
                    if keys.0 & windows::Win32::System::SystemServices::MK_RBUTTON.0 != 0
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    if !handler_port.collect_accepting()
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    let paths = drop_content::collect_file_paths(data).unwrap_or_default(); // 保留失效路径，由收入规则逐项处理。
                    let offer = drop_content::offer(data, &paths); // 只探测格式，不读取正文。
                    if !offer.targets.contains(&drop_content::COLLECT)
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    let collect_names = drop_target::source_drag_session(data).and_then(|source| source.collect_names.clone());
                    let selected = collect_effect(allowed); // 收入只表达复制，绝不向源端报告移动。
                    if selected == DROPEFFECT_NONE
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    *handler_session.borrow_mut() = Some(IslandDropSession { collect_names, offer });
                    handler_accepted.set(true);
                    Ok(selected)
                }
                DropPhase::Over(_keys, _point, allowed) =>
                {
                    if !handler_accepted.get()
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    Ok(collect_effect(allowed))
                }
                DropPhase::Drop(data, _keys, _point, allowed) =>
                {
                    if !handler_accepted.replace(false)
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    let Some(session) = handler_session.borrow_mut().take()
                    else
                    {
                        return Ok(DROPEFFECT_NONE);
                    };
                    let selected = collect_effect(allowed);
                    if selected == DROPEFFECT_NONE || !session.offer.targets.contains(&drop_content::COLLECT)
                    {
                        return Ok(DROPEFFECT_NONE);
                    }
                    let content = if session.collect_names.is_some()
                    {
                        crate::collect::IncomingContent::Files(Vec::new()) // 内部收集项只按登记身份复制，不把数据对象再当成外部文件。
                    }
                    else
                    {
                        let paths = drop_content::collect_file_paths(data).unwrap_or_default();
                        match drop_content::extract(data, paths, &session.offer)
                        {
                            Ok(content) => content,
                            Err(error) =>
                            {
                                handler_port.report_error(error);
                                return Ok(DROPEFFECT_NONE);
                            }
                        }
                    }; // 数据对象只在原生回调生命周期内有效。
                    let (sender, receiver) = std::sync::mpsc::channel(); // 后台收入完成后回到 OLE 消息泵。
                    let worker_port = handler_port.clone();
                    let collect_names = session.collect_names.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let _ = sender.send(worker_port.receive_collect(content, "copy".into(), collect_names));
                    });
                    match drop_target::wait_transfer(receiver)
                    {
                        Ok(received) if received.processed > 0 => Ok(DROPEFFECT_COPY),
                        Ok(_) => Ok(DROPEFFECT_NONE),
                        Err(error) =>
                        {
                            handler_port.report_error(error);
                            Ok(DROPEFFECT_NONE)
                        }
                    }
                }
                DropPhase::Leave =>
                {
                    *handler_session.borrow_mut() = None;
                    handler_accepted.set(false);
                    Ok(DROPEFFECT_NONE)
                }
            }),
        );
        let _ = unsafe { RevokeDragDrop(target_window) }; // 复用窗口时先撤销旧目标，避免重复注册失败。
        match unsafe { RegisterDragDrop(target_window, &target) }
        {
            Ok(()) => targets.push((target_window.0 as isize, target)),
            Err(error) => crate::log_warn!("浮岛：拖放目标注册失败，窗口句柄={:#x}，错误：{error}", target_window.0 as isize),
        }
    }
    if targets.is_empty()
    {
        if ole
        {
            unsafe { windows::Win32::System::Ole::OleUninitialize() };
        }
        return Err("浮岛没有可用的拖放目标".into());
    }
    crate::log_debug!("浮岛：已注册拖放目标，窗口数量={}", targets.len());
    Ok(IslandDropTargets { targets, ole })
}

/// 收集箱收纳不删除源内容，只协商复制效果。
fn collect_effect(allowed: DROPEFFECT) -> DROPEFFECT
{
    if (allowed & DROPEFFECT_COPY).0 != 0
    {
        DROPEFFECT_COPY
    }
    else
    {
        DROPEFFECT_NONE
    }
}

/// 已初始化的 UI 线程不重复初始化，避免销毁时误减宿主 OLE 引用。
fn initialize_ole() -> Result<bool, String>
{
    use windows::Win32::System::Com::{APTTYPE, APTTYPEQUALIFIER, CoGetApartmentType};
    let mut apartment = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    if unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.is_ok()
    {
        return Ok(false);
    }
    unsafe { OleInitialize(None) }.map_err(|error| format!("初始化浮岛 OLE 拖放失败：{error}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 收集箱收入不能向源端报告移动，只接受源允许的复制效果。
    #[test]
    fn collect_effect_only_reports_copy()
    {
        assert_eq!(collect_effect(DROPEFFECT_COPY), DROPEFFECT_COPY);
        assert_eq!(collect_effect(DROPEFFECT_COPY | windows::Win32::System::Ole::DROPEFFECT_MOVE), DROPEFFECT_COPY);
        assert_eq!(collect_effect(windows::Win32::System::Ole::DROPEFFECT_MOVE), DROPEFFECT_NONE);
        assert_eq!(collect_effect(DROPEFFECT_NONE), DROPEFFECT_NONE);
    }
}

impl Drop for IslandDropTargets
{
    /// 释放窗口投放目标；WebView2 已销毁时撤销失败只记调试日志。
    fn drop(&mut self)
    {
        for (hwnd, target) in self.targets.drain(..)
        {
            let window = HWND(hwnd as *mut _);
            let _ = unsafe { RevokeDragDrop(window) };
            drop(target);
        }
        if self.ole
        {
            unsafe { windows::Win32::System::Ole::OleUninitialize() };
        }
    }
}
