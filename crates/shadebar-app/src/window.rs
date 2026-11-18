//! 主窗口与 DWM 视觉（§4 #26 / T03.1 / T03.6 / T04.1）。
//!
//! # B3 定稿的窗口配置（`docs/spike-report.md` §3.8）
//!
//! 1. `WS_OVERLAPPEDWINDOW`，不加 `WS_EX_LAYERED`；
//! 2. `DWMWA_SYSTEMBACKDROP_TYPE = 4`（Mica Alt），不接管 `WM_NCCALCSIZE`；
//! 3. `DWMWA_WINDOW_CORNER_PREFERENCE = 2`（圆角）；
//! 4. 不调 `DwmExtendFrameIntoClientArea`；
//! 5. `WM_ERASEBKGND` 返回 1，`WM_PAINT` 里由 [`View`] 铺满整幅不透明背景；
//! 6. 保留 DWM 标准标题栏（B3 的标题栏 Mica 靠 DWM frame 绘制）。
//!
//! # T03.6 / T04.1 新增
//!
//! * `WM_CLOSE` 按 S-41 决定「最小化到托盘」或「直接退出」（A-09）；
//! * 托盘 6 项菜单（`WM_COMMAND`）、左键三选一（S-40）、判定色点（S-43，1 Hz）；
//! * `WM_SHADEBAR_ACTIVATE` 供第二实例激活既有窗口（A-01）；
//! * 系统深浅色 / UI 语言变化即时重绘（FR-40 / A-23）；
//! * 所有输入消息统一转发给 [`View::on_message`]，窗口层不做布局判断。

use std::sync::Arc;

use shadebar_core::win::HResultExt;
use shadebar_core::{PRODUCT_NAME, SbError, SbResult};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW, LoadCursorW, MINMAXINFO, MSG,
    PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA,
    SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_PAINT, WM_SETTINGCHANGE, WM_SIZE,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use crate::app::App;
use crate::ipc_server::{
    MAIN_WINDOW_CLASS, WM_SHADEBAR_ACTIVATE, WM_SHADEBAR_DIAG, WM_SHADEBAR_TRAY,
};
use crate::render::{self, RenderCtx};
use crate::tray::{
    MENU_ABOUT, MENU_AUTOSTART, MENU_EXIT, MENU_EXPORT, MENU_ID_BASE, MENU_OPEN, MENU_PAUSE,
    TRAY_LBUTTONUP, TRAY_RBUTTONUP, TrayIcon, handle_left_click, menu_id, tray_event,
};
use crate::ui::theme::{self, ThemeMode};
use crate::ui::widgets::PopupKind;
use crate::ui::View;

/// `DWMWA_SYSTEMBACKDROP_TYPE`（38）。
const DWMWA_SYSTEMBACKDROP_TYPE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(38);
/// `DWMWA_WINDOW_CORNER_PREFERENCE`（33）。
const DWMWA_WINDOW_CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
/// `DWMSBT_TABBEDWINDOW`（Mica Alt）。
const DWMSBT_TABBEDWINDOW: i32 = 4;
/// `DWMWCP_ROUND`。
const DWMWCP_ROUND: i32 = 2;
/// `ERROR_CLASS_ALREADY_EXISTS`（1410）：同一进程重复注册窗口类不算错误。
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

/// 挂在 `GWLP_USERDATA` 上的窗口状态。
struct WindowState {
    ctx: RenderCtx,
    mode: ThemeMode,
    view: View,
    app: Arc<App>,
    tray: TrayIcon,
    /// 首启气泡是否已展示（S-02 默认关，只提示一次）。
    balloon_shown: bool,
}

