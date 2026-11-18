//! T04 真实抓屏：复用 `HDC` + `CreateDIBSection` + `BitBlt`（架构 §4 #42 / T02.2）。
//!
//! # 零堆分配
//!
//! `GdiScreenSource` 持有一次创建的内存 DC / DIB，**只在尺寸变化时重建**；
//! 每次 `grab` 只做 `BitBlt` + 一次 `copy_from_slice` 到 core 的复用
//! [`ScreenBuffer`]，热路径不触碰分配器。
//!
//! # DIB 布局
//!
//! 32bpp BGRA8、`biHeight` 取负 → top-down，与 core `ScreenBuffer` 的行序一致，
//! 因此无需逐行翻转，也不需要 `GetDIBits`。
//!
//! # EX-24 真实失败注入
//!
//! [`GdiScreenSource::set_fault_injection`] 把源 DC 换成 NULL，触发**真实 Win32
//! `BitBlt` 返回 0**（不是假数据），用于验证失败时不崩且保持上次判定。


// 说明：本模块同时服务「正式 release 的 EngineSession」与「verify 取证档」。
// 关闭 verify 时，仅供取证档使用的访问器/字段会变成 dead_code；它们不是废代码，
// 在 --features verify 构建与 cargo test 中都被真实调用。故只在非 verify 构建放宽该 lint。
#![cfg_attr(not(feature = "verify"), allow(dead_code))]

use std::ffi::c_void;

use shadebar_core::engine::{RectPx, ScreenBuffer, ScreenSource, rect_height, rect_width};
use shadebar_core::{SbError, SbResult};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HDC, HBITMAP, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Graphics::Gdi::BitBlt;

use crate::engine::platform::win_err;

/// 真实 GDI 抓屏源。
pub struct GdiScreenSource {
    screen_dc: HDC,
    mem_dc: HDC,
    dib: HBITMAP,
    old_obj: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
    /// 重建次数（验证 600 次采样只建一次）。
    rebuilds: u64,
    /// EX-24 故障注入：把源 DC 置 NULL，触发真实 BitBlt 失败。
    fault_injection: bool,
    /// 累计 BitBlt 调用次数。
    blits: u64,
}

impl Default for GdiScreenSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GdiScreenSource {
    /// 建立抓屏源（DC/DIB 延迟到首次 `grab` 按实际尺寸创建）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            screen_dc: HDC::default(),
            mem_dc: HDC::default(),
            dib: HBITMAP::default(),
            old_obj: HGDIOBJ::default(),
            bits: std::ptr::null_mut(),
            width: 0,
            height: 0,
            rebuilds: 0,
            fault_injection: false,
            blits: 0,
        }
    }

    /// DIB 重建次数。
    #[must_use]
    pub const fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// 累计 BitBlt 次数。
    #[must_use]
    pub const fn blits(&self) -> u64 {
        self.blits
    }

    /// 开关 EX-24 真实失败注入。
    pub fn set_fault_injection(&mut self, on: bool) {
        self.fault_injection = on;
    }

    /// 确保内存 DC / DIB 能容纳 `w × h`。
    ///
    /// # Errors
    ///
    /// `GetDC` / `CreateCompatibleDC` / `CreateDIBSection` 失败时返回
    /// `SbError::BitBltFailed`（保留 `GetLastError()` 原值）。
    fn ensure(&mut self, w: u32, h: u32) -> SbResult<()> {
        if w == 0 || h == 0 {
            return Err(SbError::BitBltFailed { code: 87 });
        }
        if self.width == w && self.height == h && !self.bits.is_null() {
            return Ok(());
        }
        self.release_dib();

        if self.screen_dc.is_invalid() {
            // SAFETY: GetDC(NULL) 取整个虚拟屏 DC，返回裸 HDC（§11.9 不加 ?）。
            self.screen_dc = unsafe { GetDC(HWND::default()) };
            if self.screen_dc.is_invalid() {
                return Err(win_err("GetDC(NULL)"));
            }
        }
        if self.mem_dc.is_invalid() {
            // SAFETY: screen_dc 有效；CreateCompatibleDC 返回裸 HDC。
            self.mem_dc = unsafe { CreateCompatibleDC(self.screen_dc) };
            if self.mem_dc.is_invalid() {
                return Err(win_err("CreateCompatibleDC"));
            }
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // 负高度 = top-down，与 ScreenBuffer 行序一致。
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default(); 1],
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: bmi 完整初始化；bits 为本地输出指针；hsection 传 default(NULL)；
        // CreateDIBSection 返回 Result<HBITMAP>（§11.9）。
        let dib = unsafe {
            CreateDIBSection(
                self.mem_dc,
                &bmi as *const BITMAPINFO,
                DIB_RGB_COLORS,
                &mut bits as *mut *mut c_void,
                windows::Win32::Foundation::HANDLE::default(),
                0,
            )
        }
        .map_err(|_| win_err("CreateDIBSection"))?;
        if bits.is_null() {
            // SAFETY: dib 由本次调用创建，未被选入 DC。
            let _ = unsafe { DeleteObject(dib) };
            return Err(SbError::BitBltFailed {
                code: shadebar_core::win::last_error_code(),
            });
        }
        // SAFETY: dib 有效且尚未被选入；SelectObject 返回旧对象。
        let old = unsafe { SelectObject(self.mem_dc, dib) };
        self.dib = dib;
        self.old_obj = old;
        self.bits = bits.cast::<u8>();
        self.width = w;
        self.height = h;
        self.rebuilds = self.rebuilds.saturating_add(1);
        Ok(())
    }

    /// 释放当前 DIB（尺寸变化时调用），保留 DC 以便复用。
    fn release_dib(&mut self) {
        if !self.dib.is_invalid() {
            if !self.mem_dc.is_invalid() && !self.old_obj.is_invalid() {
                // SAFETY: 恢复旧对象，避免 DC 仍持有即将删除的 DIB。
                unsafe { SelectObject(self.mem_dc, self.old_obj) };
            }
            // SAFETY: dib 由 CreateDIBSection 创建且仅在此处释放一次。
            let _ = unsafe { DeleteObject(self.dib) };
        }
        self.dib = HBITMAP::default();
        self.old_obj = HGDIOBJ::default();
        self.bits = std::ptr::null_mut();
        self.width = 0;
        self.height = 0;
    }
}

