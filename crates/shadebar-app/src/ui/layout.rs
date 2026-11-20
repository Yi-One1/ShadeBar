//! 设置窗口的几何原语。
//!
//! 布局只在 `ui/mod.rs` 里按页面手工排布（固定 720×560 一屏即可容纳全部内容），
//! 因此这里只留矩形与文字度量，不再有滚动视口/自动排版。

/// 内容坐标下的矩形（dip）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// 左上角 x。
    pub x: f32,
    /// 左上角 y。
    pub y: f32,
    /// 宽。
    pub w: f32,
    /// 高。
    pub h: f32,
}

impl Rect {
    /// 构造。
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// 右边界。
    #[must_use]
    pub const fn right(self) -> f32 {
        self.x + self.w
    }

    /// 下边界。
    #[must_use]
    pub const fn bottom(self) -> f32 {
        self.y + self.h
    }

    /// 是否包含某点。
    #[must_use]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// 向内收缩四边。
    #[must_use]
    pub fn inset(self, d: f32) -> Self {
        Self::new(self.x + d, self.y + d, (self.w - d * 2.0).max(0.0), (self.h - d * 2.0).max(0.0))
    }

}

/// 估算一行文本的宽度（dip）。
///
/// 只用它判断"会不会被截断"和排工具栏按钮，不参与像素级对齐；
/// 真正的排版交给 DirectWrite。全角字符按 1 em，ASCII 按 0.55 em。
#[must_use]
pub fn text_width(text: &str, font_dip: f32) -> f32 {
    text.chars()
        .map(|c| if (c as u32) < 0x2E80 { 0.55 } else { 1.0 } * font_dip)
        .sum()
}

/// 超宽时截断并补省略号（`…`）。
#[must_use]
pub fn ellipsize(text: &str, font_dip: f32, max_w: f32) -> String {
    if text_width(text, font_dip) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = text_width("…", font_dip);
    for c in text.chars() {
        let cw = if (c as u32) < 0x2E80 { 0.55 } else { 1.0 } * font_dip;
        if w + cw > max_w {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// 颜色转 `#RRGGBB` 大写十六进制。
#[must_use]
pub fn hex_rgb(argb: i64) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        (argb >> 16) & 0xFF,
        (argb >> 8) & 0xFF,
        argb & 0xFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_hit_uses_half_open_bounds() {
        let r = Rect::new(10.0, 20.0, 100.0, 40.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(109.9, 59.9));
        assert!(!r.contains(110.0, 40.0));
        assert!(!r.contains(9.9, 30.0));
    }

    #[test]
    fn ellipsize_keeps_short_text_and_marks_long_text() {
        assert_eq!(ellipsize("亮度阈值", 12.0, 200.0), "亮度阈值");
        let cut = ellipsize("这是一段很长的中文标签需要被截断", 12.0, 60.0);
        assert!(cut.ends_with('…'));
        assert!(text_width(&cut, 12.0) <= 60.0);
    }

    #[test]
    fn hex_rgb_ignores_alpha() {
        assert_eq!(hex_rgb(0xFF00_78D4), "#0078D4");
        assert_eq!(hex_rgb(0x8012_3456), "#123456");
    }
}
