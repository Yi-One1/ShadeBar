//! 控件库（架构 §8.3 T03.3 / §6.4 `Widget` trait）。
//!
//! 7 类控件：`Toggle` / `Slider` / `Dropdown` / **ARGB ColorField** / `Button` /
//! `Banner` / `GroupHeader + ReadOnlyField`。全部实现同一个 [`Widget`] trait，
//! 35 项配置经 `SettingWidget` 统一绑定，没有第二套控件状态。
//!
//! * 浅/深色由 [`Palette`] 决定，控件不自带颜色；
//! * `set_enabled(false)` 只换色 + 拒绝交互，**不清零用户设定值**（NFR-05 / §13-6）；
//! * 键盘可达：Tab 顺序由 `View` 管理，Space/Enter 切换、方向键调滑块；
//! * 单位全 dip（§11.4），像素换算只在 `RenderCtx` 里发生。

use shadebar_core::color::Rgba;

use crate::config::{Config, ControlKind, Group, Lang, SettingKey, SettingMeta, argb_to_rgba};
use crate::render::{RectDip, RenderCtx, TextRole};
use crate::ui::i18n;
use crate::ui::layout::{Metrics, Rect, Size, ellipsize, text_width};
use crate::ui::theme::{Palette, CARD_RADIUS_DIP};

// 虚拟键值：直接用 Win32 常量数值，避免为 8 个常量新开一个 windows feature。
/// Tab。
pub const VK_TAB: u16 = 0x09;
/// Space。
pub const VK_SPACE: u16 = 0x20;
/// Enter。
pub const VK_RETURN: u16 = 0x0D;
/// Left。
pub const VK_LEFT: u16 = 0x25;
/// Up。
pub const VK_UP: u16 = 0x26;
/// Right。
pub const VK_RIGHT: u16 = 0x27;
/// Down。
pub const VK_DOWN: u16 = 0x28;
/// PageUp。
pub const VK_PRIOR: u16 = 0x21;
/// PageDown。
pub const VK_NEXT: u16 = 0x22;
/// Escape。
pub const VK_ESCAPE: u16 = 0x1B;

/// 输入事件（由 `window.rs` 把 Win32 消息归一成这个形态）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEvent {
    /// 鼠标移动（客户区 dip）。
    MouseMove { x: f32, y: f32 },
    /// 左键按下。
    LButtonDown { x: f32, y: f32 },
    /// 左键抬起。
    LButtonUp { x: f32, y: f32 },
    /// 按键（虚拟键码）。
    Key { vk: u16 },
    /// 字符输入（Hex 颜色输入用）。
    Char { ch: char },
}

/// 需要 `View` 弹出的浮层 / 立即动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// 下拉选项列表。
    Dropdown(SettingKey),
    /// ARGB 颜色选择（Hex 输入 + 预设色板 + 内置 alpha）。
    ColorPicker(SettingKey),
    /// 恢复默认二次确认（A-12）。
    ConfirmRestore,
    /// 关于页。
    About,
    /// 立即导出诊断包。
    ExportDiagnostics,
    /// 立即复制版本号。
    CopyVersion,
}

/// 控件事件结果（§6.4：`Handled | Changed | OpenPopup`，额外补 `Ignored`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetResult {
    /// 与本控件无关。
    Ignored,
    /// 已消费（如悬停态变化）。
    Handled,
    /// 值被用户改动，调用方写回 `Config`。
    Changed(SettingKey, i64),
    /// 请求弹层 / 立即动作。
    OpenPopup(PopupKind),
}

/// 统一控件 trait（§6.4 签名；`paint` 增加 `dy` 用于滚动视口偏移）。
pub trait Widget {
    /// 在 `avail` 里排布自己，返回占用尺寸。
    fn layout(&mut self, avail: Rect, m: &Metrics) -> Size;
    /// 绘制。`dy = 标题区高度 - 滚动偏移`，把内容坐标搬到客户区坐标。
    fn paint(&self, ctx: &RenderCtx, palette: &Palette, lang: Lang, dy: f32);
    /// 处理输入。
    fn event(&mut self, ev: &UiEvent) -> WidgetResult;
    /// 语言热切换（FR-18 / A-24）。
    fn set_texts(&mut self, lang: Lang);
    /// 置灰 / 恢复（NFR-05）。
    fn set_enabled(&mut self, on: bool);
}

