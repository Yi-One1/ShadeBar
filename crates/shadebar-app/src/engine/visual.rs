//! T04 取证用**自建**可视化窗口（诊断面板 / 纯色对照背景）。
//!
//! # 环境纪律
//!
//! * 窗口由本进程新建，`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`，不抢焦点、
//!   不进入任务栏、不接收输入；**不触碰任何用户已打开的窗口**。
//! * 不改壁纸、不改注册表、不重启 explorer、不动 TranslucentTB。
//! * 取证结束（[`ProbeWindow::drop`]）立即销毁窗口并注销窗口类。
//! * 任意时刻最多一个探针窗口，绘制数据放 `thread_local`，不跨 FFI 传裸指针
//!   （§11.10 引用生命周期纪律）。
//!
//! # 两种模式
//!
//! * [`ProbeMode::Panel`] —— 画 A-02 几何示意 + 数值文本，用于可视化截图；
//! * [`ProbeMode::Solid`] —— 纯色覆盖采样矩形，用于 A-13/A-14 的真机黑白对照
//!   （真实 `BitBlt` 读到真实屏幕像素，不是假数据；**不是**修改用户壁纸）。

use std::cell::RefCell;

use shadebar_core::engine::{RectPx, rect_height, rect_width};
use shadebar_core::{SbError, SbResult};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
    PAINTSTRUCT, Rectangle, SetBkMode, SetTextColor, TextOutW, TRANSPARENT, UpdateWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetWindowRect, HWND_TOPMOST,
    MSG, PeekMessageW, PM_REMOVE, PostQuitMessage, RegisterClassW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, SW_SHOW, TranslateMessage,
    UnregisterClassW, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR};

/// 自建窗口类名（含进程标识，避免与主窗口类冲突）。
const PROBE_CLASS: &str = "ShadeBar_T04_Probe";

/// 探针窗口默认尺寸（px）。
const PANEL_W: i32 = 760;
const PANEL_H: i32 = 420;

/// 绘制模式。
#[derive(Debug, Clone)]
pub enum ProbeMode {
    /// 诊断面板：文本行 + 托盘/采样矩形示意（坐标相对面板客户区）。
    Panel {
        /// 逐行文本（ASCII，GDI `TextOutW` 直接绘制）。
        lines: Vec<String>,
        /// 示意托盘矩形（相对坐标）。
        tray: RectPx,
        /// 示意采样矩形（相对坐标）。
        sample: RectPx,
    },
    /// 纯色覆盖窗口（用于真机黑/白背景对照）。
    Solid {
        /// BGRA 颜色（GDI `COLORREF` 用 0x00BBGGRR）。
        color: [u8; 4],
    },
}

thread_local! {
    /// 当前探针窗口的绘制数据。窗口与取证循环同线程，生命周期由
    /// [`ProbeWindow`] 严格包住；不使用 `GWLP_USERDATA` 裸指针。
    static PAINT: RefCell<ProbeMode> = const { RefCell::new(ProbeMode::Solid { color: [0, 0, 0, 255] }) };
}

/// 自建探针窗口句柄（RAII）。
pub struct ProbeWindow {
    hwnd: HWND,
    class: HSTRING,
    /// 屏幕绝对矩形（含边框），截图时用它抓屏幕区域。
    pub screen_rect: RectPx,
}

impl ProbeWindow {
    /// 创建并显示一个探针窗口。
    ///
    /// `x` / `y` / `w` / `h` 为屏幕绝对坐标（px）。`Solid` 模式通常传入
    /// 真实采样矩形；`Panel` 模式传入屏幕空白处的一块矩形。
    ///
    /// # Errors
    ///
    /// 窗口类注册 / 窗口创建 / 置顶失败时返回 `SbError`。
    pub fn show(mode: ProbeMode, x: i32, y: i32, w: i32, h: i32) -> SbResult<Self> {
        PAINT.with(|p| *p.borrow_mut() = mode);

        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|e| SbError::HResult {
                op: "GetModuleHandleW",
                hr: e.code().0,
            })?;
        let class = HSTRING::from(PROBE_CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(probe_wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // 类可能已被上一次取证注册过（同一进程内重复运行）；重复注册返回 0
        // 且 GetLastError=ERROR_CLASS_ALREADY_EXISTS，属于可复用状态，不视为失败。
        let atom = unsafe { RegisterClassW(&wc) };
        if atom == 0 {
            let code = shadebar_core::win::last_error_code();
            if code != 1410 {
                // ERROR_CLASS_ALREADY_EXISTS
                return Err(SbError::Win32 {
                    op: "RegisterClassW",
                    code,
                });
            }
        }

        let title = HSTRING::from("ShadeBar T04 verification probe");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                &title,
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                w,
                h,
                HWND::default(),
                windows::Win32::UI::WindowsAndMessaging::HMENU::default(),
                instance,
                None,
            )
        }
        .map_err(|e| SbError::HResult {
            op: "CreateWindowExW",
            hr: e.code().0,
        })?;

