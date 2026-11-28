//! 限定窗口的真实消息诊断；不注册全局钩子，不接管 Explorer，也不注入输入。
use super::{
    draw::{Color, DrawCommand, DrawList, Rect},
    geometry::{DpiScale, HostGeometry, PixelRect},
    host::{ComApartment, DemoWindows, Devices, SurfaceHost},
    input::{FocusAction, FocusState, HitEntry, HitScene, identity_scene},
    painter::Painter,
    workspace_scene::{draw_with_focus, intersect, layout_fingerprint, read_scene, scene_hits},
};
use crate::contracts::{WorkspaceSnapshot, settings::ApplicationPreferences};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{E_FAIL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{ClientToScreen, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{GetFocus, GetKeyState, VK_SHIFT},
            WindowsAndMessaging::*,
        },
    },
    core::{Error, Result, w},
};

const LIMIT: Duration = Duration::from_secs(60);
fn fail(s: impl std::fmt::Display) -> Error { Error::new(E_FAIL, s.to_string()) }
/// 无标题栏、无拖动/缩放模态循环的普通可激活窗口；输入在创建线程自己的消息泵中路由。
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT
{
    if msg == WM_PAINT
    {
        unsafe {
            let _ = ValidateRect(Some(hwnd), None);
        }
        return LRESULT(0);
    }
    if msg == WM_ERASEBKGND
    {
        return LRESULT(1);
    }
    if matches!(msg, WM_SETFOCUS | WM_KILLFOCUS | WM_MOUSEACTIVATE | WM_DPICHANGED | WM_DISPLAYCHANGE | WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_KEYDOWN | WM_KEYUP)
    {
        let posted = DISPATCHED.get() == Some((msg, wp.0, lp.0));
        let msg = MSG { hwnd, message: msg, wParam: wp, lParam: lp, time: unsafe { GetMessageTime() } as u32, ..Default::default() };
        RECEIVED.with_borrow_mut(|q| {
            if q.len() < 512
            {
                q.push_back((msg, posted, unsafe { GetForegroundWindow() } == hwnd && unsafe { GetFocus() } == hwnd));
            }
            else
            {
                OVERFLOW.set(true);
            }
        });
        if matches!(msg.message, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_KEYDOWN | WM_KEYUP | WM_DPICHANGED | WM_DISPLAYCHANGE)
        {
            return LRESULT(0);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
thread_local! {
static RECEIVED:std::cell::RefCell<std::collections::VecDeque<(MSG,bool,bool)>>=const{std::cell::RefCell::new(std::collections::VecDeque::new())};
static DISPATCHED:std::cell::Cell<Option<(u32,usize,isize)>>=const{std::cell::Cell::new(None)};
static OVERFLOW:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
}
/// 保留桌面坐标，只有可见部分参与键盘候选；鼠标和绘制分别加/减同一个裁剪起点。
fn clipped(entries: Vec<HitEntry>, crop: Rect, focus: Option<&str>) -> HitScene
{
    HitScene::new(
        entries
            .into_iter()
            .filter_map(|mut e| {
                e.clip = intersect(e.visible()?, crop)?;
                Some(e)
            })
            .collect(),
        focus,
    )
}
struct Scene
{
    s: WorkspaceSnapshot,
    p: ApplicationPreferences,
    id: String,
    desktop: HostGeometry,
    crop: Rect,
}
impl Scene
{
    fn hits(&self, f: Option<&str>) -> HitScene { clipped(scene_hits(&self.s, &self.p, &self.id, self.desktop, f), self.crop, f) }
    fn frame(&self, f: Option<&str>) -> DrawList
    {
        let mut l = DrawList::new();
        let size = Rect::new(0., 0., self.crop.width, self.crop.height);
        // 窗口有自己的底色，明确区别于透明桌面；工作区空白仍可命中。
        l.push(DrawCommand::FillRoundRect { rect: size, radius: 0., color: Color(0x283236ff) });
        let contents = draw_with_focus(&self.s, &self.p, &self.id, self.desktop, f).translated(-self.crop.x, -self.crop.y);
        for c in contents.commands()
        {
            l.push(c.clone());
        }
        l
    }
}

/// 仅显式验收用：在 HWND 所属 GUI 线程逐个定向投递，不走系统输入流，不调用焦点状态机。
/// 同线程不能在校验和 PostMessage 之间销毁/复用这个窗口；即使前台变化，目标也不是新前台窗。
fn post_probe(hwnd: HWND, hits: &HitScene, crop: Rect, dpi: DpiScale) -> Result<(std::collections::VecDeque<(u32, usize, isize)>, Vec<Value>)>
{
    let (mut header, mut content, mut background) = (None, None, None);
    // 选择可见目标只作坐标校准；最终事件记录必须在 Windows 消息泵接收后重新计算。
    'scan: for y in (2..dpi.dip_to_px(crop.height)).step_by(5)
    {
        for x in (2..dpi.dip_to_px(crop.width)).step_by(5)
        {
            let Some(h) = hits.hit(crop.x + dpi.px_to_dip(x), crop.y + dpi.px_to_dip(y))
            else
            {
                continue;
            };
            let xy = (x, y);
            match h.region.as_str().as_str()
            {
                "header" if header.is_none() => header = Some((h.id, xy)),
                "content" if content.is_none() && header.as_ref().is_some_and(|(id, _)| id == &h.id) => content = Some(xy),
                "background" if background.is_none() => background = Some(xy),
                _ =>
                {}
            }
            if header.is_some() && content.is_some() && background.is_some()
            {
                break 'scan;
            }
        }
    }
    let pts = [header.ok_or_else(|| fail("裁剪视图无标题命中点"))?.1, content.ok_or_else(|| fail("裁剪视图没有同目标内容点"))?, background.ok_or_else(|| fail("裁剪视图无空白点"))?];
    let mut audit = Vec::new();
    let mut sent = std::collections::VecDeque::new();
    let mut count = 0;
    let mut baseline = POINT::default();
    unsafe { GetCursorPos(&mut baseline) }?;
    let mut post = |message, w, l| -> Result<()> {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != std::process::id() || unsafe { GetForegroundWindow() } != hwnd || unsafe { GetFocus() } != hwnd
        {
            return Err(fail(format!("第 {} 条投递前前台/焦点/所有者校验失败；已投递 {count} 条", count + 1)));
        }
        let fg_before = unsafe { GetForegroundWindow() };
        let mut cursor_before = POINT::default();
        unsafe { GetCursorPos(&mut cursor_before) }?;
        let focus_before = unsafe { GetFocus() };
        if fg_before != hwnd || focus_before != hwnd || cursor_before != baseline
        {
            return Err(fail(format!("第 {} 条投递前前台、焦点或光标变化，中止", count + 1)));
        }
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(w), LPARAM(l)) }?;
        let fg_after = unsafe { GetForegroundWindow() };
        let mut cursor_after = POINT::default();
        unsafe { GetCursorPos(&mut cursor_after) }?;
        let focus_after = unsafe { GetFocus() };
        if fg_after != hwnd || cursor_before != cursor_after || focus_after != hwnd
        {
            return Err(fail(format!("第 {} 条投递后前台/焦点/光标变化，停止", count + 1)));
        }
        audit.push(json!({"post":count+1,"pid":pid,"fg_before":fg_before.0 as usize,"fg_after":fg_after.0 as usize,"focus_before":focus_before.0 as usize,"focus_after":focus_after.0 as usize,"cursor_before":[cursor_before.x,cursor_before.y],"cursor_after":[cursor_after.x,cursor_after.y]}));
        sent.push_back((message, w, l));
        count += 1;
        Ok(())
    };
    post(WM_MOUSEACTIVATE, hwnd.0 as usize, ((WM_LBUTTONDOWN << 16) | HTCLIENT as u32) as isize)?;
    for (x, y) in pts
    {
        let lp = (((y as u32) << 16) | (x as u32 & 0xffff)) as isize;
        post(WM_MOUSEMOVE, 0, lp)?;
        post(WM_LBUTTONDOWN, 1, lp)?;
        post(WM_LBUTTONUP, 0, lp)?;
    }
    for (m, k) in [(WM_KEYDOWN, 9), (WM_KEYUP, 9), (WM_KEYDOWN, 16), (WM_KEYDOWN, 9), (WM_KEYUP, 9), (WM_KEYUP, 16), (WM_KEYDOWN, 38), (WM_KEYUP, 38), (WM_KEYDOWN, 39), (WM_KEYUP, 39), (WM_KEYDOWN, 40), (WM_KEYUP, 40), (WM_KEYDOWN, 37), (WM_KEYUP, 37), (WM_KEYDOWN, 27)]
    {
        post(m, k, 1)?;
    }
    Ok((sent, audit))
}
/// 测试数据在内存构造，三种外壳均交给生产 scene_blocks 排序、绘制与命中。
fn fixture_scene() -> WorkspaceSnapshot
{
    let bounds = |left, top, width, height| json!({"left":left,"top":top,"width":width,"height":height});
    serde_json::from_value(json!({"schemaVersion":1,"revision":0,"unclassifiedItems":[],
       "fences":[{"id":"fixture-fence","title":"测试围栏","monitorId":"fixture","bounds":bounds(40,80,330,180),"members":[],"layoutMode":"tile","scrollTop":0,"collapsed":false,"lastExpandedHeight":180,"zIndex":1}],
       "components":[{"instanceId":"fixture-clock","componentId":"official.local-clock","monitorId":"fixture","bounds":bounds(420,80,260,180),"stateVersion":1,"state":{}}],
       "folderPortals":[{"id":"fixture-portal","rootPath":"测试门户","monitorId":"fixture","bounds":bounds(420,310,260,160),"zIndex":2,"appearance":{"title":"测试门户","titleMode":"fixed","buttonMode":"hover","collapsed":false,"autoExpand":false}}]}))
    .expect("内置消息链路测试场景必须可解码")
}
pub fn run_live(fixture: bool) -> Result<Value>
{
    let _apt = ComApartment::enter()?;
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // fixture 模式不打开配置目录，前后指纹由进程外验收器单独核对。
    let (before, s, p) = if fixture
    {
        (None, fixture_scene(), ApplicationPreferences { clock_enabled: true, folder_portal_enabled: true, ..Default::default() })
    }
    else
    {
        let before = layout_fingerprint()?;
        let (s, p) = read_scene()?;
        (Some(before), identity_scene(&s), p)
    };
    let m = crate::platform::windows::enumerate_desktop_monitors().map_err(fail)?.into_iter().find(|m| m.primary).ok_or_else(|| fail("无主屏"))?;
    let dpi = DpiScale::new(m.dpi);
    let work = PixelRect::from_xywh(m.work_x, m.work_y, m.work_width as i32, m.work_height as i32);
    let desktop = HostGeometry::for_work_area(work, dpi);
    // 裁剪右上角原布局，而不是把所有持久矩形重新钳制到小窗口。
    let w = dpi.dip_to_px(900.).min(work.width() * 3 / 4);
    let h = dpi.dip_to_px(560.).min(work.height() * 3 / 4);
    if w <= 0 || h <= 0
    {
        return Err(fail("有效工作区不足"));
    }
    let crop = Rect::new(if fixture { 0. } else { dpi.px_to_dip(work.width() - w) }, 0., dpi.px_to_dip(w), dpi.px_to_dip(h));
    let scene = Scene { s, p, id: if fixture { "fixture".into() } else { m.id.clone() }, desktop: if fixture { HostGeometry::for_work_area(PixelRect::from_xywh(0, 0, w, h), dpi) } else { desktop }, crop };
    let mut focus = FocusState::default();
    let mut hits = scene.hits(None);
    if hits.focus_candidates().is_empty()
    {
        return Err(fail("当前主屏裁剪区域无持久对象；不借用其他屏布局"));
    }
    let geometry = HostGeometry::for_work_area(PixelRect::from_xywh(0, 0, w, h), dpi);
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }?.0);
    let class = w!("DesktopToolsNativeInputLive");
    let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, hInstance: instance, lpszClassName: class, lpfnWndProc: Some(proc), hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?, ..Default::default() };
    if unsafe { RegisterClassExW(&wc) } == 0
    {
        return Err(Error::from_win32());
    }
    let devices = Devices::new()?;
    let mut painter = Painter::new()?;
    let probe = fixture || std::env::args_os().any(|a| a == "--verify-window-messages");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            class,
            if probe
            {
                w!("点击此处开始（或按 F8）· 点击窗口获得焦点 · 300 秒超时")
            }
            else
            {
                w!("Desktop Tools · 局部输入验证（Esc 退出）")
            },
            WS_POPUP,
            work.left + (work.width() - w) / 2,
            work.top + (work.height() - h) / 2,
            w,
            h,
            None,
            None,
            Some(instance),
            None,
        )
    }?;
    let start = Instant::now();
    let mut deadline = start + if probe { Duration::from_secs(300) } else { LIMIT };
    let owned = DemoWindows { hwnds: vec![hwnd] };
    let mut host = SurfaceHost::attach(hwnd, geometry, &devices.composition)?;
    let button = Rect::new(10., 10., (crop.width - 20.).max(0.), 38.);
    let mut paint = |f: Option<&str>, waiting: bool| -> Result<()> {
        let mut frame = scene.frame(f);
        if waiting
        {
            let rect = button;
            frame.push(DrawCommand::FillRoundRect { rect, radius: 3., color: Color(0x14252eff) });
            frame.push(DrawCommand::Caption { rect: Rect::new(rect.x + 8., rect.y + 6., (rect.width - 16.).max(0.), 26.), text: "点击此处开始（或按 F8）· 点击窗口获得焦点 · 300 秒超时".into(), size: 16., weight: 600, color: Color(0xffffffff), align: super::draw::TextAlign::Start });
        }
        let (surface, _) = SurfaceHost::prepare(geometry, &mut painter, &devices, &frame).map_err(Error::from)?;
        host.publish(geometry, surface, &devices)
    };
    paint(None, probe)?;
    unsafe {
        let _ = ShowWindow(hwnd, if probe { SW_SHOWNOACTIVATE } else { SW_SHOW });
    }
    let mut origin = POINT::default();
    if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool()
    {
        return Err(Error::from_win32());
    }
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }?;
    if rect.right != w || rect.bottom != h || unsafe { GetDpiForWindow(hwnd) } != m.dpi
    {
        return Err(fail("实际窗口几何与裁剪几何不一致"));
    }
    if probe
    {
        eprintln!("待命 pid={} hwnd={} rect={},{},{},{} start_screen={},{},{},{} timeout=300s until={} start_center_hits_hwnd={}", std::process::id(), hwnd.0 as usize, origin.x, origin.y, w, h, origin.x + dpi.dip_to_px(button.x), origin.y + dpi.dip_to_px(button.y), dpi.dip_to_px(button.width), dpi.dip_to_px(button.height), (chrono::Local::now() + chrono::Duration::seconds(300)).to_rfc3339(), unsafe { WindowFromPoint(POINT { x: origin.x + w / 2, y: origin.y + dpi.dip_to_px(button.y + button.height / 2.) }) } == hwnd);
    }
    let (mut posted, mut audit) = (std::collections::VecDeque::new(), Vec::new());
    let mut activated_at = None;
    let mut events = Vec::new();
    let mut dropped = 0usize;
    let mut rejected = 0usize;
    let mut shift = false;
    let reason = 'pump: loop
    {
        let now = Instant::now();
        if now >= deadline
        {
            break if probe && activated_at.is_none() { "未激活" } else { "timeout" };
        }
        if OVERFLOW.get()
        {
            return Err(fail("消息收件队列溢出"));
        }
        let received = RECEIVED.with_borrow_mut(|q| q.pop_front());
        let Some((msg, from_probe, authorized)) = received
        else
        {
            let mut queued = MSG::default();
            if unsafe { PeekMessageW(&mut queued, None, 0, 0, PM_REMOVE) }.as_bool()
            {
                if queued.message == WM_QUIT || !unsafe { IsWindow(Some(hwnd)) }.as_bool()
                {
                    break "closed";
                }
                let tuple = (queued.message, queued.wParam.0, queued.lParam.0);
                let verified = queued.hwnd == hwnd && posted.front() == Some(&tuple);
                if verified
                {
                    posted.pop_front();
                    DISPATCHED.set(Some(tuple));
                }
                unsafe {
                    DispatchMessageW(&queued);
                }
                DISPATCHED.set(None);
            }
            else
            {
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(None, (deadline - now).as_millis().min(1000) as u32, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                }
            }
            continue;
        };
        let source = if from_probe { "posted" } else { "window" };
        if matches!(msg.message, WM_MOUSEACTIVATE | WM_KILLFOCUS | WM_SETFOCUS | WM_DPICHANGED | WM_DISPLAYCHANGE)
        {
            let before = focus.focused_id().map(str::to_owned);
            if msg.message == WM_KILLFOCUS
            {
                shift = false;
                focus.set(None);
                hits = scene.hits(None);
                if before.is_some()
                {
                    paint(None, probe && activated_at.is_none())?;
                }
            }
            let name = match msg.message
            {
                WM_MOUSEACTIVATE => "WM_MOUSEACTIVATE",
                WM_KILLFOCUS => "WM_KILLFOCUS",
                WM_SETFOCUS => "WM_SETFOCUS",
                WM_DPICHANGED => "WM_DPICHANGED",
                _ => "WM_DISPLAYCHANGE",
            };
            if events.len() < 8000
            {
                events.push(json!({"seq":events.len()+dropped+1,"source":source,"message":name,"at_foreground_focus":authorized,"before":before,"after":focus.focused_id()}));
            }
            else
            {
                dropped += 1;
            }
            if matches!(msg.message, WM_DPICHANGED | WM_DISPLAYCHANGE)
            {
                break "geometry_changed";
            }
            continue;
        }
        let mouse = matches!(msg.message, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP);
        let key = matches!(msg.message, WM_KEYDOWN | WM_KEYUP);
        if !authorized || unsafe { GetForegroundWindow() } != hwnd || unsafe { GetFocus() } != hwnd
        {
            if from_probe
            {
                return Err(fail("探针投递后失去前台，停止"));
            }
            rejected += 1;
            continue;
        }
        let px = msg.lParam.0 as i16 as i32;
        let py = (msg.lParam.0 >> 16) as i16 as i32;
        let click = msg.message == WM_LBUTTONDOWN && button.contains(dpi.px_to_dip(px), dpi.px_to_dip(py));
        let f8 = msg.message == WM_KEYDOWN && msg.wParam.0 == 0x77 && msg.lParam.0 & (1 << 30) == 0 && unsafe { GetKeyState(0x77) } < 0;
        if probe && activated_at.is_none() && !from_probe && (click || f8)
        {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid != std::process::id()
            {
                return Err(fail("开始信号的窗口所有者不匹配，0 条投递"));
            }
            if click
            {
                let mut pt = POINT { x: px, y: py };
                if !unsafe { ClientToScreen(hwnd, &mut pt) }.as_bool()
                {
                    return Err(Error::from_win32());
                }
                if unsafe { WindowFromPoint(pt) } != hwnd
                {
                    return Err(fail("开始区实际屏幕命中其他窗口，0 条投递"));
                }
            }
            activated_at = Some(start.elapsed().as_millis() as u64);
            deadline = Instant::now() + LIMIT;
            events.push(json!({"seq":events.len()+dropped+1,"message":if click{"WM_LBUTTONDOWN"}else{"WM_KEYDOWN"},"source":"window","vk":f8.then_some(119),"action":if click{"activate_button"}else{"activate_f8"},"client_px":click.then_some([px,py]),"screen_px":click.then_some([px+origin.x,py+origin.y]),"workspace_dip":click.then_some([crop.x+dpi.px_to_dip(px),crop.y+dpi.px_to_dip(py)]),"hit":click.then_some("standby-start"),"before":focus.focused_id(),"after":focus.focused_id(),"foreground":hwnd.0 as usize,"focus":hwnd.0 as usize,"pid":pid,"standby_posts":0}));
            paint(focus.focused_id(), false)?;
            (posted, audit) = post_probe(hwnd, &hits, crop, dpi)?;
            continue;
        }
        if key && !matches!(msg.wParam.0, 9 | 16 | 27 | 37..=40)
        {
            continue;
        }
        let prior = focus.focused_id().map(str::to_owned);
        let mut client = None;
        let mut screen = None;
        let mut workspace = None;
        let mut hit = None;
        if mouse
        {
            let x = msg.lParam.0 as i16 as i32;
            let y = (msg.lParam.0 >> 16) as i16 as i32;
            let mut pt = POINT { x, y };
            if !unsafe { ClientToScreen(hwnd, &mut pt) }.as_bool()
            {
                return Err(Error::from_win32());
            }
            client = Some([x, y]);
            screen = Some([pt.x, pt.y]);
            let xy = [crop.x + dpi.px_to_dip(x), crop.y + dpi.px_to_dip(y)];
            workspace = Some(xy);
            if x >= 0 && y >= 0 && x < w && y < h
            {
                hit = hits.hit(xy[0], xy[1]);
            }
        }
        let vk = msg.wParam.0;
        let pressed = msg.message == WM_KEYDOWN;
        if key && vk == 0x10
        {
            shift = pressed;
        }
        let action = if msg.message == WM_LBUTTONDOWN
        {
            Some(FocusAction::Click)
        }
        else if pressed
        {
            match vk
            {
                9 => Some(
                    if shift || unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0
                    {
                        FocusAction::Previous
                    }
                    else
                    {
                        FocusAction::Next
                    },
                ),
                37 => Some(FocusAction::Left),
                38 => Some(FocusAction::Up),
                39 => Some(FocusAction::Right),
                40 => Some(FocusAction::Down),
                _ => None,
            }
        }
        else
        {
            None
        };
        if let Some(a) = action
        {
            if focus.apply(a, &hits, hit.as_ref()).is_some()
            {
                hits = scene.hits(focus.focused_id());
                paint(focus.focused_id(), probe && activated_at.is_none())?;
            }
        }
        let name = match msg.message
        {
            WM_MOUSEMOVE => "WM_MOUSEMOVE",
            WM_LBUTTONDOWN => "WM_LBUTTONDOWN",
            WM_LBUTTONUP => "WM_LBUTTONUP",
            WM_KEYDOWN => "WM_KEYDOWN",
            _ => "WM_KEYUP",
        };
        let entry = json!({"seq":events.len()+dropped+1,"source":source,"message":name,"vk":key.then_some(vk),"time":msg.time,"screen_px":screen,"client_px":client,"workspace_dip":workspace,"hit":hit.map(|h|json!({"id":h.id,"region":h.region.as_str()})),"action":action.map(|a|a.as_str()),"before":prior,"after":focus.focused_id()});
        if events.len() < 8000
        {
            events.push(entry);
        }
        else
        {
            dropped += 1;
        }
        if pressed && vk == 27
        {
            break 'pump "esc";
        }
    };
    drop(host);
    drop(owned);
    let after = if fixture { None } else { Some(layout_fingerprint()?) };
    if after != before || unsafe { IsWindow(Some(hwnd)) }.as_bool()
    {
        return Err(fail("指纹变化或窗口残留"));
    }
    Ok(json!({"activated":activated_at.is_some(),"standby_ms":activated_at.unwrap_or(start.elapsed().as_millis() as u64),"standby_posts":0,"posted_checks":audit,"posted_remaining":posted.len(),"pid":std::process::id(),"hwnd":hwnd.0 as usize,"scene_kind":if fixture{"self_contained"}else{"persisted"},"monitor":m.id,"dpi":m.dpi,"client_screen_origin_px":[origin.x,origin.y],"size_px":[w,h],"crop_workspace_dip":crop,"exit_reason":reason,"rejected_not_foreground":rejected,"events":events,"omitted":dropped,"fingerprint_before":before,"fingerprint_after":after,"residual":false}))
}