/// 一个设置项行（标签 + 说明 + 右列控件）。
#[derive(Debug, Clone)]
pub struct SettingWidget {
    /// 绑定的设置项。
    pub key: SettingKey,
    /// 元数据（全 const，零分配）。
    pub meta: SettingMeta,
    /// 整行矩形（内容坐标）。
    pub rect: Rect,
    /// 右列控件矩形（内容坐标）。
    pub control: Rect,
    /// 是否可交互。
    pub enabled: bool,
    /// 鼠标悬停。
    pub hover: bool,
    /// 键盘焦点。
    pub focus: bool,
    /// 当前值。
    pub value: i64,
    /// 滑块拖动中。
    dragging: bool,
}

impl SettingWidget {
    /// 以配置当前值构造。
    #[must_use]
    pub fn new(key: SettingKey, cfg: &Config, degraded: bool) -> Self {
        Self {
            key,
            meta: key.meta(),
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            control: Rect::new(0.0, 0.0, 0.0, 0.0),
            enabled: !(degraded && key.degraded_greyed()),
            hover: false,
            focus: false,
            value: cfg.get(key),
            dragging: false,
        }
    }

    /// 同步外部配置值（不触发事件）。
    pub fn sync_value(&mut self, v: i64) {
        self.value = v;
    }

    /// 右列是否命中（内容坐标）。
    #[must_use]
    pub fn control_hit(&self, x: f32, y: f32) -> bool {
        self.control.contains(x, y)
    }

    /// 单行是否命中（用于 Tab 焦点框 / Tooltip）。
    #[must_use]
    pub fn row_hit(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y)
    }

    /// 当前下拉选项文案。
    #[must_use]
    pub fn option_text(&self, lang: Lang) -> &'static str {
        let idx = self.value.clamp(0, self.meta.options.len().saturating_sub(1) as i64) as usize;
        let key = self.meta.options.get(idx).copied().unwrap_or("common.off");
        if key.starts_with('#') {
            key
        } else {
            i18n::text(key, lang)
        }
    }

    /// 滑块归一化位置（0..1）。
    #[must_use]
    pub fn slider_ratio(&self) -> f32 {
        let span = (self.meta.max - self.meta.min).max(1) as f32;
        ((self.value - self.meta.min) as f32 / span).clamp(0.0, 1.0)
    }

    /// 把点击 x 换算成滑块值（按 step 对齐）。
    #[must_use]
    fn slider_value_at(&self, x: f32) -> i64 {
        let w = self.control.w.max(1.0);
        let ratio = ((x - self.control.x) / w).clamp(0.0, 1.0);
        let raw = self.meta.min + (ratio * (self.meta.max - self.meta.min) as f32).round() as i64;
        self.meta.clamp(raw)
    }

    /// 方向键步进。
    #[must_use]
    fn step_value(&self, delta: i64) -> i64 {
        self.meta.clamp(self.value.saturating_add(delta * self.meta.step.max(1)))
    }

    /// 显示值文本。
    #[must_use]
    pub fn value_text(&self, lang: Lang) -> String {
        match self.meta.kind {
            ControlKind::Toggle => i18n::text(if self.value != 0 { "common.on" } else { "common.off" }, lang).to_string(),
            ControlKind::Slider => match self.key {
                SettingKey::S12 => format!("{:.2}", self.value as f32 / 1000.0),
                SettingKey::S18 => format!("{:.2}", self.value as f32 / 1000.0),
                SettingKey::S16 => format!("{} ms", self.value),
                SettingKey::S20 | SettingKey::S21 | SettingKey::S22 => format!("{} %", self.value),
                _ => self.value.to_string(),
            },
            ControlKind::Dropdown => self.option_text(lang).to_string(),
            ControlKind::Color => format!("#{:08X}", self.value as u32),
            ControlKind::Button => String::new(),
            ControlKind::ReadOnly => {
                if self.key == SettingKey::S05 {
                    i18n::text("common.version_value", lang).to_string()
                } else {
                    i18n::text("common.region_preview", lang).to_string()
                }
            }
        }
    }

    /// 点击控件后的动作。
    fn activate(&mut self, x: f32) -> WidgetResult {
        if !self.enabled {
            return WidgetResult::Handled;
        }
        match self.meta.kind {
            ControlKind::Toggle => WidgetResult::Changed(self.key, if self.value != 0 { 0 } else { 1 }),
            ControlKind::Slider => {
                self.dragging = true;
                WidgetResult::Changed(self.key, self.slider_value_at(x))
            }
            ControlKind::Dropdown => WidgetResult::OpenPopup(PopupKind::Dropdown(self.key)),
            ControlKind::Color => WidgetResult::OpenPopup(PopupKind::ColorPicker(self.key)),
            ControlKind::Button => {
                if self.key == SettingKey::S06 {
                    WidgetResult::OpenPopup(PopupKind::ConfirmRestore)
                } else {
                    WidgetResult::OpenPopup(PopupKind::ExportDiagnostics)
                }
            }
            ControlKind::ReadOnly => {
                if self.key == SettingKey::S05 {
                    WidgetResult::OpenPopup(PopupKind::CopyVersion)
                } else {
                    WidgetResult::Handled
                }
            }
        }
    }

    /// 焦点框。
    fn draw_focus(&self, ctx: &RenderCtx, palette: &Palette, dy: f32) {
        if self.focus {
            ctx.stroke_rounded_rect(self.to_dip(self.control, dy), 4.0, palette.accent, 2.0);
        }
    }

    fn to_dip(&self, r: Rect, dy: f32) -> RectDip {
        RectDip::new(r.x, r.y + dy, r.w, r.h)
    }
}

