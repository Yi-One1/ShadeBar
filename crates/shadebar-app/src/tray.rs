//! 托盘外壳（架构 §8.3 T03.6 / §8.4 T04.1 / FR-07 / A-06 / A-07 / S-40 / S-43）。
//!
//! * 图标**程序生成**（16×16 32bpp DIB → `CreateIconIndirect`）：深/浅任务栏下
//!   轮廓清晰，与 TranslucentTB 的半透明条明显不同（A-06）；6 档尺寸由系统
//!   从同一 HICON 缩放，不依赖外部图标文件。
//! * 菜单 6 项全部生效（A-07）：打开设置 / 暂停自适应（勾选）/ 开机自启动（勾选）/
//!   导出诊断包 / 关于 / 退出。
//! * S-40 左键三选一；S-43 判定色小圆点（节流 1 Hz，由 `App` 通知节奏保证）；
//!   降级时显示「受限」角标与 tip。
//! * 权限模型：全程普通权限，无 `runas`、无提权入口（FR-20 / NFR-11）。

use shadebar_core::color::Rgba;
use shadebar_core::win::to_wide;
use shadebar_core::PRODUCT_NAME;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, GetDC, HDC, HBITMAP, ReleaseDC, RGBQUAD,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, ICONINFO, MF_CHECKED, MF_STRING, SetForegroundWindow, TrackPopupMenu,
    TPM_RIGHTBUTTON,
};

use crate::config::{Config, Lang, SettingKey};
use crate::ipc_server::{WM_SHADEBAR_TRAY, post_to_main};
use crate::ui::i18n;

/// 托盘图标 ID（本进程唯一）。
pub const TRAY_UID: u32 = 1;
/// 托盘菜单命令 ID 基址。
pub const MENU_ID_BASE: u32 = 0x5100;
/// 打开设置。
pub const MENU_OPEN: u32 = 0x5100;
/// 暂停自适应（勾选项）。
pub const MENU_PAUSE: u32 = 0x5101;
/// 开机自启动（勾选项）。
pub const MENU_AUTOSTART: u32 = 0x5102;
/// 导出诊断包。
pub const MENU_EXPORT: u32 = 0x5103;
/// 关于。
pub const MENU_ABOUT: u32 = 0x5104;
/// 退出。
pub const MENU_EXIT: u32 = 0x5105;

/// 图标边长（像素）。
const ICON_SIZE: i32 = 16;

/// 托盘图标句柄的 RAII 状态。
pub struct TrayIcon {
    /// 是否已 `NIM_ADD`。
    added: bool,
    /// 当前图标（`NIM_MODIFY` 换图标时销毁旧的）。
    icon: Option<HICON>,
    /// 上一次成功呈现的外观参数。
    ///
    /// 采样线程 1 Hz 触发一次刷新，但图标只在**判定色/色点/降级态**变化时才需要变。
    /// 不做这个缓存的话，每秒都要重建一次 HICON 并走一次 `Shell_NotifyIcon`
    /// （要过 explorer 的 IPC），实测常驻 CPU 因此被顶到 1% 以上（NFR-03 要求 ≤ 0.1%）。
    last_look: Option<(Rgba, bool, bool, Lang)>,
}

impl TrayIcon {
    /// 创建托盘图标。失败时 `added=false`，调用方仍可继续（不崩溃）。
    #[must_use]
    pub fn add(
        hwnd: HWND,
        decision: Option<Rgba>,
        show_dot: bool,
        degraded: bool,
        lang: Lang,
    ) -> Self {
        let mut tray = Self {
            added: false,
            icon: None,
            last_look: None,
        };
        tray.refresh(hwnd, decision, show_dot, degraded, lang, true);
        tray
    }

