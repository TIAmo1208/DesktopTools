use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, OleInitialize, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
use windows::core::BOOL;

use super::drop_content;
use super::drop_target::{self, DropPhase};
use crate::drop_bridge::{IslandDropFact, IslandDropKind};

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
        let engaged = Rc::new(Cell::new(None::<IslandDropKind>)); // 本次拖入是否进过接收路径及类型；右键保持 None。
        let handler_port = port.clone();
        let handler_session = session.clone();
        let handler_engaged = engaged.clone();
        let target = drop_target::create_drop_target(
            Some(target_window),
            Rc::new(move |phase| match phase
            {
                DropPhase::Enter(data, keys, _point, allowed) =>
                {
                    *handler_session.borrow_mut() = None;
                    handler_engaged.set(None);
                    let paths = drop_content::collect_file_paths(data).unwrap_or_default(); // 保留失效路径，由收入规则逐项处理。
                    let offer = drop_content::offer(data, &paths); // 只探测格式，不读取正文。
                    let decision = island_enter_decision(keys.0 & windows::Win32::System::SystemServices::MK_RBUTTON.0 != 0, handler_port.collect_accepting(), offer.targets.contains(&drop_content::COLLECT), allowed);
                    if let Some(fact) = decision.fact
                    {
                        handler_port.island_drop_fact(fact); // 不可接收也要让页面拿到事实，否则「无法接收」永远不可达。
                        handler_engaged.set(Some(fact.kind));
                    }
                    if decision.accepts_collect
                    {
                        let collect_names = drop_target::source_drag_session(data).and_then(|source| source.collect_names.clone());
                        *handler_session.borrow_mut() = Some(IslandDropSession { collect_names, offer }); // 数据对象只在原生回调生命周期内有效。
                    }
                    Ok(decision.effect)
                }
                DropPhase::Over(_keys, _point, allowed) => Ok(island_over_effect(handler_engaged.get(), allowed)),
                DropPhase::Drop(data, _keys, _point, allowed) =>
                {
                    let engaged = handler_engaged.replace(None); // 放下即结束会话，随后 Leave 不再重复通知。
                    let session = handler_session.borrow_mut().take();
                    if let Some(fact) = island_leave_decision(engaged)
                    {
                        handler_port.island_drop_fact(fact);
                    }
                    let has_collect = session.as_ref().is_some_and(|session| session.offer.targets.contains(&drop_content::COLLECT));
                    let decision = island_drop_decision(engaged, session.is_some(), has_collect, allowed);
                    let (Some(session), IslandDropDecision::Income) = (session, decision)
                    else
                    {
                        return Ok(DROPEFFECT_NONE); // reject：不收入、不执行、回退原内容。
                    };
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
                    handler_session.borrow_mut().take();
                    if let Some(fact) = island_leave_decision(handler_engaged.replace(None))
                    {
                        handler_port.island_drop_fact(fact);
                    }
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

/// 不可接收时回给源的有效效果：Enter 不能返回 NONE，否则 OS 不进入 Over/Drop/Leave，页面收不到事实。
fn reject_effect(allowed: DROPEFFECT) -> DROPEFFECT
{
    for candidate in [DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE]
    {
        if (allowed & candidate).0 != 0
        {
            return candidate;
        }
    }
    DROPEFFECT_COPY // 源没有声明任何效果时仍返回非 NONE，事实先于最终效果送达页面。
}

/// 一次进入的纯决策：是否进入收入路径、回给 OS 的投放效果与要广播的进入事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IslandEnterDecision
{
    pub effect: DROPEFFECT,           // 交给 OS 的投放效果。
    pub accepts_collect: bool,        // 只有收集箱内容进入收入路径。
    pub fact: Option<IslandDropFact>, // 右键不进入接收路径，也不发事件。
}

/// 进入判定：右键一律拒绝；其余拖入都返回有效效果并广播 collect 或 reject 事实。
pub(crate) fn island_enter_decision(right_button: bool, accepting: bool, offer_has_collect: bool, allowed: DROPEFFECT) -> IslandEnterDecision
{
    if right_button
    {
        return IslandEnterDecision { effect: DROPEFFECT_NONE, accepts_collect: false, fact: None };
    }
    if accepting && offer_has_collect && collect_effect(allowed) != DROPEFFECT_NONE
    {
        return IslandEnterDecision { effect: DROPEFFECT_COPY, accepts_collect: true, fact: Some(IslandDropFact { active: true, kind: IslandDropKind::Collect }) };
    }
    // 不可接收也要返回有效效果：进入先用源允许的效果，随后的 Over 立刻改报 NONE，光标表现为不可接收。
    IslandEnterDecision { effect: reject_effect(allowed), accepts_collect: false, fact: Some(IslandDropFact { active: true, kind: IslandDropKind::Reject }) }
}

/// 移动判定：只有已进入收集箱接收路径才报复制效果；reject 立即报禁止。
pub(crate) fn island_over_effect(engaged: Option<IslandDropKind>, allowed: DROPEFFECT) -> DROPEFFECT
{
    match engaged
    {
        Some(IslandDropKind::Collect) => collect_effect(allowed),
        _ => DROPEFFECT_NONE,
    }
}

/// 放下时的两种去向：走既有收入路径，或不收入并回退原内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IslandDropDecision
{
    Income,
    Ignore,
}

/// 放下判定：只有收集箱会话、格式仍含收集箱且源允许复制才收入；reject 只回退。
pub(crate) fn island_drop_decision(engaged: Option<IslandDropKind>, has_session: bool, offer_has_collect: bool, allowed: DROPEFFECT) -> IslandDropDecision
{
    if engaged == Some(IslandDropKind::Collect) && has_session && offer_has_collect && collect_effect(allowed) != DROPEFFECT_NONE
    {
        return IslandDropDecision::Income;
    }
    IslandDropDecision::Ignore
}

/// 离开或放下时补发的 inactive 事实：类型沿用本次进入，未进入接收路径（右键）不发事件。
pub(crate) fn island_leave_decision(engaged: Option<IslandDropKind>) -> Option<IslandDropFact>
{
    engaged.map(|kind| IslandDropFact { active: false, kind })
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
#[path = "island_drop.test.rs"]
mod tests;

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