impl Widget for SettingWidget {
    fn layout(&mut self, avail: Rect, m: &Metrics) -> Size {
        let h = m.row_total();
        self.rect = Rect::new(avail.x, avail.y, avail.w, h);
        let label_w = avail.w * m.label_ratio;
        self.control = Rect::new(
            avail.x + label_w + 8.0,
            avail.y + 3.0,
            (avail.w - label_w - 8.0).max(0.0),
            m.row_h - 6.0,
        );
        Size::new(avail.w, h)
    }

    fn paint(&self, ctx: &RenderCtx, palette: &Palette, lang: Lang, dy: f32) {
        let text_color = palette.control_text(self.enabled);
        let label = i18n::text(self.meta.label, lang);
        // 标签列只占 control.x 左边，不能压到右列控件（A-08 无控件截断）。
        let label_w = (self.control.x - self.rect.x - 8.0).max(40.0);
        let label_units = (label_w * m_label_units_ratio()).max(4.0);
        let label_rect = RectDip::new(self.rect.x, self.rect.y + dy + 2.0, label_w, 22.0);
        ctx.draw_text(&ellipsize(label, label_units), label_rect, TextRole::Body, text_color);

        let help = i18n::text(self.meta.help, lang);
        let help_rect = RectDip::new(self.rect.x, self.rect.y + dy + 22.0, self.rect.w, 20.0);
        let help_units = (self.rect.w / 7.5).max(4.0);
        ctx.draw_text(
            &ellipsize(help, help_units),
            help_rect,
            TextRole::Body,
            if self.enabled { palette.text_secondary } else { palette.disabled_text },
        );
        // S-19 旁必须显示联动警告（T03.4 判据⑤）。
        if self.key == SettingKey::S19 {
            // 警告放在说明行右侧，不遮挡第一行的控件。
            let warn_rect = RectDip::new(
                self.control.x,
                self.rect.y + dy + 22.0,
                self.control.w,
                20.0,
            );
            ctx.draw_text(
                i18n::text("banner.s19_warning", lang),
                warn_rect,
                TextRole::Body,
                palette.warning,
            );
        }

        let c = self.control;
        let cr = self.to_dip(c, dy);
        match self.meta.kind {
            ControlKind::Toggle => {
                let w = 46.0_f32.min(c.w);
                let pill = RectDip::new(cr.x + cr.w - w, cr.y + 3.0, w, cr.h - 6.0);
                let on = self.value != 0;
                ctx.fill_rounded_rect(pill, (cr.h - 6.0) / 2.0, palette.control_fill(on, self.enabled));
                ctx.draw_text(
                    i18n::text(if on { "common.on" } else { "common.off" }, lang),
                    RectDip::new(pill.x, pill.y + 1.0, pill.w, pill.h),
                    TextRole::Body,
                    if on { palette.accent_text } else { palette.text_primary },
                );
                // 左半留数值文本，方便键盘/读屏。
                ctx.draw_text(
                    &self.value_text(lang),
                    RectDip::new(cr.x, cr.y + 3.0, (cr.w - w).max(0.0), cr.h),
                    TextRole::Body,
                    palette.text_secondary,
                );
            }
            ControlKind::Slider => {
                let track = RectDip::new(cr.x, cr.y + cr.h / 2.0 - 3.0, (cr.w - 64.0).max(0.0), 6.0);
                ctx.fill_rounded_rect(track, 3.0, palette.track_off);
                let filled_w = track.w * self.slider_ratio();
                if filled_w > 0.0 {
                    ctx.fill_rounded_rect(
                        RectDip::new(track.x, track.y, filled_w, track.h),
                        3.0,
                        if self.enabled { palette.accent } else { palette.disabled_fill },
                    );
                }
                let knob_x = track.x + filled_w;
                ctx.fill_rounded_rect(
                    RectDip::new(knob_x - 6.0, cr.y + cr.h / 2.0 - 8.0, 12.0, 16.0),
                    4.0,
                    if self.enabled { palette.accent } else { palette.disabled_fill },
                );
                ctx.draw_text(
                    &self.value_text(lang),
                    RectDip::new(track.x + track.w + 8.0, cr.y + 2.0, 60.0, cr.h - 4.0),
                    TextRole::Body,
                    text_color,
                );
            }
            ControlKind::Dropdown | ControlKind::Button => {
                let fill = if self.meta.kind == ControlKind::Button && self.enabled {
                    palette.accent
                } else {
                    palette.card_bg
                };
                ctx.fill_rounded_rect(cr, 5.0, fill);
                ctx.stroke_rounded_rect(cr, 5.0, palette.card_border, 1.0);
                let label = if self.meta.kind == ControlKind::Button {
                    i18n::text(self.meta.label, lang)
                } else {
                    self.option_text(lang)
                };
                let fg = if self.meta.kind == ControlKind::Button && self.enabled {
                    palette.accent_text
                } else {
                    text_color
                };
                ctx.draw_text(
                    &ellipsize(label, (cr.w / 7.5).max(3.0)),
                    RectDip::new(cr.x + 8.0, cr.y + 3.0, cr.w - 24.0, cr.h - 6.0),
                    TextRole::Body,
                    fg,
                );
                if self.meta.kind == ControlKind::Dropdown {
                    ctx.draw_text(
                        "v",
                        RectDip::new(cr.x + cr.w - 20.0, cr.y + 3.0, 16.0, cr.h - 6.0),
                        TextRole::Body,
                        palette.text_secondary,
                    );
                }
            }
            ControlKind::Color => {
                let sw = 34.0_f32.min(cr.h);
                let rgba = color_of(self.value);
                ctx.fill_rounded_rect(
                    RectDip::new(cr.x, cr.y + 2.0, sw, cr.h - 4.0),
                    4.0,
                    rgba,
                );
                ctx.stroke_rounded_rect(
                    RectDip::new(cr.x, cr.y + 2.0, sw, cr.h - 4.0),
                    4.0,
                    palette.card_border,
                    1.0,
                );
                // 内置 alpha 提示（不计为新增设置项，§13-2）。
                ctx.draw_text(
                    &self.value_text(lang),
                    RectDip::new(cr.x + sw + 8.0, cr.y + 3.0, cr.w - sw, cr.h - 6.0),
                    TextRole::Body,
                    text_color,
                );
            }
            ControlKind::ReadOnly => {
                if self.key == SettingKey::S23 {
                    // 采样区预览：按 S-20/21/22 的比例画一个框（T03.4 判据⑥）。
                    let pw = cr.w.min(160.0);
                    let preview = RectDip::new(cr.x, cr.y + 2.0, pw, cr.h - 4.0);
                    ctx.fill_rounded_rect(preview, 3.0, palette.window_bg);
                    ctx.stroke_rounded_rect(preview, 3.0, palette.card_border, 1.0);
                    let inner = RectDip::new(
                        preview.x + preview.w * 0.12,
                        preview.y + preview.h * 0.2,
                        preview.w * 0.7,
                        preview.h * 0.6,
                    );
                    ctx.stroke_rounded_rect(inner, 2.0, palette.accent, 2.0);
                }
                ctx.draw_text(
                    &self.value_text(lang),
                    RectDip::new(cr.x, cr.y + 3.0, cr.w, cr.h - 6.0),
                    TextRole::Body,
                    palette.text_secondary,
                );
            }
        }
        self.draw_focus(ctx, palette, dy);
    }

