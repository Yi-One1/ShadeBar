//! 抓屏与屏幕枚举（T02.2，架构 §8.2 / §4 #42）。
//!
//! # 分层
//!
//! 本文件**只定义数据结构、复用缓冲与纯算法**，不含任何 `BitBlt` / `GetDC` 调用。
//! 真实 GDI 抓屏由 [`ScreenSource`] 的实现者提供（T04 在主进程 T1 线程内注入），
//! 单测用 `testing::FakeScreen` 注入假像素——符合 §11.7「单测不得依赖真实屏幕」。
//!
//! # 零堆分配（T02.2 判据①）
//!
//! [`ScreenBuffer`] 在 `ensure` 时按需分配一次，之后 600 次采样只写不分配；
//! 缓冲按 **BGRA8、逐行 top-down、行跨度 `width*4`** 布局，读行用 [`ScreenBuffer::row`]。
//! `windows` 0.58 的 GDI 绑定细节统一封在注入实现里，业务代码不出现 `Result`/裸返回混用
//! （§11.9 统一出口）。

use crate::engine::{
    RectPx, rect_area, rect_height, rect_intersection, rect_is_empty, rect_px, rect_width,
};
use crate::error::{SbError, SbResult};

/// 单块显示器的信息（物理像素）。
///
/// `monitor` 用 `isize` 承载 `HMONITOR`（与 §5.1 `TaskbarGeom.monitor` 一致），
/// 避免 core 依赖 `Win32_Graphics_Gdi` feature。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenInfo {
    /// 显示器整块矩形（含任务栏区）。
    pub bounds: RectPx,
    /// 工作区矩形（排除任务栏）。
    pub work: RectPx,
    /// 该显示器的 DPI（96 = 100%）。
    pub dpi: u32,
    /// `HMONITOR` 句柄值。
    pub monitor: isize,
    /// 是否主显示器。
    pub primary: bool,
}

impl Default for ScreenInfo {
    fn default() -> Self {
        Self {
            bounds: rect_px(0, 0, 0, 0),
            work: rect_px(0, 0, 0, 0),
            dpi: 96,
            monitor: 0,
            primary: false,
        }
    }
}

/// 抓屏平台能力（薄 trait）。
///
/// 实现者负责：把 `rect` 指定的屏幕区域以 **BGRA8 top-down** 写入 `buffer`，
/// 失败时返回 [`SbError::BitBltFailed`]（EX-24：明确错误、不崩、不 panic）。
/// 实现者必须先调用 [`ScreenBuffer::ensure`]，且不得在热路径上分配堆内存。
pub trait ScreenSource {
    /// 抓一帧到复用缓冲。
    ///
    /// # Errors
    ///
    /// `BitBlt` 返回 0、设备上下文丢失、或采样区被完全裁剪到屏幕外时返回错误。
    fn grab(&mut self, rect: &RectPx, buffer: &mut ScreenBuffer) -> SbResult<()>;
}

/// 复用的 BGRA8 像素缓冲（几何不变时零堆分配）。
pub struct ScreenBuffer {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    ensure_count: u64,
}

