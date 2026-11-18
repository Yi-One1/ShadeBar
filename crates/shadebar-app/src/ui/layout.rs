//! 纵向流式布局与文本排版（架构 §8.3 T03.2 / §6.4 `layout`）。
//!
//! # 单位纪律（§11.4）
//!
//! 本模块所有坐标/尺寸都是 **dip**；唯一换算点在 `render::RenderCtx::px`。
//! 屏幕物理像素（`RECT` / `i32`）绝不出现在布局计算里。
//!
//! # 判据
//!
//! * 纵向流式 + 分组卡片 + 滚动视口（滚轮 / 拖动 / PageUp/PageDown）；
//! * 长文案自动换行；单行溢出走省略号 + 悬浮 Tooltip；
//! * 720×560 dip 下无横向滚动条、无控件截断（A-08）。


// 布局 API 是完整工具集：部分辅助函数/方法只在单测里被直接调用，
// 但保留它们可避免把布局公式复制进 View（§11.4 单位铁律要求单一换算点）。
#![cfg_attr(not(test), allow(dead_code))]

/// dip 矩形。
#[derive(Debug, Clone, Copy, PartialEq)]
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

    /// 是否包含点。
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.w && y <= self.y + self.h
    }

    /// 向内收缩。
    #[must_use]
    pub const fn inset(&self, d: f32) -> Self {
        Self {
            x: self.x + d,
            y: self.y + d,
            w: (self.w - 2.0 * d).max(0.0),
            h: (self.h - 2.0 * d).max(0.0),
        }
    }
}

/// dip 尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    /// 宽。
    pub w: f32,
    /// 高。
    pub h: f32,
}

impl Size {
    /// 构造。
    #[must_use]
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// 布局度量（全部 dip，与 `ui/theme.rs` 的常量对齐，不另起一套）。
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// 内容区外边距。
    pub margin: f32,
    /// 卡片内边距。
    pub card_pad: f32,
    /// 单行控件高度。
    pub row_h: f32,
    /// 行内说明文字高度。
    pub help_h: f32,
    /// 分组标题高度。
    pub group_h: f32,
    /// 分组间距。
    pub group_gap: f32,
    /// 左侧标签宽度占比（0..1）。
    pub label_ratio: f32,
    /// 标题区高度。
    pub heading_h: f32,
    /// 横幅高度（降级时）。
    pub banner_h: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            margin: 16.0,
            card_pad: 16.0,
            row_h: 34.0,
            help_h: 20.0,
            group_h: 30.0,
            group_gap: 12.0,
            label_ratio: 0.46,
            heading_h: 44.0,
            banner_h: 52.0,
        }
    }
}

impl Metrics {
    /// 单行总高（标签 + 说明）。
    #[must_use]
    pub const fn row_total(&self) -> f32 {
        self.row_h + self.help_h
    }

    /// 给定客户区宽度，算一张卡片的内容宽度。
    #[must_use]
    pub fn content_width(&self, client_w: f32) -> f32 {
        (client_w - self.margin * 2.0 - self.card_pad * 2.0).max(0.0)
    }

    /// 标签列宽。
    #[must_use]
    pub fn label_width(&self, client_w: f32) -> f32 {
        self.content_width(client_w) * self.label_ratio
    }

    /// 控件列宽。
    #[must_use]
    pub fn control_width(&self, client_w: f32) -> f32 {
        (self.content_width(client_w) - self.label_width(client_w)).max(0.0)
    }
}

/// 滚动视口（内容坐标以「内容顶端 = 0」为基准）。
#[derive(Debug, Clone, Copy)]
pub struct Scroll {
    /// 当前滚动偏移（≥0）。
    pub offset: f32,
    /// 内容总高。
    pub content_h: f32,
    /// 视口高。
    pub view_h: f32,
}

impl Default for Scroll {
    fn default() -> Self {
        Self { offset: 0.0, content_h: 0.0, view_h: 0.0 }
    }
}

impl Scroll {
    /// 最大合法偏移。
    #[must_use]
    pub fn max_offset(&self) -> f32 {
        (self.content_h - self.view_h).max(0.0)
    }

    /// 钳到合法区间。
    pub fn clamp(&mut self) {
        self.offset = self.offset.clamp(0.0, self.max_offset());
    }

    /// 滚轮/拖动增量；返回是否真的改变。
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        let before = self.offset;
        self.offset = (self.offset + delta).clamp(0.0, self.max_offset());
        self.offset != before
    }

    /// 翻页（PageUp / PageDown）。
    pub fn page(&mut self, pages: f32) -> bool {
        self.scroll_by(pages * self.view_h * 0.9)
    }

    /// 回到顶部。
    pub fn scroll_to_top(&mut self) {
        self.offset = 0.0;
    }

    /// 把内容坐标换算成客户区 y。
    #[must_use]
    pub fn content_to_view_y(&self, content_y: f32, heading_h: f32) -> f32 {
        heading_h + content_y - self.offset
    }

    /// 客户区 y 换算成内容坐标。
    #[must_use]
    pub fn view_to_content_y(&self, view_y: f32, heading_h: f32) -> f32 {
        view_y - heading_h + self.offset
    }
}