    /// 刷新图标 / tip（S-43 判定色点、降级受限角标）。
    pub fn refresh(
        &mut self,
        hwnd: HWND,
        decision: Option<Rgba>,
        show_dot: bool,
        degraded: bool,
        lang: Lang,
        first: bool,
    ) {
        let color = decision.unwrap_or(Rgba::opaque(255, 255, 255));
        // 外观没变就不碰系统：`Shell_NotifyIcon` 要过 explorer 的 IPC，
        // 每秒一次足以把常驻 CPU 顶到 1% 以上。
        let look = (color, show_dot, degraded, lang);
        if !first && self.added && self.last_look == Some(look) {
            return;
        }
        let Some(new_icon) = make_icon(color, show_dot, degraded) else {
            return;
        };
        let tip = if degraded {
            format!("{PRODUCT_NAME} — {}", i18n::text("tray.restricted", lang))
        } else {
            PRODUCT_NAME.to_string()
        };
        let mut data = base_data(hwnd);
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_SHADEBAR_TRAY;
        data.hIcon = new_icon;
        copy_wide(&tip, &mut data.szTip);
        // SAFETY: data 完整初始化，指针在调用期间存活。
        let ok = unsafe { Shell_NotifyIconW(if first { NIM_ADD } else { NIM_MODIFY }, &data) };
        if ok.as_bool() {
            self.added = true;
            self.last_look = Some(look);
            if let Some(old) = self.icon.take() {
                // SAFETY: old 由 CreateIconIndirect 创建，只在此处销毁一次。
                unsafe { DestroyIcon(old) }.ok();
            }
            self.icon = Some(new_icon);
        } else {
            // 添加/更新失败：不保留句柄，避免泄漏。
            // SAFETY: new_icon 是本次生成的图标，未被系统接管。
            unsafe { DestroyIcon(new_icon) }.ok();
        }
    }

    /// 移除托盘图标。
    pub fn remove(&mut self, hwnd: HWND) {
        if self.added {
            let data = base_data(hwnd);
            // SAFETY: 只需 hWnd/uID/cbSize 即可 NIM_DELETE。
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            self.added = false;
        }
        if let Some(icon) = self.icon.take() {
            // SAFETY: 同上。
            unsafe { DestroyIcon(icon) }.ok();
        }
        // 移除后下次 refresh 必须真正重建，不能命中"外观未变"的短路。
        self.last_look = None;
    }

    /// 首启气泡引导用户开启自启（S-02 默认关）。
    pub fn show_balloon(hwnd: HWND, lang: Lang) {
        let mut data = base_data(hwnd);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_wide(PRODUCT_NAME, &mut data.szInfoTitle);
        copy_wide(i18n::text("tray.balloon.first_run", lang), &mut data.szInfo);
        // SAFETY: data 完整初始化。
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    /// 弹出 6 项右键菜单（A-07）。菜单命令由 `WM_COMMAND` 回到 `window.rs` 处理。
    pub fn show_menu(hwnd: HWND, cfg: &Config, lang: Lang) {
        // SAFETY: 无参数。
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };
        let paused = cfg.get(SettingKey::S42) != 0;
        let autostart = cfg.get(SettingKey::S02) != 0;
        let items: [(u32, &str, bool); 6] = [
            (MENU_OPEN, "tray.open", false),
            (MENU_PAUSE, "tray.pause", paused),
            (MENU_AUTOSTART, "tray.autostart", autostart),
            (MENU_EXPORT, "tray.export", false),
            (MENU_ABOUT, "tray.about", false),
            (MENU_EXIT, "tray.exit", false),
        ];
        for (id, key, checked) in items {
            let wide = to_wide(i18n::text(key, lang));
            let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
            // SAFETY: menu 有效；wide 在调用期间存活，末尾含 NUL。
            let _ = unsafe {
                AppendMenuW(menu, flags, id as usize, PCWSTR(wide.as_ptr()))
            };
        }
        let mut pt = POINT::default();
        // SAFETY: pt 为本地可写结构。
        let _ = unsafe { GetCursorPos(&mut pt) };
        // 标准托盘菜单前置动作：先 SetForegroundWindow，再 PostMessage(WM_NULL)。
        // SAFETY: hwnd 有效。
        let _ = unsafe { SetForegroundWindow(hwnd) };
        // SAFETY: menu 有效；RECT 传 None。
        let _ = unsafe {
            TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None)
        };
        let _ = post_to_main(hwnd, 0, 0, 0); // WM_NULL
        // SAFETY: menu 由 CreatePopupMenu 创建，只在此处销毁一次。
        unsafe { DestroyMenu(menu) }.ok();
    }

    /// 是否已成功添加。
    #[must_use]
    pub const fn is_added(&self) -> bool {
        self.added
    }
}