#[cfg(test)]
mod tests
{
    use super::super::input::TargetKind;
    use super::*;
    #[test]
    fn fixture_routes_three_production_blocks()
    {
        let g = HostGeometry::for_work_area(PixelRect::from_xywh(0, 0, 900, 560), DpiScale::new(96));
        let scene = Scene { s: fixture_scene(), p: ApplicationPreferences { clock_enabled: true, folder_portal_enabled: true, ..Default::default() }, id: "fixture".into(), desktop: g, crop: Rect::new(0., 0., 900., 560.) };
        let h = scene.hits(None);
        assert_eq!(h.focus_candidates().len(), 3);
        for (x, y, id, region) in [(42., 82., "fixture-fence", "header"), (42., 120., "fixture-fence", "content"), (2., 2., "workspace-background", "background")]
        {
            let hit = h.hit(x, y).unwrap();
            assert_eq!((hit.id.as_str(), hit.region.as_str().as_str()), (id, region));
        }
        assert!(!scene.frame(None).is_empty());
    }
    #[test]
    fn crop_preserves_workspace_origin_and_removes_offscreen_keyboard_targets()
    {
        let entry = |id, x| HitEntry::new(id, TargetKind::Fence, Rect::new(x, 20., 60., 70.), 28., vec![], 50, 0, None);
        let crop = Rect::new(100., 0., 200., 120.);
        let scene = clipped(vec![entry("outside", 0.), entry("partial", 75.)], crop, None);
        assert_eq!(scene.reading_order(), ["partial"]);
        assert!(scene.hit(99., 25.).is_none());
        assert_eq!(scene.hit(100., 25.).unwrap().region.as_str(), "header");
        assert_eq!(scene.hit(100., 51.).unwrap().region.as_str(), "content");
    }
}