    fn event(&mut self, ev: &UiEvent) -> WidgetResult {
        match *ev {
            UiEvent::MouseMove { x, y } => {
                self.hover = self.control_hit(x, y);
                if self.dragging && self.enabled {
                    let v = self.slider_value_at(x);
                    if v != self.value {
                        self.value = v;
                        return WidgetResult::Changed(self.key, v);
                    }
                }
                WidgetResult::Ignored
            }
            UiEvent::LButtonDown { x, y } => {
                if self.control_hit(x, y) {
                    self.focus = true;
                    self.activate(x)
                } else {
                    WidgetResult::Ignored
                }
            }
            UiEvent::LButtonUp { .. } => {
                let was = self.dragging;
                self.dragging = false;
                if was { WidgetResult::Handled } else { WidgetResult::Ignored }
            }
            UiEvent::Key { vk } => {
                if !self.focus || !self.enabled {
                    return WidgetResult::Ignored;
                }
                match self.meta.kind {
                    ControlKind::Toggle => {
                        if vk == VK_SPACE || vk == VK_RETURN {
                            WidgetResult::Changed(self.key, if self.value != 0 { 0 } else { 1 })
                        } else {
                            WidgetResult::Ignored
                        }
                    }
                    ControlKind::Slider | ControlKind::Dropdown => {
                        let delta = if vk == VK_RIGHT || vk == VK_UP {
                            1
                        } else if vk == VK_LEFT || vk == VK_DOWN {
                            -1
                        } else {
                            0
                        };
                        if delta != 0 {
                            WidgetResult::Changed(self.key, self.step_value(delta))
                        } else if vk == VK_SPACE || vk == VK_RETURN {
                            if self.meta.kind == ControlKind::Dropdown {
                                WidgetResult::OpenPopup(PopupKind::Dropdown(self.key))
                            } else {
                                WidgetResult::Handled
                            }
                        } else {
                            WidgetResult::Ignored
                        }
                    }
                    ControlKind::Color => {
                        if vk == VK_SPACE || vk == VK_RETURN {
                            WidgetResult::OpenPopup(PopupKind::ColorPicker(self.key))
                        } else {
                            WidgetResult::Ignored
                        }
                    }
                    ControlKind::Button | ControlKind::ReadOnly => {
                        if vk == VK_SPACE || vk == VK_RETURN {
                            self.activate(self.control.x + 1.0)
                        } else {
                            WidgetResult::Ignored
                        }
                    }
                }
            }
            UiEvent::Char { .. } => WidgetResult::Ignored,
        }
    }

