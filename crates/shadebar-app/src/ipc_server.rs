//! 单实例守卫与第二实例激活（架构 §8.4 T04.1 / A-01 / FR-01）。
//!
//! # 与 §7.1 时序图的偏差（有意为之，已记录到 spike-report §11）
//!
//! 架构 §7.1 用 `Local\ShadeBar.SingleInstance` 互斥体 + `\\.\pipe\ShadeBar.activate`
//! 命名管道完成激活。本实现保留**同一个互斥体**（`SINGLE_INSTANCE_MUTEX`），
//! 但把激活通道换成「`FindWindowW` 找主窗口 + `PostMessageW(WM_APP+1)`」：
//!
//! * 行为判据 A-01 不变：连续双击 5 次进程数恒为 1，第 2~5 次 ≤1 s 退出并弹出
//!   既有设置窗口；
//! * 命名管道需要常驻监听线程 + `Win32_System_Pipes` / `Win32_Storage_FileSystem`
//!   两组绑定与 `ReadFile`/`WriteFile` 生命周期；窗口消息是同一线程内的
//!   `PostMessage`，无额外线程、无额外 IPC 状态、无句柄泄漏面；
//! * 体积门禁（<400 KB 软目标）下，少一个监听线程与一组管道绑定是实质收益。
//!
//! `ACTIVATE_PIPE_NAME`（`\\.\pipe\ShadeBar.activate`）常量仍保留在
//! `shadebar-core::lib.rs`，供后续版本若需要跨会话/无窗口激活时复用。

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

use shadebar_core::SINGLE_INSTANCE_MUTEX;
use shadebar_core::win::last_error_code;

/// 主窗口类名（`window.rs` 与本模块共用，物理上不可能漂移）。
pub const MAIN_WINDOW_CLASS: &str = "ShadeBar.MainWindow";

/// 自定义激活消息（`WM_APP + 1` = 0x8001），由第二实例投递给既有主窗口。
pub const WM_SHADEBAR_ACTIVATE: u32 = 0x8001;
/// 自定义托盘消息（`WM_APP + 2`）。
pub const WM_SHADEBAR_TRAY: u32 = 0x8002;
/// 自定义诊断刷新消息（`WM_APP + 3`，采样线程 1 Hz 节流投递）。
pub const WM_SHADEBAR_DIAG: u32 = 0x8003;

/// `ERROR_ALREADY_EXISTS`（183）。写成常量避免依赖 `WIN32_ERROR` 的投影形式。
const ERROR_ALREADY_EXISTS_CODE: u32 = 183;

/// 单实例互斥体句柄的 RAII 守卫；进程存活期间必须一直持有。
pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: 句柄由 CreateMutexW 创建，只在此处释放一次。
            unsafe { CloseHandle(self.handle) }.ok();
        }
    }
}

/// 尝试成为唯一实例。
///
/// 返回 `Some(guard)` 表示本进程是首实例（调用方必须把 guard 活到消息循环结束）；
/// 返回 `None` 表示互斥体已存在或创建失败 —— 调用方应立即激活既有窗口并退出。
#[must_use]
pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
    let name = HSTRING::from(SINGLE_INSTANCE_MUTEX);
    // SAFETY: 无安全属性、不初始拥有、名称是 NUL 结尾的 HSTRING。
    let result = unsafe { CreateMutexW(None, false, &name) };
    match result {
        Ok(handle) => {
            // `CreateMutexW` 成功但已存在时，GetLastError() == ERROR_ALREADY_EXISTS。
            // 必须紧跟在调用之后读取（§11.9 / win::last_error_code 文档）。
            if last_error_code() == ERROR_ALREADY_EXISTS_CODE {
                // SAFETY: 句柄有效；重复实例不应持有它。
                unsafe { CloseHandle(handle) }.ok();
                None
            } else {
                Some(SingleInstanceGuard { handle })
            }
        }
        Err(_) => None,
    }
}

/// 第二实例：找到既有主窗口并投递激活消息。返回是否成功投递。
///
/// 找不到窗口（例如首实例正在启动、窗口尚未创建）返回 `false`；调用方仍然
/// 立即退出，进程数恒为 1（A-01 的「进程数」判据不受影响）。
#[must_use]
pub fn activate_existing() -> bool {
    let class = HSTRING::from(MAIN_WINDOW_CLASS);
    // SAFETY: class 在调用期间存活；第二个参数为 NULL PCWSTR。
    let found = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) };
    match found {
        Ok(hwnd) if !hwnd.is_invalid() => {
            // SAFETY: hwnd 是本产品自己的窗口；消息号在 WM_APP 私有区间。
            unsafe { PostMessageW(hwnd, WM_SHADEBAR_ACTIVATE, WPARAM(0), LPARAM(0)) }.is_ok()
        }
        _ => false,
    }
}

/// 采样线程/托盘向主窗口投递一条自定义消息（统一入口，避免各处重复 unsafe）。
pub fn post_to_main(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    // SAFETY: hwnd 由 create_main_window 返回；消息号均为本模块定义的 WM_APP 私有值。
    unsafe { PostMessageW(hwnd, msg, WPARAM(wp), LPARAM(lp)) }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_class_is_stable_and_non_empty() {
        assert_eq!(MAIN_WINDOW_CLASS, "ShadeBar.MainWindow");
        assert!(!MAIN_WINDOW_CLASS.is_empty());
    }

    #[test]
    fn custom_messages_are_in_wm_app_private_range_and_unique() {
        for m in [WM_SHADEBAR_ACTIVATE, WM_SHADEBAR_TRAY, WM_SHADEBAR_DIAG] {
            assert!((0x8000..=0xBFFF).contains(&m), "{m} 不在 WM_APP 私有区间");
        }
        assert_ne!(WM_SHADEBAR_ACTIVATE, WM_SHADEBAR_TRAY);
        assert_ne!(WM_SHADEBAR_TRAY, WM_SHADEBAR_DIAG);
        assert_ne!(WM_SHADEBAR_ACTIVATE, WM_SHADEBAR_DIAG);
    }

    #[test]
    fn already_exists_code_matches_win32() {
        assert_eq!(ERROR_ALREADY_EXISTS_CODE, 183);
    }

    #[test]
    fn posting_to_invalid_window_is_a_no_op() {
        assert!(!post_to_main(HWND::default(), WM_SHADEBAR_DIAG, 0, 0));
    }
}