impl Drop for GdiScreenSource {
    fn drop(&mut self) {
        self.release_dib();
        if !self.mem_dc.is_invalid() {
            // SAFETY: mem_dc 由 CreateCompatibleDC 创建，仅此一处释放。
            let _ = unsafe { DeleteDC(self.mem_dc) };
        }
        if !self.screen_dc.is_invalid() {
            // SAFETY: screen_dc 由 GetDC(NULL) 取得，按契约 ReleaseDC(NULL, dc)。
            unsafe { ReleaseDC(HWND::default(), self.screen_dc) };
        }
        self.mem_dc = HDC::default();
        self.screen_dc = HDC::default();
    }
}

impl GdiScreenSource {
    /// 抓**本进程自建窗口**的客户区到复用缓冲。
    ///
    /// 与 [`ScreenSource::grab`] 的区别：源 DC 用 `GetDC(hwnd)` 而不是
    /// `GetDC(NULL)`。DWM 下 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` 窗口不一定
    /// 被合成进桌面 DC，从屏幕抓会得到一片白；直接抓窗口自身 DC 才能拿到
    /// `WM_PAINT` 真正绘制的像素（真机取证首轮实测到的缺陷）。
    ///
    /// # Errors
    ///
    /// `GetDC` / `BitBlt` 失败时返回 `SbError::BitBltFailed`。
    pub fn grab_window(
        &mut self,
        hwnd: HWND,
        w: u32,
        h: u32,
        buffer: &mut ScreenBuffer,
    ) -> SbResult<()> {
        if w == 0 || h == 0 {
            return Err(SbError::BitBltFailed { code: 87 });
        }
        self.ensure(w, h)?;
        buffer.ensure(w, h)?;

        // SAFETY: hwnd 由本进程创建且在本调用期间存活；ReleaseDC 成对调用。
        let window_dc = unsafe { GetDC(hwnd) };
        if window_dc.is_invalid() {
            return Err(win_err("GetDC(hwnd)"));
        }
        // SAFETY: mem_dc / window_dc 有效，尺寸与 DIB 一致。
        let ret = unsafe {
            BitBlt(self.mem_dc, 0, 0, w as i32, h as i32, window_dc, 0, 0, SRCCOPY)
        };
        // SAFETY: 与上面的 GetDC 成对。
        unsafe { ReleaseDC(hwnd, window_dc) };
        ret.map_err(|_| SbError::BitBltFailed {
            code: shadebar_core::win::last_error_code(),
        })?;

        // SAFETY: bits 指向 CreateDIBSection 分配的 w*h*4 字节，长度与 buffer 一致。
        let src = unsafe {
            std::slice::from_raw_parts(self.bits, (w as usize).saturating_mul(h as usize) * 4)
        };
        let dst = buffer.as_bytes_mut();
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        Ok(())
    }
}

