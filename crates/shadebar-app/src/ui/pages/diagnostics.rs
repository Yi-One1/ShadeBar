//! 诊断面板的纯数据视图（架构 §8.3 T03.5 / §4 #37）。
//!
//! 数据来自 `app::App` 的共享诊断视图；采样线程每 1 s 通过 `PostMessage`
//! 通知主线程重绘，主线程**不做任何抓屏/采样**，只画已有纯数据。
//! 绘制在 `ui::View::paint_diagnostics`，本文件只负责数据判定。

use shadebar_core::color::Rgba;

use crate::ui::pages::region;

/// 对比度 3:1 的千分比阈值（NFR-17 底线）。
pub const CONTRAST_MIN_MILLI: u32 = 3000;

/// 诊断面板的纯数据视图（主线程只读，采样线程写入）。
#[derive(Debug, Clone, PartialEq)]
pub struct DiagView {
    /// 判定序号。
    pub seq: u64,
    /// 是否已取得至少一拍真机数据。
    pub has_data: bool,
    /// 代表色。
    pub rep_color: Rgba,
    /// 判定色。
    pub decision_color: Rgba,
    /// `y * 1000`。
    pub y_milli: u32,
    /// `contrast * 1000`。
    pub contrast_milli: u32,
    /// 有效像素数。
    pub valid_px: u32,
    /// 本拍耗时（微秒）。
    pub cost_us: u64,
    /// 累计采样次数。
    pub samples: u64,
    /// 最近 10 次 `y*1000`，用于折线。
    pub recent_y: [u32; 10],
    /// 最近值个数。
    pub recent_len: usize,
    /// 最近一次更新的单调毫秒（A-24：语言切换时时间戳必须继续更新）。
    pub timestamp_ms: u64,
}

impl Default for DiagView {
    fn default() -> Self {
        Self {
            seq: 0,
            has_data: false,
            rep_color: Rgba::opaque(0, 0, 0),
            decision_color: Rgba::opaque(255, 255, 255),
            y_milli: 0,
            contrast_milli: 0,
            valid_px: 0,
            cost_us: 0,
            samples: 0,
            recent_y: [0; 10],
            recent_len: 0,
            timestamp_ms: 0,
        }
    }
}

impl DiagView {
    /// 追加一个 Y 样本，保留最近 10 个。
    pub fn push_y(&mut self, y_milli: u32) {
        if self.recent_len < self.recent_y.len() {
            self.recent_y[self.recent_len] = y_milli;
            self.recent_len += 1;
        } else {
            self.recent_y.rotate_left(1);
            self.recent_y[self.recent_y.len() - 1] = y_milli;
        }
    }

    /// 对比度是否低于 3:1（标红）。
    #[must_use]
    pub const fn contrast_low(&self) -> bool {
        self.has_data && self.contrast_milli < CONTRAST_MIN_MILLI
    }

    /// 采样区是否需要「过小」预警（§13-5）。
    #[must_use]
    pub const fn region_small(&self) -> bool {
        region::needs_small_region_warning(self.valid_px, self.has_data)
    }

    /// 采样区是否疑似不在任务栏内（FR-35：有效像素为 0 且已有数据）。
    #[must_use]
    pub const fn region_suspect(&self) -> bool {
        self.has_data && self.valid_px == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_y_keeps_last_10() {
        let mut d = DiagView::default();
        for i in 0..25u32 {
            d.push_y(i * 10);
        }
        assert_eq!(d.recent_len, 10);
        assert_eq!(d.recent_y[0], 150);
        assert_eq!(d.recent_y[9], 240);
    }

    #[test]
    fn contrast_low_requires_real_data() {
        let fresh = DiagView::default();
        assert!(!fresh.contrast_low(), "无数据不得虚报");
        let mut d = DiagView {
            has_data: true,
            contrast_milli: 2999,
            ..Default::default()
        };
        assert!(d.contrast_low());
        d.contrast_milli = 3000;
        assert!(!d.contrast_low());
    }

    #[test]
    fn region_warnings_are_independent() {
        let mut d = DiagView {
            has_data: true,
            valid_px: 0,
            ..Default::default()
        };
        assert!(d.region_suspect());
        assert!(d.region_small());
        d.valid_px = 1499;
        assert!(!d.region_suspect());
        assert!(d.region_small());
        d.valid_px = 1500;
        assert!(!d.region_small());
    }
}