impl Default for ScreenBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenBuffer {
    /// 建立空缓冲（首次 `ensure` 时分配）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            width: 0,
            height: 0,
            ensure_count: 0,
        }
    }

    /// 确保缓冲能容纳 `width × height` 的 BGRA8 帧。
    ///
    /// 尺寸未变时**只做一次比较就返回**，不触碰堆分配器；尺寸变化时才重分配并清零。
    /// 这是 T02.2「DIB/HDC 只创建一次」在 core 侧的对应物。
    ///
    /// # Errors
    ///
    /// 尺寸为 0 或行跨度溢出时返回 [`SbError::SampleRegionTooSmall`]。
    pub fn ensure(&mut self, width: u32, height: u32) -> SbResult<()> {
        if width == 0 || height == 0 {
            return Err(SbError::SampleRegionTooSmall { valid_px: 0 });
        }
        let stride = bgra_stride(width);
        let total = stride.checked_mul(height as usize).ok_or(
            SbError::SampleRegionTooSmall { valid_px: 0 },
        )?;
        if self.width == width && self.height == height && self.bytes.len() >= total {
            return Ok(());
        }
        self.bytes.clear();
        self.bytes.resize(total, 0);
        self.width = width;
        self.height = height;
        self.ensure_count += 1;
        Ok(())
    }

    /// 帧宽（像素）。
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 帧高（像素）。
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 行跨度（字节）。
    #[must_use]
    pub const fn stride(&self) -> usize {
        bgra_stride(self.width)
    }

    /// 缓冲实际重分配次数（单测用来证明"600 次零分配"）。
    #[must_use]
    pub const fn ensure_count(&self) -> u64 {
        self.ensure_count
    }

    /// 取第 `y` 行的 BGRA 字节切片；越界返回 `None`。
    #[must_use]
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        let stride = self.stride();
        let start = (y as usize).checked_mul(stride)?;
        let end = start.checked_add(stride)?;
        self.bytes.get(start..end)
    }

    /// 全部字节（BGRA8 top-down）。
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 全部字节的可变视图（BGRA8 top-down）。
    ///
    /// T04 的真实 GDI 适配用它把 `CreateDIBSection` 拿到的像素**直接拷进复用缓冲**，
    /// 避免再分配一份 `Vec`。调用方必须写入恰好 `stride * height` 字节；
    /// 尺寸不符时应先调 [`ScreenBuffer::ensure`]。
    #[must_use]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// 用不透明纯色填满缓冲（假数据源与诊断缩略图共用）。
    ///
    /// `r/g/b` 为 sRGB gamma 编码的 0..=255；alpha 恒写 255。
    pub fn fill_uniform(&mut self, r: u8, g: u8, b: u8) {
        let (pixels, _tail) = self.bytes.as_chunks_mut::<4>();
        for px in pixels {
            px[0] = b;
            px[1] = g;
            px[2] = r;
            px[3] = 255;
        }
    }

    /// 生成 A-28 用的污染帧：黑底上按行周期插入 `white_per_100`% 的纯白像素。
    ///
    /// 用于验证 P50 中位数对 ≤20% 字符污染的免疫（§13-1 拍板口径）。
    pub fn fill_black_with_white_noise(&mut self, white_per_100: u32) {
        let ratio = white_per_100.min(100);
        let stride = self.stride();
        for (y, row) in self.bytes.chunks_exact_mut(stride).enumerate() {
            let (pixels, _tail) = row.as_chunks_mut::<4>();
            for (x, px) in pixels.iter_mut().enumerate() {
                // 用整数散列把白像素均匀铺开，避免整行/整列聚簇导致 P50 失去代表性。
                let hash = (x as u32)
                    .wrapping_mul(2_654_435_761)
                    .wrapping_add((y as u32).wrapping_mul(2_246_822_519));
                let is_white = hash % 100 < ratio;
                let v = if is_white { 255 } else { 0 };
                px[0] = v;
                px[1] = v;
                px[2] = v;
                px[3] = 255;
            }
        }
    }

    /// 把缓冲恢复为全 0（不释放内存，供异常帧复用）。
    pub fn clear_pixels(&mut self) {
        self.bytes.fill(0);
    }
}

/// BGRA8 行跨度：`width * 4`，用 `usize` 运算避免 `u32` 溢出。
#[must_use]
pub const fn bgra_stride(width: u32) -> usize {
    (width as usize).saturating_mul(4)
}

/// 把采样矩形裁剪到**含任务栏的那块屏**（NFR-07 / EX-24）。
///
/// 优先取主显示器（`primary == true`）的交集：任务栏默认在主屏，这样即使采样矩形
/// 因 DPI/折叠误差跨到副屏边缘，也不会因为"副屏交集面积更大"而误采到副屏像素
/// （EX-08 硬要求）。主屏无交集时（任务栏被用户移到副屏）退化为取面积最大的交集。
/// 完全落在所有屏幕之外时返回 `None`。
///
/// 注意：闭环主路径不调用本函数——`sample_once` 用的是已由
/// `TaskbarGeom::from_probe` 选好的 `geom.screen` 单屏裁剪。本函数是给 T04 的
/// 真实 GDI 注入实现做二次兜底用的。
#[must_use]
pub fn clip_rect_to_screens(rect: &RectPx, screens: &[ScreenInfo]) -> Option<RectPx> {
    if let Some(primary) = screens.iter().find(|s| s.primary) {
        if let Some(part) = rect_intersection(rect, &primary.bounds) {
            return Some(part);
        }
    }
    let mut best: Option<RectPx> = None;
    let mut best_area: u32 = 0;
    for s in screens {
        if let Some(part) = rect_intersection(rect, &s.bounds) {
            let area = rect_area(&part);
            if area > best_area {
                best_area = area;
                best = Some(part);
            }
        }
    }
    best
}