        // 明确置顶（TOPMOST 组内也要排到前面），确保 BitBlt 抓到的是本窗口。
        // SAFETY: hwnd 有效；不改变位置/尺寸、不激活。
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )
        };
        // SAFETY: hwnd 有效。
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        // 强制整幅客户区失效并同步重绘：`WS_EX_NOACTIVATE` 窗口刚显示时
        // `WM_PAINT` 的 rcPaint 可能为空，只跑消息泵会留下全白窗口（首轮
        // 真机取证实测到的缺陷）。InvalidateRect(None, true) + UpdateWindow
        // 保证 WM_PAINT 在返回前已经画完，BitBlt 才能抓到真实内容。
        let _ = unsafe { InvalidateRect(hwnd, None, true) };
        let _ = unsafe { UpdateWindow(hwnd) };
        pump_messages(12);

        let mut rect = RECT::default();
        // SAFETY: rect 为本地可写结构，hwnd 有效。
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            rect = RECT {
                left: x,
                top: y,
                right: x.saturating_add(w),
                bottom: y.saturating_add(h),
            };
        }
        Ok(Self {
            hwnd,
            class,
            screen_rect: rect,
        })
    }

    /// 窗口句柄（只读）。
    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 泵若干轮消息，让 `WM_PAINT` 真正落地后再截图。
    pub fn refresh(&self) {
        pump_messages(8);
    }
}

impl Drop for ProbeWindow {
    fn drop(&mut self) {
        // SAFETY: hwnd 由本结构创建且只在此处销毁一次。
        let _ = unsafe { DestroyWindow(self.hwnd) };
        pump_messages(4);
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) };
        if let Ok(inst) = instance {
            // SAFETY: 类名 HSTRING 在本结构生命周期内有效。
            let _ = unsafe { UnregisterClassW(PCWSTR(self.class.as_ptr()), inst) };
        }
        self.hwnd = HWND::default();
    }
}

/// 非阻塞消息泵：把本线程消息队列里的消息分发出去，最多 `rounds` 轮。
fn pump_messages(rounds: usize) {
    let mut msg = MSG::default();
    for _ in 0..rounds {
        // SAFETY: msg 由系统填写；过滤器为空表示收本线程全部消息。
        let got = unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) };
        if !got.as_bool() {
            break;
        }
        // SAFETY: msg 有效。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 窗口过程：只处理绘制与销毁，其余交给 `DefWindowProcW`。
unsafe extern "system" fn probe_wndproc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: hwnd 有效，ps 由系统填写；BeginPaint/EndPaint 成对。
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            // 用「整个客户区」而不是 `ps.rcPaint` 作为填充范围：后者在
            // NOACTIVATE 窗口首帧可能为空，会导致整幅窗口保持白色。
            let mut client = RECT::default();
            if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
                client = ps.rcPaint;
            }
            paint_client(hdc, &client);
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        // 背景完全由 WM_PAINT 铺满，禁止系统擦除造成闪烁。
        WM_ERASEBKGND => LRESULT(1),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// 在 HDC 上绘制探针内容（仅由 `WM_PAINT` 调用）。
