//! 共享 FFI 小工具（§4 #23）。
//!
//! 只放"跨模块重复率最高、且不该各自实现一遍"的东西：
//!
//! * [`wide!`](crate::wide) / [`to_wide`] / [`from_wide_lossy`] —— UTF-8 ↔ UTF-16 互转；
//! * [`HResultExt`] —— `windows` crate 的 `Result<T>` → [`SbResult<T>`]，保留原始 HRESULT；
//! * [`win32_check`] / [`last_error_code`] —— `BOOL` 返回值 → [`SbResult<()>`]，保留
//!   `GetLastError`。
//!
//! 这里**不做**任何业务判断，也不吞错误（§11.2：可降级错误由调用点显式处理）。

use crate::error::{SbError, SbResult};
use windows::core::Result as WinResult;
use windows::Win32::Foundation::{GetLastError, BOOL, HANDLE};

/// 把 `&str` 转成以 `NUL` 结尾的 UTF-16 缓冲（Win32 的 `LPCWSTR` 形态）。
///
/// 返回 `Vec<u16>` 而不是固定数组：注册表路径、DLL 路径都是动态字符串。
/// 调用点必须在**同一条语句**内取 `.as_ptr()`，否则临时量会在语句结束时被释放：
///
/// ```
/// # use shadebar_core::win::to_wide;
/// # fn takes_ptr(_p: *const u16) {}
/// takes_ptr(to_wide("ShadeBar").as_ptr());
/// ```
#[must_use]
pub fn to_wide(value: &str) -> Vec<u16> {
    // +1 给末尾 NUL。UTF-16 码元数可能多于 char 数（补充平面 2 个码元），
    // 故用 `encode_utf16` 计数而不是 `len()`。
    let mut buf: Vec<u16> = Vec::with_capacity(value.encode_utf16().count() + 1);
    buf.extend(value.encode_utf16());
    buf.push(0);
    buf
}