/// 生成诊断面板缩略图：最近邻缩放，BGRA8 → `0xAARRGGBB` 的 `u32`。
///
/// 复杂度 O(`tw*th`)，无堆分配；T02.2 判据「≤32 µs」由调用点在 Debug 日志里实测，
/// 本函数只保证结构上不分配、不做浮点运算。返回 `false` 表示缓冲为空或输出尺寸非法。
pub fn thumbnail_into(buffer: &ScreenBuffer, out: &mut [u32], tw: usize, th: usize) -> bool {
    if tw == 0 || th == 0 || out.len() < tw.saturating_mul(th) {
        return false;
    }
    let (w, h) = (buffer.width() as usize, buffer.height() as usize);
    if w == 0 || h == 0 {
        return false;
    }
    for dy in 0..th {
        let sy = (dy * h / th).min(h - 1);
        let Some(row) = buffer.row(sy as u32) else {
            return false;
        };
        for dx in 0..tw {
            let sx = (dx * w / tw).min(w - 1);
            let i = sx * 4;
            if i + 3 >= row.len() {
                return false;
            }
            let (b, g, r) = (u32::from(row[i]), u32::from(row[i + 1]), u32::from(row[i + 2]));
            out[dy * tw + dx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
    true
}

/// 计算矩形宽高并做合法性检查（抓屏前的最后一道防线）。
///
/// # Errors
///
/// 空矩形返回 [`SbError::SampleRegionTooSmall`]；不在这里静默跳过，
/// 由调用点按 §13-5「保持上次判定 + Debug 日志」处理。
#[must_use]
pub fn checked_frame_size(rect: &RectPx) -> Option<(u32, u32)> {
    if rect_is_empty(rect) {
        return None;
    }
    let w = rect_width(rect) as u32;
    let h = rect_height(rect) as u32;
    Some((w, h))
}

/// 把任意 `SbError` 归一为抓屏失败码（供注入实现写日志用）。
///
/// 已是 `BitBltFailed` 时原样返回，其余按 `code = 0` 记录，
/// 保证采样链路只出现一种抓屏错误词干（§11.1 便于 grep）。
#[must_use]
pub fn as_bitblt_failure(err: SbError) -> SbError {
    match err {
        SbError::BitBltFailed { code } => SbError::BitBltFailed { code },
        _ => SbError::BitBltFailed { code: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{FakeScreen, alloc_count, reset_alloc_count};

    #[test]
    fn buffer_ensure_only_allocates_on_size_change() {
        let mut buf = ScreenBuffer::new();
        assert_eq!(buf.ensure_count(), 0);
        for _ in 0..600 {
            buf.ensure(300, 48).expect("ensure 应成功");
        }
        assert_eq!(buf.ensure_count(), 1, "几何不变时 600 次不得重分配");
        buf.ensure(320, 48).expect("尺寸变化应重分配");
        assert_eq!(buf.ensure_count(), 2);
        assert_eq!(buf.width(), 320);
        assert_eq!(buf.stride(), 320 * 4);
    }

    #[test]
    fn buffer_rejects_zero_size_without_panic() {
        let mut buf = ScreenBuffer::new();
        assert!(buf.ensure(0, 10).is_err());
        assert!(buf.ensure(10, 0).is_err());
        assert!(buf.row(0).is_none());
    }

    #[test]
    fn six_hundred_grabs_are_allocation_free() {
        // T02.2 判据①：GlobalAlloc 计数器证明热路径零堆分配。
        let mut src = FakeScreen::uniform(0, 0, 0);
        let mut buf = ScreenBuffer::new();
        let rect = rect_px(0, 0, 300, 48);
        src.grab(&rect, &mut buf).expect("预热帧");
        let expected = buf.ensure_count();

        reset_alloc_count();
        for _ in 0..600 {
            let r = src.grab(&rect, &mut buf);
            assert!(r.is_ok(), "600 次抓屏不得失败");
        }
        assert_eq!(alloc_count(), 0, "600 次采样出现堆分配");
        assert_eq!(buf.ensure_count(), expected, "DIB 不得被重建");
    }

    #[test]
    fn bitblt_failure_returns_error_not_panic() {
        // EX-24：抓屏失败返回明确错误，调用点可保持上次判定。
        let mut src = FakeScreen::failing(87);
        let mut buf = ScreenBuffer::new();
        let rect = rect_px(0, 0, 300, 48);
        assert!(
            matches!(src.grab(&rect, &mut buf), Err(SbError::BitBltFailed { code }) if code == 87),
            "EX-24：应返回 BitBltFailed(87) 而不是崩溃"
        );
    }

    #[test]
    fn clip_fully_offscreen_is_rejected() {
        let screens = [ScreenInfo {
            bounds: rect_px(0, 0, 1920, 1080),
            work: rect_px(0, 0, 1920, 1032),
            dpi: 96,
            monitor: 1,
            primary: true,
        }];
        let off = rect_px(2000, 2000, 2100, 2100);
        assert_eq!(clip_rect_to_screens(&off, &screens), None);
        assert!(!rect_is_empty(&off), "矩形本身有效，只是完全落在屏外");
    }

    #[test]
    fn clip_partial_overlap_prefers_primary_screen() {
        // 主屏 + 右侧副屏：采样矩形横跨两屏时，即便副屏交集面积更大也取主屏
        // （NFR-07 / EX-08：绝不因面积更大而误采副屏像素）。
        let screens = [
            ScreenInfo {
                bounds: rect_px(0, 0, 1920, 1080),
                work: rect_px(0, 0, 1920, 1032),
                dpi: 96,
                monitor: 1,
                primary: true,
            },
            ScreenInfo {
                bounds: rect_px(1920, 0, 3840, 1080),
                work: rect_px(1920, 0, 3840, 1032),
                dpi: 96,
                monitor: 2,
                primary: false,
            },
        ];
        let rect = rect_px(1900, 1000, 1960, 1060);
        assert_eq!(
            clip_rect_to_screens(&rect, &screens),
            Some(rect_px(1900, 1000, 1920, 1060))
        );
    }

    #[test]
    fn thumbnail_is_nearest_neighbour_and_argb() {
        let mut buf = ScreenBuffer::new();
        buf.ensure(4, 2).expect("ensure");
        buf.fill_uniform(0x11, 0x22, 0x33);
        let mut out = [0u32; 4];
        assert!(thumbnail_into(&buf, &mut out, 2, 2));
        assert_eq!(out[0], 0xFF11_2233);
        assert_eq!(out[3], 0xFF11_2233);
        let mut small = [0u32; 1];
        assert!(!thumbnail_into(&buf, &mut small, 2, 2));
        assert!(!thumbnail_into(&buf, &mut out, 0, 2));
    }

    #[test]
    fn white_noise_ratio_is_bounded() {
        let mut buf = ScreenBuffer::new();
        buf.ensure(300, 48).expect("ensure");
        buf.fill_black_with_white_noise(20);
        let (pixels, _tail) = buf.as_bytes().as_chunks::<4>();
        let white = pixels
            .iter()
            .filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
            .count();
        let total = 300 * 48;
        let pct = white * 100 / total;
        assert!((15..=25).contains(&pct), "白像素占比应接近 20%，实际 {pct}%");
    }

    #[test]
    fn checked_frame_size_matches_rect() {
        assert_eq!(checked_frame_size(&rect_px(0, 0, 300, 48)), Some((300, 48)));
        assert_eq!(checked_frame_size(&rect_px(5, 5, 5, 48)), None);
    }
}