/// 创建主窗口（尚不显示）。
///
/// # Errors
///
/// 类注册、窗口创建、渲染上下文初始化或 backdrop 设置失败时返回错误。
pub fn create_main_window(app: Arc<App>) -> SbResult<HWND> {
    let instance =
        unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.map_err(|e| {
            SbError::HResult {
                op: "GetModuleHandleW",
                hr: e.code().0,
            }
        })?;
    let class = HSTRING::from(MAIN_WINDOW_CLASS);

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    // SAFETY: wc 已完整初始化，class 字符串在调用期间存活。
    if unsafe { RegisterClassW(&wc) } == 0 {
        let code = shadebar_core::win::last_error_code();
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(SbError::Win32 { op: "RegisterClassW", code });
        }
    }

    let title = HSTRING::from(PRODUCT_NAME);
    // SAFETY: 所有参数有效；父窗口/菜单为空。
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            &title,
            WS_OVERLAPPEDWINDOW,
            160,
            120,
            760,
            600,
            HWND::default(),
            HMENU::default(),
            instance,
            None,
        )
    }
    .map_err(|e| SbError::HResult {
        op: "CreateWindowExW",
        hr: e.code().0,
    })?;

    // SAFETY: hwnd 有效。
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let mut client = RECT::default();
    // SAFETY: hwnd 有效，client 为本地可写结构。
    unsafe { GetClientRect(hwnd, &mut client) }.map_err(|e| SbError::HResult {
        op: "GetClientRect",
        hr: e.code().0,
    })?;
    let size_px = (
        (client.right - client.left).max(1) as u32,
        (client.bottom - client.top).max(1) as u32,
    );
    let mode = ThemeMode::system();
    let cfg = app.config();
    let lang = app.language();
    let degraded = app.degraded();
    let mut view = View::new(&cfg, lang, degraded);
    view.set_dpi(dpi);
    view.set_theme(mode);
    let client_dip = (
        (client.right - client.left) as f32 * 96.0 / dpi as f32,
        (client.bottom - client.top) as f32 * 96.0 / dpi as f32,
    );
    view.set_client(client_dip.0, client_dip.1);
    let tray = TrayIcon::add(
        hwnd,
        None,
        cfg.get(crate::config::SettingKey::S43) != 0,
        degraded,
        lang,
    );

    let state = Box::new(WindowState {
        ctx: RenderCtx::new(hwnd, dpi, size_px)?,
        mode,
        view,
        app,
        tray,
        balloon_shown: false,
    });
    let raw = Box::into_raw(state);
    // SAFETY: 把 Box 的所有权移交给窗口；WM_DESTROY 里取回并释放。
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize) };

    apply_b3_backdrop(hwnd)?;
    Ok(hwnd)
}

/// 显示窗口并进入消息循环。返回进程退出码。
///
/// `show_on_start = false` 对应 S-03「启动时最小化到托盘」：窗口保持隐藏，
/// 托盘图标已在 [`create_main_window`] 里创建，进程照常驻留。
pub fn run_message_loop(hwnd: HWND, show_on_start: bool) -> i32 {
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = ShowWindow(hwnd, if show_on_start { SW_SHOW } else { SW_HIDE });
        if show_on_start {
            let _ = SetForegroundWindow(hwnd);
        }
        let _ = InvalidateRect(hwnd, None, false);
    }
    let mut msg = MSG::default();
    loop {
        // SAFETY: msg 由系统填写；窗口过滤器为空表示收本线程全部消息。
        let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
        if ret.0 == -1 {
            return 1;
        }
        if ret.0 == 0 {
            return msg.wParam.0 as i32;
        }
        // SAFETY: msg 有效。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 显示并激活设置窗口（单实例激活 / 托盘「打开设置」共用）。
pub fn show_settings(hwnd: HWND) {
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = InvalidateRect(hwnd, None, false);
    }
}

/// 最小化到托盘（A-09：进程仍在、托盘图标仍在）。
pub fn hide_to_tray(hwnd: HWND) {
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// 按 B3 定稿配置 DWM 视觉，并回读确认属性已写入。
///
/// 注意：**回读成功不等于视觉生效**，视觉验收必须靠截图采样（T03.1 已取证）。
///
/// # Errors
///
/// `DwmSetWindowAttribute` 失败，或回读值与设定值不符时返回 [`SbError::Unsupported`]。
fn apply_b3_backdrop(hwnd: HWND) -> SbResult<()> {
    let backdrop = DWMSBT_TABBEDWINDOW;
    // SAFETY: i32 的地址与 size_of::<i32>() 一致。
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    }
    .sb("DwmSetWindowAttribute(SYSTEMBACKDROP_TYPE)")?;

    let mut read_back: i32 = -1;
    // SAFETY: 同上。
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &mut read_back as *mut i32 as *mut std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    }
    .sb("DwmGetWindowAttribute(SYSTEMBACKDROP_TYPE)")?;
    if read_back != backdrop {
        return Err(SbError::Unsupported {
            what: "DWMWA_SYSTEMBACKDROP_TYPE 回读与设定值不一致",
        });
    }
    let corner = DWMWCP_ROUND;
    // SAFETY: 同上。
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    }
    .sb("DwmSetWindowAttribute(WINDOW_CORNER_PREFERENCE)")?;
    Ok(())
}

/// 取回并释放挂在窗口上的状态。
///
/// # Safety
///
/// `hwnd` 必须是我们自己创建、且 `GWLP_USERDATA` 里放的是 `Box<WindowState>` 的窗口。
unsafe fn take_state(hwnd: HWND) -> Option<Box<WindowState>> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if raw.is_null() {
        return None;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
    // SAFETY: 指针由 Box::into_raw 产生，只在这里被取回一次。
    Some(unsafe { Box::from_raw(raw) })
}