fn paint_client(hdc: windows::Win32::Graphics::Gdi::HDC, clip: &RECT) {
    let mode = PAINT.with(|p| p.borrow().clone());
    // SAFETY: hdc 在 BeginPaint/EndPaint 之间有效；所有 GDI 对象即用即删。
    unsafe {
        match mode {
            ProbeMode::Solid { color } => {
                let brush = CreateSolidBrush(colorref(color));
                FillRect(hdc, clip as *const RECT, brush);
                let _ = DeleteObject(brush);
            }
            ProbeMode::Panel { lines, tray, sample } => {
                let bg = CreateSolidBrush(colorref([32, 34, 38, 255]));
                FillRect(hdc, clip as *const RECT, bg);
                let _ = DeleteObject(bg);

                let tray_brush = CreateSolidBrush(colorref([80, 80, 80, 255]));
                let sample_brush = CreateSolidBrush(colorref([255, 220, 60, 255]));
                // 托盘示意：实心灰底 + 采样矩形亮黄描边（用两个矩形叠出 2px 边框）。
                FillRect(hdc, &tray as *const RectPx, tray_brush);
                let outer = inflate(&sample, 2);
                FillRect(hdc, &outer as *const RectPx, sample_brush);
                let inner_brush = CreateSolidBrush(colorref([32, 34, 38, 255]));
                FillRect(hdc, &sample as *const RectPx, inner_brush);
                let _ = DeleteObject(inner_brush);
                let _ = DeleteObject(tray_brush);
                let _ = DeleteObject(sample_brush);

                let _ = SetBkMode(hdc, TRANSPARENT);
                let _ = SetTextColor(hdc, COLORREF(0x00F0_F0F0));
                let mut y = 12i32;
                for line in lines {
                    let wide: Vec<u16> = line.encode_utf16().collect();
                    let _ = TextOutW(hdc, 12, y, &wide);
                    y = y.saturating_add(20);
                    if y > PANEL_H - 20 {
                        break;
                    }
                }
                // 画一个细边框，便于确认整幅客户区确实被绘制（不是空白）。
                let _ = Rectangle(hdc, 0, 0, PANEL_W, PANEL_H);
            }
        }
    }
}

/// BGRA -> COLORREF(0x00BBGGRR)。
#[must_use]
const fn colorref(bgra: [u8; 4]) -> COLORREF {
    COLORREF(
        (bgra[0] as u32) | ((bgra[1] as u32) << 8) | ((bgra[2] as u32) << 16),
    )
}

/// 矩形外扩（用于描边）。
#[must_use]
const fn inflate(r: &RectPx, px: i32) -> RectPx {
    RECT {
        left: r.left.saturating_sub(px),
        top: r.top.saturating_sub(px),
        right: r.right.saturating_add(px),
        bottom: r.bottom.saturating_add(px),
    }
}

/// 把真实几何映射到面板示意坐标。
///
/// 保持宽高比，最长边不超过 `max_w/max_h`；空矩形退化为 1px 点。
#[must_use]
pub fn map_rect_to_panel(real: &RectPx, origin_x: i32, origin_y: i32, max_w: i32, max_h: i32) -> RectPx {
    let rw = rect_width(real).max(1);
    let rh = rect_height(real).max(1);
    let scale = (max_w as f32 / rw as f32).min(max_h as f32 / rh as f32).min(1.0);
    let w = ((rw as f32 * scale) as i32).max(1);
    let h = ((rh as f32 * scale) as i32).max(1);
    RECT {
        left: origin_x,
        top: origin_y,
        right: origin_x.saturating_add(w),
        bottom: origin_y.saturating_add(h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadebar_core::engine::rect_px;

    #[test]
    fn colorref_swaps_channels() {
        assert_eq!(colorref([0x11, 0x22, 0x33, 255]).0, 0x0033_2211);
    }

    #[test]
    fn inflate_expands_all_sides() {
        let r = rect_px(10, 20, 30, 40);
        let o = inflate(&r, 2);
        assert_eq!((o.left, o.top, o.right, o.bottom), (8, 18, 32, 42));
    }

    #[test]
    fn map_rect_keeps_aspect_ratio_and_bounds() {
        let real = rect_px(0, 0, 1000, 100);
        let m = map_rect_to_panel(&real, 20, 200, 600, 200);
        assert_eq!(rect_width(&m), 600);
        assert_eq!(rect_height(&m), 60);
        assert_eq!((m.left, m.top), (20, 200));
    }

    #[test]
    fn map_rect_never_produces_empty_rect() {
        let m = map_rect_to_panel(&rect_px(0, 0, 0, 0), 0, 0, 10, 10);
        assert!(rect_width(&m) >= 1 && rect_height(&m) >= 1);
    }
}
