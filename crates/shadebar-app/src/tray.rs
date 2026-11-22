//! 托盘外壳（架构 §8.3 T03.6 / §8.4 T04.1 / FR-07 / A-06 / A-07 / S-40 / S-43）。
//!
//! * 图标**程序生成**（16×16 32bpp DIB → `CreateIconIndirect`）：由 `icon_pixel`
//!   按 A·对分方案的三色几何逐像素绘制，与 `resources/app.ico`（exe / 标题栏）
//!   同构但**不共享同一 HICON**；深/浅任务栏下轮廓清晰（A-06）。
//! * 菜单 6 项全部生效（A-07）：打开设置 / 暂停自适应（勾选）/ 开机自启动（勾选）/
//!   导出诊断包 / 关于 / 退出。
//! * S-40 左键三选一；**S-43 自 G4 起没有任何视觉表现**（用户决定去掉白点，
//!   图标限定黑白 + 深灰三色）。它不能当"仍然生效"糊弄过去：**必须在 G5 重排
//!   设置页时删除或改名**。对外签名与缓存键暂不动，避免扩大改动面；降级只
//!   影响 tip 的「受限」文案。
//! * 权限模型：全程普通权限，无 `runas`、无提权入口（FR-20 / NFR-11）。

use shadebar_core::color::Rgba;
use shadebar_core::win::to_wide;
use shadebar_core::PRODUCT_NAME;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, GetLastError, HWND, LPARAM, POINT, WPARAM};
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
    /// 采样线程 1 Hz 触发一次刷新，但图标只在**状态点/降级态/语言**变化时才需要变。
    /// 判定色自 G4 起不参与图标（A 案限定三色），所以不再进缓存键——否则每秒
    /// 都会重建 HICON 并走一次 `Shell_NotifyIcon`（explorer IPC），常驻 CPU 被顶高。
    last_look: Option<(bool, bool, Lang)>,
}

impl TrayIcon {
    /// 创建托盘图标。失败时 `added=false`，调用方仍可继续（不崩溃）。
    #[must_use]
    pub fn add(
        hwnd: HWND,
        _decision: Option<Rgba>,
        show_dot: bool,
        degraded: bool,
        lang: Lang,
    ) -> Self {
        let mut tray = Self {
            added: false,
            icon: None,
            last_look: None,
        };
        tray.refresh(hwnd, None, show_dot, degraded, lang, true);
        tray
    }

    /// 刷新图标 / tip（降级只改 tip 的「受限」文案）。
    ///
    /// `decision` 与 `show_dot` 保留在公开签名与缓存键里，只是为了避免改动面扩大；
    /// A 案图标限定三色且已去掉状态点，它们**都不再影响像素**。
    /// **S-43 必须在 G5 删除或改名**，别把它当成还生效的设置。
    pub fn refresh(
        &mut self,
        hwnd: HWND,
        _decision: Option<Rgba>,
        show_dot: bool,
        degraded: bool,
        lang: Lang,
        first: bool,
    ) {
        // 外观没变就不碰系统：`Shell_NotifyIcon` 要过 explorer 的 IPC，
        // 每秒一次足以把常驻 CPU 顶到 1% 以上。
        let look = (show_dot, degraded, lang);
        if !needs_shell_call(first, self.added, self.last_look == Some(look)) {
            return;
        }
        let Some(new_icon) = make_icon() else {
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
            // 失败必须把状态退回"未挂载"，否则心跳里的 `first = !is_added()` 永远是
            // false，只会一直发 NIM_MODIFY：图标丢了就再也回不来。
            self.added = false;
            self.last_look = None;
            // SAFETY: 只读线程本地错误码。
            let err = unsafe { GetLastError() };
            log_tray_failure(first, err.0);
        }
    }