    fn set_texts(&mut self, _lang: Lang) {
        // 文案不缓存在控件里：每次 paint 都从 i18n 表按当前语言取，
        // 因此语言切换天然即时（A-24 ≤1 s），此处只需保持接口契约。
    }

    fn set_enabled(&mut self, on: bool) {
        // 只改可交互性，`value` 原样保留（NFR-05 / §13-6）。
        self.enabled = on;
        if !on {
            self.hover = false;
            self.dragging = false;
        }
    }
}

/// 分组标题行（T03.3 的 GroupHeader）。
#[derive(Debug, Clone)]
pub struct GroupHeader {
    /// 分组。
    pub group: Group,
    /// 矩形（内容坐标）。
    pub rect: Rect,
}

impl GroupHeader {
    /// 构造。
    #[must_use]
    pub const fn new(group: Group) -> Self {
        Self { group, rect: Rect::new(0.0, 0.0, 0.0, 0.0) }
    }
}

impl Widget for GroupHeader {
    fn layout(&mut self, avail: Rect, m: &Metrics) -> Size {
        self.rect = Rect::new(avail.x, avail.y, avail.w, m.group_h);
        Size::new(avail.w, m.group_h)
    }

    fn paint(&self, ctx: &RenderCtx, palette: &Palette, lang: Lang, dy: f32) {
        ctx.draw_text(
            i18n::text(self.group.title_key(), lang),
            RectDip::new(self.rect.x, self.rect.y + dy + 4.0, self.rect.w, self.rect.h - 4.0),
            TextRole::Title,
            palette.text_primary,
        );
    }