/// 把以 `NUL` 结尾（或未结尾）的 UTF-16 缓冲转成 `String`，非法码元用 U+FFFD 替换。
///
/// 用于读回 Win32 写出的路径、注册表值、管道名等。**不 panic**：
/// 遇到孤立代理项时 `from_utf16_lossy` 会替换字符，日志里仍可读。
#[must_use]
pub fn from_wide_lossy(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// 构造以 `NUL` 结尾的 UTF-16 缓冲，用法与 `windows::core::w!` 类似但支持运行时字符串。
///
/// ```
/// # use shadebar_core::wide;
/// let name = "ShadeBar";
/// let buf = wide!(name);          // Vec<u16>，末尾含 NUL
/// assert_eq!(buf.last(), Some(&0));
/// ```
#[macro_export]
macro_rules! wide {
    ($s:expr) => {
        $crate::win::to_wide($s)
    };
}

/// `GetLastError()` 的当前值，转成 `u32`。
///
/// 单独封装的原因：`GetLastError` 是 `unsafe fn`，且必须**紧跟在失败的调用之后**
/// 读取（中间任何 Win32 调用都可能覆盖它）。把它包成安全函数，
/// 调用点就不必写 `unsafe`。
#[must_use]
pub fn last_error_code() -> u32 {
    // SAFETY: GetLastError 无参数、无前置条件，只读线程局部状态。
    unsafe { GetLastError().0 }
}

/// 检查 Win32 的 `BOOL` 返回值；失败时读取 `GetLastError()` 构造 [`SbError::Win32`]。
///
/// `op` 必须是固定词干（如 `"OpenProcess"`），用于日志 grep。
pub fn win32_check(op: &'static str, ok: BOOL) -> SbResult<()> {
    if ok.as_bool() {
        Ok(())
    } else {
        Err(SbError::Win32 {
            op,
            code: last_error_code(),
        })
    }
}

/// 检查句柄是否有效（`INVALID_HANDLE_VALUE` / `NULL` 都算失败）。
///
/// 注意：`windows` crate 里大多数返回句柄的 API 已经返回 `Result<HANDLE>`，
/// 这个函数只用于那些"返回 0 表示失败"的旧式 API。
pub fn win32_check_handle(op: &'static str, handle: HANDLE) -> SbResult<HANDLE> {
    if handle.is_invalid() {
        Err(SbError::Win32 {
            op,
            code: last_error_code(),
        })
    } else {
        Ok(handle)
    }
}

/// 把 `windows` crate 的 `Result<T>` 转成本项目的 [`SbResult<T>`]，
/// 并**保留原始 HRESULT**（§11.2 强制要求，否则无法对照 `winerror.h` 定位）。
///
/// 典型用法（示意，不可执行：`windows` 是内部依赖，不出现在公开 API 上）：
///
/// ```ignore
/// CoCreateInstance::<_, ID2D1Factory>(...).sb("CoCreateInstance")?;
/// ```
pub trait HResultExt<T> {
    /// 转换并附带失败的操作名。
    ///
    /// # Errors
    ///
    /// 当底层 `windows` 调用返回 `Err` 时，返回 [`SbError::HResult`]。
    fn sb(self, op: &'static str) -> SbResult<T>;
}

impl<T> HResultExt<T> for WinResult<T> {
    fn sb(self, op: &'static str) -> SbResult<T> {
        self.map_err(|e| SbError::HResult {
            op,
            hr: e.code().0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_wide_appends_nul() {
        let buf = to_wide("abc");
        assert_eq!(buf, vec![0x61, 0x62, 0x63, 0x00]);
    }

    #[test]
    fn to_wide_handles_empty_and_non_bmp() {
        assert_eq!(to_wide(""), vec![0x0000]);
        // U+1F600 在 UTF-16 里是两个码元（代理对），末尾仍只有一个 NUL。
        let buf = to_wide("\u{1F600}");
        assert_eq!(buf.len(), 3);
        assert_eq!(buf[2], 0);
    }

    #[test]
    fn to_wide_roundtrip_with_chinese() {
        let s = "随影 ShadeBar";
        let buf = to_wide(s);
        assert_eq!(from_wide_lossy(&buf), s);
    }

    #[test]
    fn from_wide_lossy_stops_at_first_nul() {
        let buf = [0x41u16, 0x42, 0x00, 0x43, 0x00];
        assert_eq!(from_wide_lossy(&buf), "AB");
    }

    #[test]
    fn from_wide_lossy_replaces_lone_surrogate() {
        // 孤立的高代理项 → 替换为 U+FFFD，不 panic。
        let buf = [0xD800u16, 0x00];
        assert_eq!(from_wide_lossy(&buf), "\u{FFFD}");
    }

    #[test]
    fn win32_check_maps_failure_to_win32_error() {
        match win32_check("TestOp", BOOL(0)) {
            Err(SbError::Win32 { op, .. }) => assert_eq!(op, "TestOp"),
            other => panic!("期望 Win32 错误，实际 {other:?}"),
        }
    }

    #[test]
    fn win32_check_passes_on_true() {
        assert!(win32_check("TestOp", BOOL(1)).is_ok());
    }

    #[test]
    fn win32_check_handle_rejects_invalid_handle() {
        // HANDLE::default() 是 NULL，属无效句柄 → 必须返回 Win32 错误。
        match win32_check_handle("CreateFileW", HANDLE::default()) {
            Err(SbError::Win32 { op, .. }) => assert_eq!(op, "CreateFileW"),
            other => panic!("期望 Win32 错误，实际 {other:?}"),
        }
    }

    #[test]
    fn hresult_ext_preserves_code() {
        let bad: WinResult<()> = Err(windows::core::Error::from_hresult(
            windows::core::HRESULT(0x8004_0102u32 as i32),
        ));
        match bad.sb("RoInitialize") {
            Err(SbError::HResult { op, hr }) => {
                assert_eq!(op, "RoInitialize");
                assert_eq!(hr, 0x8004_0102u32 as i32);
            }
            other => panic!("期望 HResult 错误，实际 {other:?}"),
        }
    }
}