/// S-40 左键行为：0=打开设置，1=打开托盘菜单，2=无操作。
pub fn handle_left_click(hwnd: HWND, cfg: &Config, lang: Lang) {
    match cfg.get(SettingKey::S40) {
        0 => {
            // 交给主窗口的激活消息，与单实例激活走同一条路径。
            let _ = post_to_main(hwnd, crate::ipc_server::WM_SHADEBAR_ACTIVATE, 0, 0);
        }
        1 => TrayIcon::show_menu(hwnd, cfg, lang),
        _ => {}
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
        hWnd: hwnd,
        uID: TRAY_UID,
        ..Default::default()
    }
}

/// 把 UTF-8 文本拷进固定宽字符数组（保留末尾 NUL）。
fn copy_wide(text: &str, dst: &mut [u16]) {
    for (i, &c) in to_wide(text).iter().take(dst.len().saturating_sub(1)).enumerate() {
        dst[i] = c;
    }
    if !dst.is_empty() {
        dst[dst.len() - 1] = 0;
    }
}

/// 程序生成 16×16 BGRA 图标：圆角方块 + 可选判定色小圆点 + 受限角标。
///
/// 用 32bpp `CreateDIBSection` 直接写像素，`CreateIconIndirect` 生成 HICON；
/// 不引入任何图像库，守住体积门禁。
fn make_icon(color: Rgba, show_dot: bool, degraded: bool) -> Option<HICON> {
    // SAFETY: GetDC(NULL) 取屏幕 DC，返回裸 HDC。
    let screen_dc: HDC = unsafe { GetDC(HWND::default()) };
    if screen_dc.is_invalid() {
        return None;
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
            biWidth: ICON_SIZE,
            biHeight: -ICON_SIZE, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: bmi 完整初始化；bits 是本地输出指针；hsection 为 None。
    let color_bmp: HBITMAP =
        match unsafe { CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) } {
            Ok(b) => b,
            Err(_) => {
                // SAFETY: screen_dc 由 GetDC 取得。
                unsafe { ReleaseDC(HWND::default(), screen_dc) };
                return None;
            }
        };
    if bits.is_null() {
        // SAFETY: 两个句柄均为本函数创建。
        unsafe {
            let _ = DeleteObject(color_bmp);
            let _ = ReleaseDC(HWND::default(), screen_dc);
        }
        return None;
    }
    let len = (ICON_SIZE * ICON_SIZE * 4) as usize;
    // SAFETY: bits 指向 CreateDIBSection 分配的 16*16*4 字节，调用期间有效。
    let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), len) };
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let idx = ((y * ICON_SIZE + x) * 4) as usize;
            let inside = (2..14).contains(&x) && (2..14).contains(&y);
            // 圆角：四个角各裁掉一个像素。
            let corner = (x == 2 || x == 13) && (y == 2 || y == 13);
            if !inside || corner {
                pixels[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // 主体：ShadeBar 蓝（BGRA）。
            pixels[idx..idx + 4].copy_from_slice(&[0xD4, 0x78, 0x00, 0xFF]);
        }
    }
    if show_dot {
        // 右下角判定色圆点（S-43）。
        for y in 10..16 {
            for x in 10..16 {
                let dx = x - 13;
                let dy = y - 13;
                if dx * dx + dy * dy > 9 {
                    continue;
                }
                let idx = ((y * ICON_SIZE + x) * 4) as usize;
                if idx + 4 <= pixels.len() {
                    pixels[idx..idx + 4].copy_from_slice(&[color.b, color.g, color.r, 0xFF]);
                }
            }
        }
    }
    if degraded {
        // 受限角标：右上角琥珀色 4×4。
        for y in 2..6 {
            for x in 10..14 {
                let idx = ((y * ICON_SIZE + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&[0x30, 0xC7, 0xFC, 0xFF]);
            }
        }
    }

    // 32bpp alpha 图标的 mask 只需是合法 HBITMAP；内容被忽略。
    // SAFETY: 尺寸与位深合法；lpbits=None 由系统清零。
    let mask: HBITMAP = unsafe { CreateBitmap(ICON_SIZE, ICON_SIZE, 1, 1, None) };
    let info = ICONINFO {
        fIcon: BOOL(1),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color_bmp,
    };
    // SAFETY: info 完整初始化；两个 HBITMAP 有效。
    let icon = unsafe { CreateIconIndirect(&info) }.ok();
    // 系统已复制位图，源位图必须释放，避免每次刷新泄漏两个 GDI 句柄。
    // SAFETY: 均为本函数创建、只在此处释放。
    unsafe {
        let _ = DeleteObject(mask);
        let _ = DeleteObject(color_bmp);
        let _ = ReleaseDC(HWND::default(), screen_dc);
    }
    icon
}