/// 取状态的可变借用。
///
/// # Safety
///
/// 只能在处理消息期间、且保证不重入地调用（Win32 消息在同一线程内串行派发）。
unsafe fn state_mut(hwnd: HWND) -> Option<&'static mut WindowState> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    // SAFETY: 指针由 create_main_window 写入，生命周期覆盖整个消息循环。
    unsafe { raw.as_mut() }
}

fn low_word(v: isize) -> i32 {
    (v & 0xFFFF) as i16 as i32
}

fn high_word(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as i16 as i32
}

/// 把输入类消息转发给 `View`，并把产生的动作回写给 `App`。
fn dispatch_input(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    // SAFETY: 同一线程消息串行派发，不会与别处形成可变别名。
    let state = unsafe { state_mut(hwnd) }?;
    if msg == 0x0200 {
        // WM_MOUSEMOVE：先更新悬停与 Tooltip，再设置光标。
        let over = state.view.hit_test_and_update_cursor(low_word(lp.0), high_word(lp.0));
        let _ = over; // 光标统一用箭头；交互态由控件自身绘制。
        // SAFETY: 无前置条件。
        if let Ok(cursor) = unsafe { LoadCursorW(None, IDC_ARROW) } {
            // SAFETY: cursor 有效。
            unsafe { SetCursor(cursor) };
        }
    }
    let result = state.view.on_message(hwnd, msg, wp, lp)?;
    let actions = state.view.take_actions();
    let changed = state.app.apply_actions(actions);
    if changed {
        state.app.sync_view(&mut state.view);
        let cfg = state.app.config();
        state.tray.refresh(
            hwnd,
            state.app.decision_color(),
            cfg.get(crate::config::SettingKey::S43) != 0,
            state.app.degraded(),
            state.app.language(),
            false,
        );
    }
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
    Some(result)
}