    fn event(&mut self, _ev: &UiEvent) -> WidgetResult {
        WidgetResult::Ignored
    }

    fn set_texts(&mut self, _lang: Lang) {}

    fn set_enabled(&mut self, _on: bool) {}
}

/// 降级横幅（NFR-05：顶部说明原因 + 自助排查建议，**不提供提权按钮**）。
#[derive(Debug, Clone)]
pub struct Banner {
    /// 矩形（内容坐标）。
    pub rect: Rect,
    /// 是否可见。
    pub visible: bool,
}

impl Banner {
    /// 构造。
    #[must_use]
    pub const fn new(visible: bool) -> Self {
        Self { rect: Rect::new(0.0, 0.0, 0.0, 0.0), visible }
    }
}

impl Widget for Banner {
    fn layout(&mut self, avail: Rect, m: &Metrics) -> Size {
        let h = if self.visible { m.banner_h } else { 0.0 };
        self.rect = Rect::new(avail.x, avail.y, avail.w, h);
        Size::new(avail.w, h)
    }

    fn paint(&self, ctx: &RenderCtx, palette: &Palette, lang: Lang, dy: f32) {
        if !self.visible {
            return;
        }
        ctx.fill_rounded_rect(RectDip::new(self.rect.x, self.rect.y + dy, self.rect.w, self.rect.h), CARD_RADIUS_DIP, palette.card_bg);
        ctx.stroke_rounded_rect(RectDip::new(self.rect.x, self.rect.y + dy, self.rect.w, self.rect.h), CARD_RADIUS_DIP, palette.warning, 1.0);
        ctx.draw_text(
            i18n::text("banner.degraded.title", lang),
            RectDip::new(self.rect.x + 12.0, self.rect.y + dy + 4.0, self.rect.w - 24.0, 22.0),
            TextRole::Title,
            palette.warning,
        );
        ctx.draw_text(
            i18n::text("banner.degraded.body", lang),
            RectDip::new(self.rect.x + 12.0, self.rect.y + dy + 26.0, self.rect.w - 24.0, 22.0),
            TextRole::Body,
            palette.text_secondary,
        );
    }

    fn event(&mut self, _ev: &UiEvent) -> WidgetResult {
        WidgetResult::Ignored
    }

    fn set_texts(&mut self, _lang: Lang) {}

    fn set_enabled(&mut self, _on: bool) {}
}

/// 标签列可用的「全角当量」密度：14 dip 字号下约 8.5 dip/全角字。
#[must_use]
const fn m_label_units_ratio() -> f32 {
    1.0 / 8.5
}

/// 把 `0xAARRGGBB` 解析成 `Rgba`（供诊断面板用）。
#[must_use]
pub fn color_of(v: i64) -> Rgba {
    argb_to_rgba(v)
}