    /// explorer 重启后重新对齐图标状态。
    ///
    /// 广播 `TaskbarCreated` 只说明"任务栏重建了"，**不代表我们的图标一定没了**：
    /// 图标若仍在，`NIM_ADD` 会撞上已存在的 uID 直接返回 `E_FAIL`（实测
    /// 0x80004005），于是状态被误判成"未挂载"，心跳开始每秒重试失败。
    /// 所以先按"仍在"处理（`NIM_MODIFY`），失败才当丢失重挂（`NIM_ADD`）。
    pub fn reattach(
        &mut self,
        hwnd: HWND,
        decision: Option<Rgba>,
        show_dot: bool,
        degraded: bool,
        lang: Lang,
    ) {
        // 清缓存键，强制真的走一次 Shell_NotifyIcon。
        self.last_look = None;
        if self.added {
            self.refresh(hwnd, decision, show_dot, degraded, lang, false);
        }
        if !self.added {
            self.refresh(hwnd, decision, show_dot, degraded, lang, true);
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

/// 是否需要真的调用 `Shell_NotifyIcon`。
///
/// 这条短路是 v1.3.2 那个"软件在跑、托盘却没图标"的根因所在：以前写成
/// `!first && added && 外观没变 → 直接 return`，而 explorer 重启后 `added` 仍是
/// `true`、外观也没变，于是一路短路，**永远不会再发 NIM_ADD**，图标再也回不来。
/// 现在只有"已挂载 + 外观没变 + 不强制"三者同时成立才跳过。
fn needs_shell_call(first: bool, added: bool, look_unchanged: bool) -> bool {
    first || !added || !look_unchanged
}

/// 托盘挂载 / 更新失败最多记 3 行：失败时心跳会按 1 Hz 重试，不设预算会刷爆日志。
fn log_tray_failure(first: bool, err: u32) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static BUDGET: AtomicUsize = AtomicUsize::new(3);
    if BUDGET
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
    {
        crate::config::note(&format!(
            "tray {} failed: err={}",
            if first { "NIM_ADD" } else { "NIM_MODIFY" },
            err
        ));
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

/// A·对分图标的三色（BGRA，DIB 口径）；不引入第四色。
/// 深灰 #595959 替代原来的浅灰 #8A8A8A：真机上浅边在白底/深底都会融掉。
const ICON_DARK: [u8; 4] = [0x1A, 0x1A, 0x1A, 0xFF];
const ICON_WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const ICON_GRAY: [u8; 4] = [0x59, 0x59, 0x59, 0xFF];
const ICON_CLEAR: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// 16 px 的轮廓参数：外框 1 px、中缝 2 px、圆角 = 16×22% = 4 px。
const ICON_FRAME: f32 = 1.0;
const ICON_RADIUS: f32 = 4.0;
/// 中缝按 45° 垂直宽度 2 px 计，x+y 方向的间隔是 `2×√2`。
const ICON_SEAM_HALF: f32 = 1.414_213_6; // √2，const 上下文不能调 sqrt()

/// 点是否落在圆角矩形里（角心用 clamp 求，四角才是圆弧而不是尖角）。
fn in_rounded_rect(u: f32, v: f32, lo: f32, hi: f32, r: f32) -> bool {
    let cx = u.clamp(lo + r, hi - r);
    let cy = v.clamp(lo + r, hi - r);
    (u - cx) * (u - cx) + (v - cy) * (v - cy) <= r * r
}

/// A·对分图标的一个像素（BGRA）。**纯函数**：渲染与单测共用同一份几何。
///
/// 16 px 像素中心采样：外框 `[0,16] r=4`、内裁剪 `[1,15] r=3`，两者角心同为
/// `(4,4)` 等，故深灰边等宽 1 px。对角分界 `u+v=16`，左右各留 √2 得到
/// 中缝垂直宽度 2 px。左上深色、右下白色不变；**不画任何状态点**。
///
/// `show_dot` / `degraded` 仅为兼容 `refresh` 的对外签名保留，函数完全忽略：
/// S-43 自 G4 起没有视觉表现，必须在 G5 删除或改名。
#[must_use]
fn icon_pixel(x: usize, y: usize, _show_dot: bool, _degraded: bool) -> [u8; 4] {
    let u = x as f32 + 0.5;
    let v = y as f32 + 0.5;

    if !in_rounded_rect(u, v, 0.0, ICON_SIZE as f32, ICON_RADIUS) {
        return ICON_CLEAR;
    }
    if !in_rounded_rect(
        u,
        v,
        ICON_FRAME,
        ICON_SIZE as f32 - ICON_FRAME,
        ICON_RADIUS - ICON_FRAME,
    ) {
        return ICON_GRAY;
    }
    let total = u + v;
    if total <= ICON_SIZE as f32 - ICON_SEAM_HALF {
        ICON_DARK
    } else if total >= ICON_SIZE as f32 + ICON_SEAM_HALF {
        ICON_WHITE
    } else {
        ICON_GRAY // 中缝：与外框同色，不是黑白主体
    }
}

/// 程序生成 16×16 BGRA 图标：A·对分方砖，无状态点。
///
/// 用 32bpp `CreateDIBSection` 直接写像素，`CreateIconIndirect` 生成 HICON；
/// 不引入任何图像库，守住体积门禁。像素填充全部走 [`icon_pixel`]。
fn make_icon() -> Option<HICON> {
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
            let px = icon_pixel(x as usize, y as usize, false, false);
            let idx = ((y * ICON_SIZE + x) * 4) as usize;
            // BGRA：Windows DIB 是蓝在前，别按 RGB 顺序写。
            pixels[idx..idx + 4].copy_from_slice(&px);
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
    fn shell_call_is_skipped_only_when_mounted_and_unchanged() {
        // 已挂载 + 外观没变 + 不强制：唯一允许跳过 `Shell_NotifyIcon` 的组合
        // （每秒都发会把常驻 CPU 顶到 1% 以上）。
        assert!(!needs_shell_call(false, true, true));

        // 首次挂载、或上次失败后状态已退回未挂载：必须真发（这是 v1.3.2 的修复点）。
        assert!(needs_shell_call(true, true, true), "首次必须发 NIM_ADD");
        assert!(needs_shell_call(true, false, true));
        assert!(needs_shell_call(false, false, true), "未挂载必须重试，不能被缓存短路");

        // explorer 重启后 reattach 会清掉缓存键：即使"已挂载 + 外观没变"也要真发，
        // 由它先用 NIM_MODIFY 探测、失败再 NIM_ADD 补挂。
        assert!(
            needs_shell_call(false, true, false),
            "reattach 清缓存后不得被短路"
        );

        // 外观变化（如降级文案切换）也要真发。
        assert!(needs_shell_call(false, true, false));
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

    #[test]
    fn icon_pixel_corners_are_transparent() {
        for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15)] {
            assert_eq!(icon_pixel(x, y, false, false), ICON_CLEAR, "角落 ({x},{y})");
        }
    }

    #[test]
    fn icon_pixel_is_diagonally_split_dark_upper_left_white_lower_right() {
        // 坐标写死为左上 / 右下，不拿函数自身当期望：深/浅界是 u+v=16。
        assert_eq!(icon_pixel(2, 2, false, false), ICON_DARK);
        assert_eq!(icon_pixel(13, 13, false, false), ICON_WHITE);
        assert_eq!(icon_pixel(4, 4, false, false), ICON_DARK);
        assert_eq!(icon_pixel(11, 11, false, false), ICON_WHITE);
        // 另两角仍归深色侧：分界是斜线，不是竖线/横线
        assert_eq!(icon_pixel(3, 10, false, false), ICON_DARK);
        assert_eq!(icon_pixel(10, 3, false, false), ICON_DARK);
    }

    #[test]
    fn icon_pixel_has_dark_gray_frame_and_two_pixel_diagonal_seam() {
        assert_eq!(icon_pixel(0, 8, false, false), ICON_GRAY, "1 px 深灰外框");
        assert_ne!(ICON_GRAY, [0x8A, 0x8A, 0x8A, 0xFF], "浅灰已弃用");
        // 中轴线 (8,y)：u+v = y+9；15 深、16/17 中缝、18 浅
        assert_eq!(icon_pixel(8, 5, false, false), ICON_DARK);
        assert_eq!(icon_pixel(8, 7, false, false), ICON_GRAY);
        assert_eq!(icon_pixel(8, 8, false, false), ICON_GRAY);
        assert_eq!(icon_pixel(8, 9, false, false), ICON_WHITE);
    }

    #[test]
    fn icon_pixels_ignore_show_dot_and_degraded() {
        // 用户决定彻底去掉白点：任意组合下像素完全一致。
        // S-43 自 G4 起无视觉表现，必须在 G5 删除或改名。
        for y in 0..ICON_SIZE {
            for x in 0..ICON_SIZE {
                let (x, y) = (x as usize, y as usize);
                let base = icon_pixel(x, y, false, false);
                assert_eq!(icon_pixel(x, y, true, false), base, "({x},{y})");
                assert_eq!(icon_pixel(x, y, false, true), base, "({x},{y})");
                assert_eq!(icon_pixel(x, y, true, true), base, "({x},{y})");
            }
        }
    }
}
