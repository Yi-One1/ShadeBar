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
//! * 托盘 4 项菜单（`WM_COMMAND`）、左键单击 / 双击（S-40 / S-51）、判定色点（S-43，1 Hz）；
//! * `WM_SHADEBAR_ACTIVATE` 供第二实例激活既有窗口（A-01）；
//! * 系统深浅色 / UI 语言变化即时重绘（FR-40 / A-23）；
//! * 所有输入消息统一转发给 [`View::on_message`]，窗口层不做布局判断。

use std::path::Path;
use std::sync::Arc;

use shadebar_core::win::HResultExt;
use shadebar_core::{PRODUCT_NAME, SbError, SbResult};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW, KillTimer,
    LoadCursorW, LoadIconW, MINMAXINFO, MSG, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    IsWindowVisible, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA,
    SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_PAINT, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use crate::app::App;
use crate::ipc_server::{
    MAIN_WINDOW_CLASS, WM_SHADEBAR_ACTIVATE, WM_SHADEBAR_DIAG, WM_SHADEBAR_TRAY,
};
use crate::render::{self, RenderCtx};
use crate::tray::{
    click_plan, handle_left_action, is_left_click, menu_id, tray_event, ClickPlan, TrayIcon,
    MENU_AUTOSTART, MENU_EXIT, MENU_ID_BASE, MENU_OPEN, TRAY_RBUTTONUP, TRAY_LBUTTONDBLCLK,
};
use crate::ui::theme::{self, ThemeMode};
use crate::ui::{View, ViewAction};

/// explorer 重启后向所有顶层窗口广播的系统消息。
///
/// 托盘图标是"挂在 explorer 上"的：资源管理器一重启，图标随旧任务栏一起消失，
/// 但我们的进程还活着。不响应这条广播，表现就是**软件在跑、托盘里没有图标**，
/// 隐藏区里也没有（因为根本没注册）。消息 ID 由系统按字符串全局分配，跨进程一致。
fn taskbar_created_message() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| {
        // SAFETY: 入参是静态宽字符串，返回值是系统分配的消息 ID。
        unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) }
    })
}

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
/// 托盘"等双击"定时器 ID。
const TIMER_TRAY_SINGLE: usize = 0x5B01;

/// 挂在 `GWLP_USERDATA` 上的窗口状态。
struct WindowState {
    /// 渲染上下文。**按需创建、隐藏即释放**（NFR-02）。
    ///
    /// `ID2D1HwndRenderTarget` 一创建就会隐式拉起 D3D11/DXGI 设备栈
    /// （实测加载 `d3d11.dll` + `dxgi.dll`，私有内存约 50 MB）。
    /// 设置页平时只是隐藏，若一直持有该上下文，一个"开了就忘"的托盘工具
    /// 会白占 60 MB 以上。因此窗口隐藏时置 `None`，下次绘制再重建。
    ctx: Option<RenderCtx>,
    mode: ThemeMode,
    view: View,
    app: Arc<App>,
    tray: TrayIcon,
    /// 首启气泡是否已展示（S-02 默认关，只提示一次）。
    balloon_shown: bool,
    /// 单击是否处于"等双击"的挂起状态（仅当 S-51≠无操作时使用）。
    pending_single: bool,
    /// 双击后要吞掉的尾随单击，截止的 tick（毫秒）。
    swallow_single_until: u32,
}