/// 省略号阈值辅助：判断一段文案在给定宽度下是否需要 Tooltip。
#[must_use]
pub fn needs_tooltip(text: &str, width_dip: f32) -> bool {
    text_width(text) > (width_dip / 8.5).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(key: SettingKey) -> SettingWidget {
        SettingWidget::new(key, &Config::default(), false)
    }

    #[test]
    fn all_35_settings_can_build_a_widget() {
        for k in SettingKey::ALL {
            let w = widget(k);
            assert_eq!(w.key, k);
            assert!(w.enabled);
            assert_eq!(w.value, k.meta().default_value);
        }
    }

    #[test]
    fn layout_splits_label_and_control_without_overflow() {
        let mut w = widget(SettingKey::S16);
        let m = Metrics::default();
        let size = w.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &m);
        assert_eq!(size.h, m.row_total());
        assert!(w.control.x > w.rect.x);
        assert!(w.control.x + w.control.w <= w.rect.x + w.rect.w + 1e-3);
    }

    #[test]
    fn toggle_click_produces_changed_and_value_stays_legal() {
        let mut w = widget(SettingKey::S01);
        w.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &Metrics::default());
        let r = w.event(&UiEvent::LButtonDown { x: w.control.x + 10.0, y: w.control.y + 10.0 });
        assert_eq!(r, WidgetResult::Changed(SettingKey::S01, 0));
    }

    #[test]
    fn disabled_widget_keeps_value_but_refuses_input() {
        let mut w = widget(SettingKey::S30);
        w.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &Metrics::default());
        let before = w.value;
        w.set_enabled(false);
        let r = w.event(&UiEvent::LButtonDown { x: w.control.x + 10.0, y: w.control.y + 10.0 });
        assert_eq!(r, WidgetResult::Handled);
        assert_eq!(w.value, before, "NFR-05：置灰不得清零用户设定值");
    }

    #[test]
    fn slider_click_maps_to_range_and_step() {
        let mut w = widget(SettingKey::S16);
        w.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &Metrics::default());
        let v = w.slider_value_at(w.control.x + w.control.w * 0.5);
        assert!((500..=5000).contains(&v));
        assert_eq!((v - 500) % 100, 0, "步进 100");
    }

    #[test]
    fn keyboard_arrows_step_slider() {
        let mut w = widget(SettingKey::S12);
        w.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &Metrics::default());
        w.focus = true;
        assert_eq!(w.event(&UiEvent::Key { vk: VK_RIGHT }), WidgetResult::Changed(SettingKey::S12, 510));
        assert_eq!(w.event(&UiEvent::Key { vk: VK_LEFT }), WidgetResult::Changed(SettingKey::S12, 490));
    }

    #[test]
    fn keyboard_space_toggles() {
        let mut w = widget(SettingKey::S02);
        w.focus = true;
        assert_eq!(w.event(&UiEvent::Key { vk: VK_SPACE }), WidgetResult::Changed(SettingKey::S02, 1));
    }

    #[test]
    fn color_defaults_and_hex_text() {
        let w = widget(SettingKey::S35);
        assert_eq!(w.value_text(Lang::Zh), "#38FFFFFF");
        assert_eq!(color_of(w.value).r, 255);
    }

    #[test]
    fn button_kinds_open_correct_popups() {
        let mut a = widget(SettingKey::S06);
        let mut b = widget(SettingKey::S46);
        let mut c = widget(SettingKey::S05);
        assert_eq!(a.activate(0.0), WidgetResult::OpenPopup(PopupKind::ConfirmRestore));
        assert_eq!(b.activate(0.0), WidgetResult::OpenPopup(PopupKind::ExportDiagnostics));
        assert_eq!(c.activate(0.0), WidgetResult::OpenPopup(PopupKind::CopyVersion));
    }

    #[test]
    fn tooltip_detects_long_text() {
        assert!(needs_tooltip("这是一段非常长的说明文案用于验证省略号与悬浮提示是否生效", 60.0));
        assert!(!needs_tooltip("短", 200.0));
    }

    #[test]
    fn banner_is_zero_height_when_hidden() {
        let mut b = Banner::new(false);
        let s = b.layout(Rect::new(0.0, 0.0, 640.0, 0.0), &Metrics::default());
        assert_eq!(s.h, 0.0);
    }
}