/// 托盘菜单命令（A-07：6 项全部生效）。
fn handle_menu(hwnd: HWND, wp: WPARAM) {
    let id = menu_id(wp);
    if !(MENU_ID_BASE..=MENU_ID_BASE + 5).contains(&id) {
        return; // 不是托盘菜单命令，不吞掉别的 WM_COMMAND。
    }
    // SAFETY: 同上。
    let Some(state) = (unsafe { state_mut(hwnd) }) else {
        return;
    };
    match id {
        MENU_OPEN => show_settings(hwnd),
        MENU_PAUSE => {
            let _ = state.app.toggle_pause();
            state.app.sync_view(&mut state.view);
        }
        MENU_AUTOSTART => {
            let _ = state.app.toggle_autostart();
            state.app.sync_view(&mut state.view);
        }
        MENU_EXPORT => {
            let _ = state.app.export_diagnostics();
            state.view.show_toast("common.export_ok");
        }
        MENU_ABOUT => state.view.request_popup(PopupKind::About),
        MENU_EXIT => {
            state.app.request_exit();
            // SAFETY: hwnd 有效。
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: 以下分支都在同一线程的消息派发上下文中执行，不会并发访问状态。
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),

            WM_SIZE => {
                if let Some(state) = state_mut(hwnd) {
                    let mut client = RECT::default();
                    // SAFETY: hwnd 有效。
                    if GetClientRect(hwnd, &mut client).is_ok() {
                        let dpi = state.ctx.dpi().max(96);
                        state.view.set_dpi(dpi);
                        state.view.set_client(
                            (client.right - client.left) as f32 * 96.0 / dpi as f32,
                            (client.bottom - client.top) as f32 * 96.0 / dpi as f32,
                        );
                    }
                }
                let _ = InvalidateRect(hwnd, None, false);
                LRESULT(0)
            }

            WM_GETMINMAXINFO => {
                let info = lp.0 as *mut MINMAXINFO;
                if !info.is_null() {
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    (*info).ptMinTrackSize.x = render::dip_to_px(dpi, theme::MIN_WIDTH_DIP).round() as i32;
                    (*info).ptMinTrackSize.y = render::dip_to_px(dpi, theme::MIN_HEIGHT_DIP).round() as i32;
                }
                LRESULT(0)
            }

            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xFFFF) as u32;
                if let Some(state) = state_mut(hwnd) {
                    let old_dpi = state.view.dpi();
                    let _ = state.ctx.set_dpi(dpi);
                    state.view.set_dpi(dpi);
                    if old_dpi != dpi {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
                let suggested = lp.0 as *const RECT;
                if !suggested.is_null() {
                    let r = *suggested;
                    let _ = SetWindowPos(
                        hwnd,
                        HWND::default(),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                let _ = InvalidateRect(hwnd, None, false);
                LRESULT(0)
            }

            WM_SETTINGCHANGE => {
                if let Some(state) = state_mut(hwnd) {
                    let mode = ThemeMode::system();
                    if mode != state.mode {
                        state.mode = mode;
                        state.view.set_theme(mode);
                    }
                    let (lang, _) = state.app.refresh_system();
                    if lang != state.view.language() {
                        state.view.rebuild_texts(lang);
                    }
                    state.app.sync_view(&mut state.view);
                }
                let _ = InvalidateRect(hwnd, None, false);
                LRESULT(0)
            }

            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                paint(hwnd);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }

            WM_CLOSE => {
                let exit = match state_mut(hwnd) {
                    Some(s) => {
                        let v = s.app.config().get(crate::config::SettingKey::S41) != 0;
                        if v {
                            s.app.request_exit();
                        }
                        v
                    }
                    None => true,
                };
                if exit {
                    let _ = DestroyWindow(hwnd);
                } else {
                    hide_to_tray(hwnd);
                }
                LRESULT(0)
            }

            WM_COMMAND => {
                handle_menu(hwnd, wp);
                LRESULT(0)
            }

            m if m == WM_SHADEBAR_ACTIVATE => {
                show_settings(hwnd);
                LRESULT(0)
            }

            m if m == WM_SHADEBAR_TRAY => {
                let ev = tray_event(lp);
                if let Some(state) = state_mut(hwnd) {
                    let cfg = state.app.config();
                    let lang = state.app.language();
                    if ev == TRAY_LBUTTONUP {
                        handle_left_click(hwnd, &cfg, lang);
                    } else if ev == TRAY_RBUTTONUP {
                        TrayIcon::show_menu(hwnd, &cfg, lang);
                    }
                }
                LRESULT(0)
            }

            m if m == WM_SHADEBAR_DIAG => {
                // 采样线程 1 Hz 通知：刷新诊断面板与 S-43 判定色点。
                if let Some(state) = state_mut(hwnd) {
                    state.app.sync_view(&mut state.view);
                    let cfg = state.app.config();
                    let lang = state.app.language();
                    // 托盘图标若首次 NIM_ADD 失败（例如 explorer 正在重启），
                    // 借 1 Hz 心跳重试一次 NIM_ADD，不改用户环境。
                    let first = !state.tray.is_added();
                    state.tray.refresh(
                        hwnd,
                        state.app.decision_color(),
                        cfg.get(crate::config::SettingKey::S43) != 0,
                        state.app.degraded(),
                        lang,
                        first,
                    );
                    // S-02 默认关：首启用一次气泡引导，不打扰后续启动。
                    if !state.balloon_shown && cfg.get(crate::config::SettingKey::S02) == 0 {
                        TrayIcon::show_balloon(hwnd, lang);
                        state.balloon_shown = true;
                    }
                }
                let _ = InvalidateRect(hwnd, None, false);
                LRESULT(0)
            }

            WM_DESTROY => {
                if let Some(mut state) = take_state(hwnd) {
                    state.tray.remove(hwnd);
                    state.app.request_exit();
                }
                PostQuitMessage(0);
                LRESULT(0)
            }

            _ => {
                if let Some(r) = dispatch_input(hwnd, msg, wp, lp) {
                    r
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
        }
    }
}

/// 画一帧：整幅不透明背景 + `View` 正式布局，由 D2D 直接呈现到窗口。
///
/// # Safety
///
/// 必须由 `WM_PAINT` 在 `BeginPaint`/`EndPaint` 之间调用。
unsafe fn paint(hwnd: HWND) {
    let Some(state) = (unsafe { state_mut(hwnd) }) else {
        return;
    };
    state.app.sync_view(&mut state.view);
    let mut client = RECT::default();
    // SAFETY: hwnd 有效。
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let w_px = client.right - client.left;
    let h_px = client.bottom - client.top;
    if w_px <= 0 || h_px <= 0 {
        return;
    }
    let dpi = state.ctx.dpi();
    let client_dip = (
        w_px as f32 * 96.0 / dpi as f32,
        h_px as f32 * 96.0 / dpi as f32,
    );
    state.view.set_dpi(dpi);
    state.view.set_client(client_dip.0, client_dip.1);
    state.view.set_theme(state.mode);
    let mode = state.view.theme();

    if state.ctx.begin_frame(w_px, h_px).is_err() {
        return;
    }
    state.view.paint(&state.ctx, mode.palette(), mode);
    // 设备丢失（D2DERR_RECREATE_TARGET）时目标已重建，必须再画一帧。
    if let Ok(true) = state.ctx.end_frame() {
        // SAFETY: hwnd 有效。
        let _ = unsafe { InvalidateRect(hwnd, None, false) };
    }
}
