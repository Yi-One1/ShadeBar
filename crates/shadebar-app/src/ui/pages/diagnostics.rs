//! 页面 6：诊断面板（架构 §8.3 T03.5 / §4 #37）。
//!
//! 判据：①缩略图实时刷新 ≤1 Hz 且仅面板可见时重绘 ②代表色 HEX / 判定色 /
//! Y(3 位小数) / 对比度 ③最近 10 次采样折线 ④对比度 <3:1 标红（NFR-17）
//! ⑤采样区疑似不在任务栏内给警告（FR-35）⑥面板可见时 UI 线程 CPU <0.5%。
//!
//! 数据来自 `app::App` 的共享诊断视图；采样线程每 1 s 通过 `PostMessage` 通知
//! 主线程重绘，主线程**不做任何抓屏/采样**，只画已有纯数据，故 CPU 开销可忽略。

use shadebar_core::color::Rgba;

use crate::config::Lang;
use crate::render::{RectDip, RenderCtx, TextRole};
use crate::ui::i18n;
use crate::ui::pages::region;
use crate::ui::theme::{Palette, CARD_RADIUS_DIP};

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

fn hex(color: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

/// 画一张诊断卡片。
pub fn paint(ctx: &RenderCtx, palette: &Palette, lang: Lang, rect: RectDip, diag: &DiagView) {
    ctx.fill_rounded_rect(rect, CARD_RADIUS_DIP, palette.card_bg);
    ctx.stroke_rounded_rect(rect, CARD_RADIUS_DIP, palette.card_border, 1.0);
    let x = rect.x + 16.0;
    let mut y = rect.y + 12.0;
    let row_h = 22.0_f32;
    let w = (rect.w - 32.0).max(0.0);

    ctx.draw_text(
        i18n::text("diag.title", lang),
        RectDip::new(x, y, w, row_h),
        TextRole::Title,
        palette.text_primary,
    );
    y += row_h + 4.0;

    if !diag.has_data {
        ctx.draw_text(
            i18n::text("diag.no_data", lang),
            RectDip::new(x, y, w, row_h),
            TextRole::Body,
            palette.text_secondary,
        );
        return;
    }

    let rows: Vec<(String, String)> = vec![
        (i18n::text("diag.rep_color", lang).to_string(), hex(diag.rep_color)),
        (i18n::text("diag.decision_color", lang).to_string(), hex(diag.decision_color)),
        (i18n::text("diag.y", lang).to_string(), format!("{:.3}", f64::from(diag.y_milli) / 1000.0)),
        (i18n::text("diag.contrast", lang).to_string(), format!("{:.3}:1", f64::from(diag.contrast_milli) / 1000.0)),
        (i18n::text("diag.valid_px", lang).to_string(), diag.valid_px.to_string()),
        (i18n::text("diag.cost_us", lang).to_string(), diag.cost_us.to_string()),
        (i18n::text("diag.samples", lang).to_string(), diag.samples.to_string()),
        (i18n::text("diag.timestamp", lang).to_string(), format!("{} ms", diag.timestamp_ms)),
    ];
    for (label, value) in &rows {
        let is_contrast = label == i18n::text("diag.contrast", lang);
        let color = if is_contrast {
            // 达标显示正常态绿色，未达 3:1 标红（NFR-17）。
            if diag.contrast_low() { palette.danger } else { palette.ok }
        } else {
            palette.text_secondary
        };
        ctx.draw_text(label, RectDip::new(x, y, w * 0.5, row_h), TextRole::Body, color);
        ctx.draw_text(value, RectDip::new(x + w * 0.5, y, w * 0.5, row_h), TextRole::Body, color);
        y += row_h;
    }

    // 最近 10 次采样折线（用等宽小竖条表示，避免引入几何图元依赖）。
    let chart_top = y + row_h + 4.0;
    let chart_h = 40.0_f32;
    let chart_w = w.min(320.0);
    ctx.draw_text(
        i18n::text("diag.trend", lang),
        RectDip::new(x, y, w, row_h),
        TextRole::Body,
        palette.text_secondary,
    );
    let bar_w = chart_w / 10.0;
    for i in 0..diag.recent_len {
        let v = diag.recent_y[i].min(1000) as f32 / 1000.0;
        let bh = (chart_h * v).max(2.0);
        ctx.fill_rect(
            RectDip::new(x + i as f32 * bar_w + 1.0, chart_top + chart_h - bh, bar_w - 2.0, bh),
            palette.accent,
        );
    }
    y = chart_top + chart_h + 8.0;

    if diag.contrast_low() {
        ctx.draw_text(
            i18n::text("diag.contrast_low", lang),
            RectDip::new(x, y, w, row_h),
            TextRole::Body,
            palette.danger,
        );
        y += row_h;
    }
    if diag.region_suspect() {
        ctx.draw_text(
            i18n::text("diag.region_suspect", lang),
            RectDip::new(x, y, w, row_h),
            TextRole::Body,
            palette.warning,
        );
        y += row_h;
    }
    if diag.region_small() {
        // T03.4 判据⑥：采样区过小红字预警，但不禁用该组合。
        ctx.draw_text(
            region::warning_text(lang),
            RectDip::new(x, y, w, row_h),
            TextRole::Body,
            palette.danger,
        );
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

    #[test]
    fn hex_is_uppercase_six_digits() {
        assert_eq!(hex(Rgba::opaque(0x0A, 0xBC, 0x0F)), "#0ABC0F");
    }
}