/// 粗略字宽估计：CJK / 全角按 1.0，ASCII 按 0.55。
///
/// 只用于「是否需要省略号 / 在哪里换行」的布局判断，不用于精确排版；
/// 精确排版由 DirectWrite 在 `RenderCtx::draw_text` 里完成。
#[must_use]
pub fn char_width(ch: char) -> f32 {
    let c = ch as u32;
    if c >= 0x2E80 || (0xFF00..=0xFF60).contains(&c) {
        1.0
    } else {
        0.55
    }
}

/// 以「全角字符当量」估算文本宽度。
#[must_use]
pub fn text_width(text: &str) -> f32 {
    text.chars().map(char_width).sum()
}

/// 按最大「全角当量」把长文案折成多行（T03.2 判据②）。
#[must_use]
pub fn wrap_text(text: &str, max_units: f32) -> Vec<String> {
    if max_units <= 0.0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut width = 0.0_f32;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(std::mem::take(&mut current));
            width = 0.0;
            continue;
        }
        let w = char_width(ch);
        if width + w > max_units && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            width = 0.0;
        }
        current.push(ch);
        width += w;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// 单行溢出时生成省略号文本（T03.2 判据②，配合悬浮 Tooltip 显示全文）。
#[must_use]
pub fn ellipsize(text: &str, max_units: f32) -> String {
    if text_width(text) <= max_units {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0.0_f32;
    for ch in text.chars() {
        let w = char_width(ch);
        if width + w + 1.0 > max_units {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_and_inset() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(110.0, 70.0));
        assert!(!r.contains(9.0, 20.0));
        assert_eq!(r.inset(5.0), Rect::new(15.0, 25.0, 90.0, 40.0));
    }

    #[test]
    fn scroll_never_exposes_content_past_bottom() {
        let mut s = Scroll { offset: 0.0, content_h: 2000.0, view_h: 500.0 };
        assert_eq!(s.max_offset(), 1500.0);
        assert!(s.scroll_by(99999.0));
        assert_eq!(s.offset, 1500.0);
        assert!(!s.scroll_by(10.0), "已到底不得再滚");
        assert!(s.scroll_by(-99999.0));
        assert_eq!(s.offset, 0.0);
        s.clamp();
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn scroll_page_and_to_top() {
        let mut s = Scroll { offset: 0.0, content_h: 2000.0, view_h: 500.0 };
        assert!(s.page(1.0));
        assert!((s.offset - 450.0).abs() < 1e-3);
        s.scroll_to_top();
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn content_view_coordinates_roundtrip() {
        let s = Scroll { offset: 120.0, content_h: 1000.0, view_h: 400.0 };
        let y = s.content_to_view_y(300.0, 44.0);
        assert!((s.view_to_content_y(y, 44.0) - 300.0).abs() < 1e-3);
    }

    #[test]
    fn short_content_has_zero_scroll_range() {
        let mut s = Scroll { offset: 50.0, content_h: 300.0, view_h: 500.0 };
        s.clamp();
        assert_eq!(s.max_offset(), 0.0);
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn cjk_is_wider_than_ascii() {
        assert!(char_width('中') > char_width('a'));
        assert!(text_width("中文") > text_width("ab"));
    }

    #[test]
    fn wrap_text_breaks_and_preserves_all_chars() {
        let lines = wrap_text("abcdefghij", 3.0);
        assert!(lines.len() > 1);
        let joined: String = lines.concat();
        assert_eq!(joined, "abcdefghij");
    }

    #[test]
    fn wrap_text_handles_explicit_newline() {
        let lines = wrap_text("ab\ncd", 10.0);
        assert_eq!(lines, vec!["ab".to_string(), "cd".to_string()]);
    }

    #[test]
    fn ellipsize_short_text_is_unchanged() {
        assert_eq!(ellipsize("short", 20.0), "short");
        let long = ellipsize("abcdefghijklmnop", 4.0);
        assert!(long.ends_with('…'));
        assert!(text_width(&long) <= 5.0);
    }

    #[test]
    fn metrics_fit_720x560_without_horizontal_overflow() {
        let m = Metrics::default();
        let w = 720.0_f32;
        assert!(m.content_width(w) > 0.0);
        assert!(m.label_width(w) + m.control_width(w) <= m.content_width(w) + 1e-3);
        assert!(m.control_width(w) > 200.0, "控件列在 720 dip 下必须有足够宽度");
    }
}