/// 创建主窗口（尚不显示）。
///
/// Win32 `MAKEINTRESOURCEW(id)` 的等价写法（windows 0.58 未导出该宏）。
///
/// 资源 ID 以伪指针的形式传进去，LoadIconW 会按 ID 解析；我们从不解引用它。
fn make_int_resource_w(id: u16) -> PCWSTR {
    PCWSTR(id as usize as *const u16)
}

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
        // 资源 ID 1 = resources/app.ico（app.rc）。标题栏与 Explorer 用同一套
        // A·对分图标；资源加载失败也不能让窗口注册失败。
        hIcon: unsafe { LoadIconW(instance, make_int_resource_w(1)) }.unwrap_or_default(),
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
        // 不在这里建渲染上下文：D2D 的 HWND 目标会拉起 D3D11/DXGI 设备栈，
        // 而首次 `WM_PAINT` 之前根本用不到它（且用户可能从不打开设置页）。
        ctx: None,
        mode,
        view,
        app,
        tray,
        balloon_shown: false,
        pending_single: false,
        swallow_single_until: 0,
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
///
/// 同时丢弃渲染上下文。D2D 的 HWND 目标会拉起 D3D11/DXGI 设备栈，
/// 窗口隐藏后这些资源毫无用处，却要白占约 50 MB 私有内存（NFR-02）。
/// 下次显示时由 [`paint`] 按需重建。
pub fn hide_to_tray(hwnd: HWND) {
    // SAFETY: 与 wndproc 同一线程，指针由 GWLP_USERDATA 持有。
    unsafe {
        if let Some(state) = state_mut(hwnd) {
            state.ctx = None;
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        // 窗口隐藏后进程进入纯后台采样状态，把工作集里不再活跃的页交还系统。
        // 传 (usize::MAX, usize::MAX) 是文档定义的「尽可能移除页面」语义。
        // 注意：这是账面修剪，采样线程 1 s 后会按需把用到的页换回来；
        // 真正省下来的资源来自上面的 `ctx = None`，这一步只是让托盘常驻时的
        // 任务管理器读数不虚高。
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
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

// ---------------------------------------------------------------------------
// 系统调色板（comdlg32）
//
// 只借一处系统对话框，手写 extern 声明比多开一组 windows crate feature 更轻，
// 也不会把未知依赖带进体积门禁（NFR-01）。
// ---------------------------------------------------------------------------

/// `CHOOSECOLORW`（x64 布局，字段顺序与 winuser.h 一致）。
#[repr(C)]
struct ChooseColorW {
    l_struct_size: u32,
    hwnd_owner: isize,
    h_instance: isize,
    rgb_result: u32,
    cust_colors: *mut u32,
    flags: u32,
    l_cust_data: isize,
    lpfn_hook: isize,
    template_name: *const u16,
}

#[link(name = "comdlg32")]
extern "system" {
    fn ChooseColorW(param: *mut ChooseColorW) -> i32;
}

/// 初始颜色生效。
const CC_RGBINIT: u32 = 0x0000_0001;
/// 直接展开完整对话框（调色板区域已显示）。
const CC_FULLOPEN: u32 = 0x0000_0002;
/// 允许任意颜色（不限于"基本颜色"）。
const CC_ANYCOLOR: u32 = 0x0000_0100;

/// 弹出系统调色板，返回 `0xAARRGGBB`；用户取消返回 `None`。
///
/// **alpha 必须保留**：填充色（S-35/S-37）是 `0xAARRGGBB`，对话框只处理 RGB，
/// 若把 alpha 归零会让"半透明填充"变成全透明。端色（S-14/S-15）的 alpha
/// 由调用方统一置 0xFF——端色是文字色，半透明对屏幕没有意义。
fn pick_color(hwnd: HWND, current_argb: u32) -> Option<u32> {
    let alpha = current_argb & 0xFF00_0000;
    let r = (current_argb >> 16) & 0xFF;
    let g = (current_argb >> 8) & 0xFF;
    let b = current_argb & 0xFF;
    let mut custom = [0u32; 16];
    let mut cc = ChooseColorW {
        l_struct_size: std::mem::size_of::<ChooseColorW>() as u32,
        hwnd_owner: hwnd.0 as isize,
        h_instance: 0,
        // COLORREF 是 0x00BBGGRR，与 ARGB 的字节序相反。
        rgb_result: r | (g << 8) | (b << 16),
        cust_colors: custom.as_mut_ptr(),
        flags: CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR,
        l_cust_data: 0,
        lpfn_hook: 0,
        template_name: std::ptr::null(),
    };
    // SAFETY: 结构体字段已全部初始化，`custom` 在调用期间存活；
    // 对话框是模态的，返回后立刻读结果，没有悬垂引用。
    let ok = unsafe { ChooseColorW(&mut cc) };
    if ok == 0 {
        return None;
    }
    let v = cc.rgb_result;
    let (r, g, b) = (v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF);
    Some(alpha | (r << 16) | (g << 8) | b)
}

/// 把输入类消息转发给 `View`，并把产生的动作回写给 `App`。
///
/// # 借用纪律（ChooseColorW 是模态对话框）
///
/// 模态对话框会跑嵌套消息循环，WM_PAINT / WM_SHADEBAR_DIAG 会重入本 WndProc。
/// 因此 `state_mut(hwnd)` 的 `&mut WindowState` **绝不能跨过 `pick_color`**：
/// 否则两处同时持有可变引用 = UB。这里严格三段式：短借用读 → 无借用弹对话框 →
/// 短借用写回。中间传给对话框的只有 owned 的 `(SettingKey, u32)`。
fn dispatch_input(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    // ① 短作用域：处理消息、取走动作、把 PickColor 需要的当前色读成 owned 值。
    let (result, mut actions, pick_colors) = {
        // SAFETY: 同一线程消息串行派发，且本作用域在模态调用前结束。
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
        let result = state.view.on_message(hwnd, msg, wp, lp).ok().flatten()?;
        let mut actions = Vec::new();
        let mut pick_colors: Vec<(crate::config::SettingKey, u32)> = Vec::new();
        for action in state.view.take_actions() {
            match action {
                ViewAction::PickColor(key) => {
                    pick_colors.push((key, state.app.config().get(key) as u32));
                }
                other => actions.push(other),
            }
        }
        (result, actions, pick_colors)
    }; // 借用在此释放

    // ② 无借用：模态调用期间可以安全重入 WndProc。
    for (key, current) in pick_colors {
        if let Some(argb) = pick_color(hwnd, current) {
            // 系统对话框只回 RGB：端色（S-14/S-15）强制不透明，
            // 填充色（S-35/S-37）保留进入对话框时的 alpha。
            let argb = if matches!(
                key,
                crate::config::SettingKey::S14 | crate::config::SettingKey::S15
            ) {
                argb | 0xFF00_0000
            } else {
                argb
            };
            actions.push(ViewAction::Set(key, i64::from(argb)));
        }
    }

    // ③ 短作用域：重新借用，统一写回动作并刷新视图 / 托盘。
    {
        // SAFETY: 模态对话框已返回，嵌套消息循环也已退出。
        let state = unsafe { state_mut(hwnd) }?;
        let mut changed = false;
        for action in actions {
            match action {
                ViewAction::ExportConfig => match state.app.export_config() {
                    Ok(path) => state.view.show_toast_text(format!(
                        "{}: {}",
                        crate::ui::i18n::text("config.export_ok", state.app.language()),
                        path.display()
                    )),
                    Err(_) => state.view.show_toast("config.export_fail"),
                },
                ViewAction::ExportDiagnostics => match state.app.export_diagnostics() {
                    Ok(path) => state
                        .view
                        .show_toast_text(export_toast_text(state.app.language(), &path)),
                    Err(_) => state.view.show_toast("diag.export_fail"),
                },
                ViewAction::ImportConfig => match state.app.import_config() {
                    Ok(_) => {
                        state.view.show_toast("config.import_ok");
                        changed = true;
                    }
                    Err(_) => state.view.show_toast("config.import_fail"),
                },
                other => changed |= state.app.apply_actions(vec![other]),
            }
        }
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
    }

    // SAFETY: hwnd 有效。
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
    Some(result)
}

/// 诊断包导出提示：把落盘路径一并显示，用户不用猜文件在哪。
fn export_toast_text(lang: crate::config::Lang, path: &Path) -> String {
    format!(
        "{}: {}",
        crate::ui::i18n::text("diag.export_ok", lang),
        path.display()
    )
}

/// 托盘菜单命令（问题 7：只保留设置 / 开机启动 / 版本号 / 退出）。
fn handle_menu(hwnd: HWND, wp: WPARAM) {
    let id = menu_id(wp);
    if !(MENU_ID_BASE..=MENU_ID_BASE + 3).contains(&id) {
        return; // 不是托盘菜单命令，不吞掉别的 WM_COMMAND。
    }
    // SAFETY: 同上。
    let Some(state) = (unsafe { state_mut(hwnd) }) else {
        return;
    };
    match id {
        MENU_OPEN => show_settings(hwnd),
        MENU_AUTOSTART => {
            let _ = state.app.toggle_autostart();
            state.app.sync_view(&mut state.view);
        }
        MENU_EXIT => {
            state.app.request_exit();
            // SAFETY: hwnd 有效。
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // MENU_VERSION 是灰显只读项，不会产生 WM_COMMAND。
        _ => {}
    }
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

/// 取消"等双击"挂起的单击。
fn cancel_pending_single(hwnd: HWND, state: &mut WindowState) {
    if state.pending_single {
        state.pending_single = false;
        // SAFETY: hwnd 有效。
        unsafe { KillTimer(hwnd, TIMER_TRAY_SINGLE) }.ok();
    }
}

/// tick 是否还没到 `until`（处理 49 天回绕）。
fn before_deadline(now: u32, until: u32) -> bool {
    now.wrapping_sub(until) > u32::MAX / 2
}

/// 托盘左键事件：单击与双击共存（S-40 / S-51）。
fn handle_tray_left_event(hwnd: HWND, ev: u32) {
    // SAFETY: 同一线程的消息派发上下文。
    let Some(state) = (unsafe { state_mut(hwnd) }) else {
        return;
    };
    let single = state.app.config().get(crate::config::SettingKey::S40);
    let double = state.app.config().get(crate::config::SettingKey::S51);
    match click_plan(single, double, ev) {
        ClickPlan::Immediate(action) => {
            let cfg = state.app.config();
            let lang = state.app.language();
            handle_left_action(hwnd, action, &cfg, lang);
        }
        ClickPlan::DeferSingle => {
            state.pending_single = true;
            // SAFETY: hwnd 有效；超时后走 WM_TIMER。
            unsafe { SetTimer(hwnd, TIMER_TRAY_SINGLE, GetDoubleClickTime(), None) };
        }
        ClickPlan::Double(action) => {
            cancel_pending_single(hwnd, state);
            // 双击之后还会来一次 WM_LBUTTONUP，必须吞掉，否则会多执行一次单击。
            state.swallow_single_until = unsafe { GetTickCount() }.wrapping_add(250);
            let cfg = state.app.config();
            let lang = state.app.language();
            handle_left_action(hwnd, action, &cfg, lang);
        }
    }
}

/// "等双击"窗口超时：执行被挂起的单击。
fn handle_tray_single_timer(hwnd: HWND) {
    // SAFETY: hwnd 有效；定时器已由系统触发。
    unsafe { KillTimer(hwnd, TIMER_TRAY_SINGLE) }.ok();
    // SAFETY: 同一线程的消息派发上下文。
    let Some(state) = (unsafe { state_mut(hwnd) }) else {
        return;
    };
    if !state.pending_single {
        return;
    }
    state.pending_single = false;
    let cfg = state.app.config();
    let lang = state.app.language();
    handle_left_action(hwnd, cfg.get(crate::config::SettingKey::S40), &cfg, lang);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: 以下分支都在同一线程的消息派发上下文中执行，不会并发访问状态。
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),

            // explorer 重启完毕：托盘被清空，重新对齐图标（v1.3.2）。
            m if m != 0 && m == taskbar_created_message() => {
                if let Some(state) = state_mut(hwnd) {
                    let cfg = state.app.config();
                    let lang = state.app.language();
                    state.tray.reattach(
                        hwnd,
                        state.app.decision_color(),
                        cfg.get(crate::config::SettingKey::S43) != 0,
                        state.app.degraded(),
                        lang,
                    );
                    crate::config::note(&format!(
                        "tray reattach after TaskbarCreated: added={}",
                        state.tray.is_added()
                    ));
                }
                LRESULT(0)
            }

            WM_SIZE => {
                if let Some(state) = state_mut(hwnd) {
                    let mut client = RECT::default();
                    // SAFETY: hwnd 有效。
                    if GetClientRect(hwnd, &mut client).is_ok() {
                        // 渲染上下文可能尚未创建（惰性化），DPI 一律以窗口为准。
                        let dpi = GetDpiForWindow(hwnd).max(96);
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
                    if let Some(ctx) = state.ctx.as_mut() {
                        let _ = ctx.set_dpi(dpi);
                    }
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
                if ev == TRAY_RBUTTONUP {
                    if let Some(state) = state_mut(hwnd) {
                        let cfg = state.app.config();
                        let lang = state.app.language();
                        TrayIcon::show_menu(hwnd, &cfg, lang);
                    }
                } else if ev == TRAY_LBUTTONDBLCLK {
                    handle_tray_left_event(hwnd, ev);
                } else if is_left_click(ev) {
                    // 双击的尾随 WM_LBUTTONUP 要吞掉，避免再触发一次单击。
                    let swallow = state_mut(hwnd)
                        .is_some_and(|s| before_deadline(GetTickCount(), s.swallow_single_until));
                    if !swallow {
                        handle_tray_left_event(hwnd, ev);
                    }
                }
                LRESULT(0)
            }

            WM_TIMER if wp.0 == TIMER_TRAY_SINGLE => {
                handle_tray_single_timer(hwnd);
                LRESULT(0)
            }

            m if m == WM_SHADEBAR_DIAG => {
                // 采样线程 1 Hz 通知：刷新诊断面板与 S-43 判定色点。
                if let Some(state) = state_mut(hwnd) {
                    // 窗口隐藏时没人看得见界面：跳过整棵视图的重建（文案/布局/诊断串）。
                    // 重新显示时 `paint` 会自己 `sync_view`，不会显示过期内容。
                    // SAFETY: hwnd 有效。
                    let visible = IsWindowVisible(hwnd).as_bool();
                    if visible {
                        state.app.sync_view(&mut state.view);
                    }
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
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let client_dip = (
        w_px as f32 * 96.0 / dpi as f32,
        h_px as f32 * 96.0 / dpi as f32,
    );
    state.view.set_dpi(dpi);
    state.view.set_client(client_dip.0, client_dip.1);
    state.view.set_theme(state.mode);
    let mode = state.view.theme();

    // 惰性创建：`hide_to_tray` 每次都会释放上下文，从托盘恢复后在这里重建。
    if state.ctx.is_none() {
        match RenderCtx::new(hwnd, dpi, (w_px as u32, h_px as u32)) {
            Ok(ctx) => state.ctx = Some(ctx),
            // 建不起来就留白这一帧，下一次 WM_PAINT 再试，不要让窗口卡死。
            Err(_) => return,
        }
    }
    // 字段级分离借用：`view` 与 `ctx` 同属 `state`，必须拆开借。
    let WindowState { ctx, view, .. } = state;
    let Some(ctx) = ctx.as_mut() else {
        return;
    };

    if ctx.begin_frame(w_px, h_px).is_err() {
        return;
    }
    view.paint(ctx, mode.palette(), mode);
    // 设备丢失（D2DERR_RECREATE_TARGET）时目标已重建，必须再画一帧。
    if let Ok(true) = ctx.end_frame() {
        // SAFETY: hwnd 有效。
        let _ = unsafe { InvalidateRect(hwnd, None, false) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Lang;

    /// 组 5 回归：导出成功提示必须带落盘路径，否则用户不知道文件在哪。
    #[test]
    fn export_toast_carries_full_path() {
        let p = Path::new(
            r"C:\Users\me\AppData\Local\ShadeBar\diagnostics\shadebar-diag-1763800000.zip",
        );
        let zh = export_toast_text(Lang::Zh, p);
        assert!(zh.contains("诊断包已导出至"), "缺失中文提示：{zh}");
        assert!(zh.contains("shadebar-diag-1763800000.zip"), "缺失文件名：{zh}");
        let en = export_toast_text(Lang::En, p);
        assert!(en.contains("exported to"), "缺失英文提示：{en}");
    }
}