impl ScreenSource for GdiScreenSource {
    fn grab(&mut self, rect: &RectPx, buffer: &mut ScreenBuffer) -> SbResult<()> {
        let w = rect_width(rect) as u32;
        let h = rect_height(rect) as u32;
        if w == 0 || h == 0 {
            return Err(SbError::BitBltFailed { code: 87 });
        }
        self.ensure(w, h)?;
        buffer.ensure(w, h)?;

        // 故障注入时故意把源 DC 置 NULL：这是**真实** BitBlt 调用失败，
        // 不是返回假错误码；EX-24 的证据由此产生。
        let src_dc = if self.fault_injection {
            HDC::default()
        } else {
            self.screen_dc
        };

        // SAFETY: mem_dc / src_dc 有效或为 NULL（NULL 只会让 BitBlt 返回 0）；
        // 尺寸与 DIB 一致；BitBlt 返回 Result<()>（§11.9）。
        let ret = unsafe {
            BitBlt(
                self.mem_dc,
                0,
                0,
                w as i32,
                h as i32,
                src_dc,
                rect.left,
                rect.top,
                SRCCOPY,
            )
        };
        self.blits = self.blits.saturating_add(1);
        ret.map_err(|_| SbError::BitBltFailed {
            code: shadebar_core::win::last_error_code(),
        })?;

        // SAFETY: bits 指向 CreateDIBSection 分配的 w*h*4 字节，且 DIB 在本对象
        // 生命周期内有效；长度与 buffer 一致（两者都用同一个 w/h ensure）。
        let src = unsafe {
            std::slice::from_raw_parts(self.bits, (w as usize).saturating_mul(h as usize) * 4)
        };
        let dst = buffer.as_bytes_mut();
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        Ok(())
    }
}

/// 把一个 BGRA 像素转成 `#RRGGBB`（证据日志用）。
#[must_use]
pub fn bgra_to_hex(px: &[u8]) -> String {
    if px.len() < 4 {
        return "#??????".to_string();
    }
    format!("#{:02X}{:02X}{:02X}", px[2], px[1], px[0])
}

/// 从 `ScreenBuffer` 取一个像素的 `#RRGGBB`。
#[must_use]
pub fn pixel_at(buffer: &ScreenBuffer, x: u32, y: u32) -> Option<String> {
    let row = buffer.row(y)?;
    let i = (x as usize).saturating_mul(4);
    if i + 4 > row.len() {
        return None;
    }
    Some(bgra_to_hex(&row[i..i + 4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_hex_swaps_channels() {
        assert_eq!(bgra_to_hex(&[0x11, 0x22, 0x33, 0xFF]), "#332211");
    }

    #[test]
    fn bgra_to_hex_rejects_short_slice() {
        assert_eq!(bgra_to_hex(&[1, 2, 3]), "#??????");
    }

    #[test]
    fn pixel_at_reads_top_left() {
        let mut b = ScreenBuffer::new();
        if b.ensure(4, 2).is_err() {
            return;
        }
        b.fill_uniform(0xAA, 0xBB, 0xCC);
        assert_eq!(pixel_at(&b, 0, 0), Some("#AABBCC".to_string()));
        assert_eq!(pixel_at(&b, 3, 1), Some("#AABBCC".to_string()));
        assert_eq!(pixel_at(&b, 4, 0), None);
        assert_eq!(pixel_at(&b, 0, 2), None);
    }

    #[test]
    fn fault_injection_flag_is_observable() {
        let mut s = GdiScreenSource::new();
        assert!(!s.fault_injection);
        s.set_fault_injection(true);
        assert!(s.fault_injection);
        assert_eq!(s.blits(), 0);
        assert_eq!(s.rebuilds(), 0);
    }
}