/// 托盘消息 `LPARAM` 低字：左键抬起（`WM_LBUTTONUP`）。
///
/// **不要写成 `0x0402`**：那是 `NIN_BALLOONSHOW`（气泡开始显示），
/// 会让"弹出首启气泡"被当成"用户左键点了托盘"，直接弹出设置窗口（v1.2.0 前的真实缺陷）。
pub const TRAY_LBUTTONUP: u32 = 0x0202;
/// `NIN_SELECT`：新版托盘回调里的"选择"，语义等同左键。
pub const TRAY_NIN_SELECT: u32 = 0x0400;
/// `NIN_KEYSELECT`：键盘回车选中，同样按左键处理。
pub const TRAY_NIN_KEYSELECT: u32 = 0x0401;
/// 托盘消息 `LPARAM` 低字：右键抬起。
pub const TRAY_RBUTTONUP: u32 = 0x0205;

/// 该托盘事件是否应视为"左键点击"。
#[must_use]
pub const fn is_left_click(ev: u32) -> bool {
    matches!(ev, TRAY_LBUTTONUP | TRAY_NIN_SELECT | TRAY_NIN_KEYSELECT)
}

/// 把 `WM_SHADEBAR_TRAY` 的 LPARAM 低字解析成动作。
#[must_use]
pub fn tray_event(lparam: LPARAM) -> u32 {
    (lparam.0 as u32) & 0xFFFF
}

/// 菜单命令 ID（供 `WM_COMMAND` 的 LOWORD 使用）。
#[must_use]
pub const fn menu_id(wparam: WPARAM) -> u32 {
    (wparam.0 as u32) & 0xFFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_menu_ids_are_sequential_and_unique() {
        let ids = [MENU_OPEN, MENU_PAUSE, MENU_AUTOSTART, MENU_EXPORT, MENU_ABOUT, MENU_EXIT];
        assert_eq!(ids.len(), 6);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(*id, MENU_ID_BASE + i as u32);
        }
        assert_eq!(ids[0], 0x5100);
        assert_eq!(ids[5], 0x5105);
    }

    #[test]
    fn copy_wide_never_overruns_and_keeps_nul() {
        let mut buf = [1u16; 8];
        copy_wide("abcdefghijklmnop", &mut buf);
        assert_eq!(buf[7], 0);
        assert_eq!(buf[0], b'a' as u16);
        let mut empty = [1u16; 4];
        copy_wide("", &mut empty);
        assert_eq!(empty[3], 0);
    }

    #[test]
    fn balloon_show_is_not_a_left_click() {
        // 0x0402 是 NIN_BALLOONSHOW，曾被误当作 "左键抬起"：
        // 首启气泡一弹出就弹出设置窗口。这条断言锁死该回归。
        assert_ne!(TRAY_LBUTTONUP, 0x0402);
        assert!(!is_left_click(0x0402), "NIN_BALLOONSHOW 不是左键");
        assert!(is_left_click(0x0202), "WM_LBUTTONUP 是左键");
        assert!(is_left_click(0x0400), "NIN_SELECT 也按左键处理");
        assert!(is_left_click(0x0401), "NIN_KEYSELECT 也按左键处理");
    }

    #[test]
    fn tray_event_extracts_low_word() {
        assert_eq!(tray_event(LPARAM(TRAY_RBUTTONUP as isize)), TRAY_RBUTTONUP);
        assert_eq!(tray_event(LPARAM(0x1234_0205)), TRAY_RBUTTONUP);
    }

    #[test]
    fn menu_command_id_extracts_low_word() {
        assert_eq!(menu_id(WPARAM(MENU_EXIT as usize)), MENU_EXIT);
        assert_eq!(menu_id(WPARAM(0x0001_5100)), MENU_OPEN);
    }

    #[test]
    fn left_click_has_three_documented_modes() {
        let cfg = Config::default();
        assert_eq!(cfg.get(SettingKey::S40), 0);
        let m = SettingKey::S40.meta();
        assert_eq!(m.min, 0);
        assert_eq!(m.max, 2);
    }

    #[test]
    fn icon_size_is_16_pixels() {
        assert_eq!(ICON_SIZE, 16);
        assert_eq!((ICON_SIZE * ICON_SIZE * 4) as usize, 1024);
    }

}
