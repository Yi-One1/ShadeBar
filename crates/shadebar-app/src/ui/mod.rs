//! UI 模块根（§4 #28 / §6.4 `View`）。
//!
//! [`View`] 持有控件树、分组卡片、滚动视口、焦点/悬停、语言版本与诊断数据。
//! `window.rs` 只负责把 Win32 消息转发进来并调用 `paint`；所有布局、命中、
//! 弹层、语言热切换都在这里收敛，六个页面不各自实现一遍。

pub mod i18n;
pub mod layout;
pub mod pages;
pub mod theme;
pub mod widgets;

use std::time::Instant;

use shadebar_core::PRODUCT_NAME;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

use crate::config::{Config, Group, Lang, SettingKey};
use crate::render::{RectDip, RenderCtx, TextRole};
use crate::ui::layout::{Metrics, Rect, Scroll};
use crate::ui::pages::diagnostics::{DiagView, paint as paint_diagnostics};
use crate::ui::pages::{SETTING_GROUPS, keys_of};
use crate::ui::theme::{Palette, ThemeMode, CARD_RADIUS_DIP};
use crate::ui::widgets::{
    Banner, GroupHeader, PopupKind, SettingWidget, UiEvent, VK_ESCAPE, VK_NEXT, VK_PRIOR,
    VK_RETURN, VK_TAB, Widget, WidgetResult,
};

// Win32 消息号（避免为几个常量再开 windows feature）。
const MSG_MOUSEMOVE: u32 = 0x0200;
const MSG_LBUTTONDOWN: u32 = 0x0201;
const MSG_LBUTTONUP: u32 = 0x0202;
const MSG_MOUSEWHEEL: u32 = 0x020A;
const MSG_KEYDOWN: u32 = 0x0100;
const MSG_CHAR: u32 = 0x0102;

/// 诊断卡片展开后的正文高度（dip）。
const DIAG_CARD_H: f32 = 320.0;
/// 诊断卡片标题行高度（dip）。收起态只占这一行，点它展开。
const DIAG_HEAD_H: f32 = 44.0;
/// 下拉单项高度（dip）。
const POPUP_ITEM_H: f32 = 28.0;
/// 弹层宽度（dip）。
const POPUP_W: f32 = 220.0;
/// 提示条显示时长（毫秒）。
const TOAST_MS: u64 = 2200;

/// `View` 交给 `App` 执行的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAction {
    /// 写入一个设置值（含 S-04 语言热切换）。
    Set(SettingKey, i64),
    /// 全量恢复默认（A-12，已经过二次确认）。
    RestoreDefaults,
    /// 导出诊断包（S-46）。
    ExportDiagnostics,
    /// 复制版本号（S-05）。
    CopyVersion,
}

/// 弹层状态。
#[derive(Debug, Clone)]
enum PopupState {
    /// 无弹层。
    None,
    /// 下拉选项。
    Dropdown { key: SettingKey, rect: Rect },
    /// ARGB 颜色选择（Hex 输入 + 预设色板 + 内置 alpha）。
    ColorPicker { key: SettingKey, rect: Rect, hex: String },
    /// 恢复默认二次确认。
    Confirm { rect: Rect },
}

/// 设置窗口的控件树与状态。
pub struct View {
    /// 7 个分组标题（6 个设置分组 + 诊断）。
    headers: Vec<GroupHeader>,
    /// 设置行（数量等于当前可见项数）。
    rows: Vec<SettingWidget>,
    /// 每个分组对应的行区间 `[start, end)`。
    row_ranges: Vec<(usize, usize)>,
    /// 每个分组的卡片矩形（内容坐标）。
    cards: Vec<Rect>,
    /// 降级横幅。
    banner: Banner,
    /// 诊断卡片矩形（含标题行）。
    diag_rect: Rect,
    /// 诊断卡片是否展开（默认收起：诊断是排查工具，不该占主页面）。
    diag_expanded: bool,
    /// 滚动视口。
    scroll: Scroll,
    /// 布局度量。
    metrics: Metrics,
    /// 当前语言。
    lang: Lang,
    /// 当前深浅色。
    mode: ThemeMode,
    /// 是否降级（L1 only）。
    degraded: bool,
    /// 焦点行下标。
    focus_index: Option<usize>,
    /// 弹层。
    popup: PopupState,
    /// 关于页浮层。
    about_visible: bool,
    /// 待交给 App 的动作。
    actions: Vec<ViewAction>,
    /// 瞬时提示（key, 到期毫秒）。
    toast: Option<(&'static str, u64)>,
    /// 悬浮提示（全文 + 内容坐标）。
    tooltip: Option<(String, f32, f32)>,
    /// 诊断数据。
    diag: DiagView,
    /// 客户区尺寸（dip）。
    client: (f32, f32),
    /// 当前 DPI（px → dip 换算用）。
    dpi: u32,
    /// 单调时钟（toast 计时）。
    clock: Instant,
}

impl View {
    /// 以配置、语言、降级状态建立控件树。
    #[must_use]
    pub fn new(cfg: &Config, lang: Lang, degraded: bool) -> Self {
        let mut headers = Vec::new();
        let mut rows = Vec::new();
        let mut row_ranges = Vec::new();
        for g in SETTING_GROUPS {
            headers.push(GroupHeader::new(g));
            let start = rows.len();
            for &k in keys_of(g) {
                rows.push(SettingWidget::new(k, cfg, degraded));
            }
            row_ranges.push((start, rows.len()));
        }
        // 诊断分组无设置项，但有自己的标题与卡片。
        headers.push(GroupHeader::new(Group::Diagnostics));
        row_ranges.push((rows.len(), rows.len()));

        // T03.4 绑定自检：每个分组的 `keys_of` 静态表必须与 `Group::keys`
        // 的元数据过滤结果一致，防止新增项漏绑到页面。
        for g in SETTING_GROUPS {
            if g.keys().len() != keys_of(g).len() {
                crate::config::note("page binding mismatch detected");
                break;
            }
        }
        // A-25 的双语覆盖由 `i18n::tests::both_tables_cover_every_required_key`
        // 在构建期逐项核对（含 35 项标签/说明、分组标题、托盘菜单、诊断、
        // 确认框、关于页、下拉选项），比运行期再扫一遍更严格且零体积开销。

        Self {
            headers,
            rows,
            row_ranges,
            cards: Vec::new(),
            banner: Banner::new(degraded),
            diag_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            // 默认收起：诊断只服务排查，常驻展开会吃掉主页面一半高度。
            diag_expanded: false,
            scroll: Scroll::default(),
            metrics: Metrics::default(),
            lang,
            mode: ThemeMode::system(),
            degraded,
            focus_index: None,
            popup: PopupState::None,
            about_visible: false,
            actions: Vec::new(),
            toast: None,
            tooltip: None,
            diag: DiagView::default(),
            client: (720.0, 560.0),
            dpi: 96,
            clock: Instant::now(),
        }
    }

    /// 取走待执行动作。
    pub fn take_actions(&mut self) -> Vec<ViewAction> {
        std::mem::take(&mut self.actions)
    }

    /// 当前语言。
    #[must_use]
    pub const fn language(&self) -> Lang {
        self.lang
    }

    /// 当前客户区 DPI。
    #[must_use]
    pub const fn dpi(&self) -> u32 {
        self.dpi
    }

    /// DPI 变化（PerMonitorV2）。
    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
    }

    /// 客户区尺寸变化（`WM_SIZE`）。
    pub fn set_client(&mut self, w_dip: f32, h_dip: f32) {
        self.client = (w_dip.max(1.0), h_dip.max(1.0));
        self.rebuild_layout();
    }

    /// 系统深浅色变化（FR-40）。
    pub fn set_theme(&mut self, mode: ThemeMode) {
        self.mode = mode;
    }

    /// 当前深浅色。
    #[must_use]
    pub const fn theme(&self) -> ThemeMode {
        self.mode
    }

    /// 语言热切换（A-24：≤1 s、PID 不变、采样不中断）。
    pub fn rebuild_texts(&mut self, lang: Lang) {
        self.lang = lang;
        for h in &mut self.headers {
            h.set_texts(lang);
        }
        for r in &mut self.rows {
            r.set_texts(lang);
        }
        self.banner.set_texts(lang);
    }

    /// 用最新配置/诊断/降级状态刷新控件（A-10：设置改动同拍生效）。
    pub fn rebuild(&mut self, cfg: &Config, diag: &DiagView, degraded: bool) {
        self.degraded = degraded;
        self.banner.visible = degraded;
        for r in &mut self.rows {
            r.sync_value(cfg.get(r.key));
            // 降级时只置灰 L2/L3 元素控件，值保留（NFR-05 / §13-6）。
            r.set_enabled(
                !(degraded && crate::ui::pages::elements::GREYED_IN_DEGRADED.contains(&r.key)),
            );
        }
        self.diag = diag.clone();
        self.rebuild_layout();
    }

    /// 打开关于页（托盘菜单「关于」）。
    pub fn show_about(&mut self) {
        self.about_visible = true;
        self.popup = PopupState::None;
    }

    /// 由托盘菜单请求一个弹层/动作（供 `window.rs` 转发 `PopupKind::About` 等）。
    pub fn request_popup(&mut self, kind: PopupKind) {
        self.open_popup(kind, self.client.0 / 2.0, self.metrics.heading_h);
    }

    /// 显示一条瞬时提示。
    pub fn show_toast(&mut self, key: &'static str) {
        self.toast = Some((key, self.now_ms().saturating_add(TOAST_MS)));
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// 重新计算所有卡片/行/诊断区的内容坐标与内容总高。
    fn rebuild_layout(&mut self) {
        let m = self.metrics;
        let outer_x = m.margin;
        let outer_w = (self.client.0 - m.margin * 2.0).max(0.0);
        let inner_x = outer_x + m.card_pad;
        let inner_w = (outer_w - m.card_pad * 2.0).max(0.0);
        let mut y = 0.0_f32;

        let banner_size = self.banner.layout(Rect::new(outer_x, y, outer_w, 0.0), &m);
        if banner_size.h > 0.0 {
            y += banner_size.h + m.group_gap;
        }

        self.cards.clear();
        let groups_len = SETTING_GROUPS.len();
        for (gi, g) in SETTING_GROUPS
            .iter()
            .copied()
            .chain([Group::Diagnostics])
            .enumerate()
        {
            let _ = g; // 分组名只用于标识，标题文字由 GroupHeader 自己取。
            let card_y = y;
            let header_size = self.headers[gi].layout(Rect::new(inner_x, y, inner_w, 0.0), &m);
            y += header_size.h;
            let (start, end) = self.row_ranges[gi];
            for idx in start..end {
                let size = self.rows[idx].layout(Rect::new(inner_x, y, inner_w, 0.0), &m);
                y += size.h;
            }
            if gi == groups_len {
                let dh = if self.diag_expanded {
                    DIAG_HEAD_H + DIAG_CARD_H
                } else {
                    DIAG_HEAD_H
                };
                self.diag_rect = Rect::new(inner_x, y, inner_w, dh);
                y += dh;
            }
            self.cards.push(Rect::new(outer_x, card_y, outer_w, y - card_y + m.card_pad));
            y += m.group_gap;
        }

        self.scroll.content_h = y + m.card_pad;
        self.scroll.view_h = (self.client.1 - m.heading_h).max(0.0);
        self.scroll.clamp();
    }

    /// 画一帧（由 `window.rs::WM_PAINT` 在 `BeginPaint/EndPaint` 之间调用）。
    pub fn paint(&mut self, ctx: &RenderCtx, palette: &Palette, mode: ThemeMode) {
        self.mode = mode;
        self.rebuild_layout();
        let (cw, ch) = self.client;
        let dy = self.metrics.heading_h - self.scroll.offset;

        // 1) 整幅不透明背景（B3：每帧从干净表面重建，绝不叠加）。
        ctx.fill_rect(RectDip::new(0.0, 0.0, cw, ch), palette.window_bg);
        ctx.fill_rect(RectDip::new(0.0, 0.0, cw, self.metrics.heading_h), palette.heading_bg);
        ctx.draw_text(
            PRODUCT_NAME,
            RectDip::new(self.metrics.margin, 10.0, cw - self.metrics.margin * 2.0, 26.0),
            TextRole::Title,
            palette.text_primary,
        );

        // 2) 分组卡片（超出视口的由系统裁剪，不影响布局正确性）。
        for card in &self.cards {
            let r = RectDip::new(card.x, card.y + dy, card.w, card.h);
            if r.y > ch || r.y + r.h < self.metrics.heading_h {
                continue;
            }
            ctx.fill_rounded_rect(r, CARD_RADIUS_DIP, palette.card_bg);
            ctx.stroke_rounded_rect(r, CARD_RADIUS_DIP, palette.card_border, 1.0);
        }

        self.banner.paint(ctx, palette, self.lang, dy);
        for h in &self.headers {
            h.paint(ctx, palette, self.lang, dy);
        }
        for r in &self.rows {
            let visible_top = r.rect.y + dy;
            if visible_top > ch || visible_top + r.rect.h < self.metrics.heading_h {
                continue;
            }
            r.paint(ctx, palette, self.lang, dy);
        }
        let dr = self.diag_rect;
        let dtop = dr.y + dy;
        if dtop < ch && dtop + dr.h > self.metrics.heading_h {
            self.paint_diag_head(ctx, palette, RectDip::new(dr.x, dtop, dr.w, DIAG_HEAD_H));
            if self.diag_expanded {
                paint_diagnostics(
                    ctx,
                    palette,
                    self.lang,
                    RectDip::new(dr.x, dtop + DIAG_HEAD_H, dr.w, dr.h - DIAG_HEAD_H),
                    &self.diag,
                );
                // 采样区过小预警叠加在诊断卡片底部（T03.4 判据⑥，红色）。
                if self.diag.region_small() {
                    ctx.draw_text(
                        i18n::text("banner.region_small", self.lang),
                        RectDip::new(dr.x + 16.0, dtop + dr.h - 24.0, dr.w - 32.0, 20.0),
                        TextRole::Body,
                        palette.danger,
                    );
                }
            }
        }

        self.paint_popup(ctx, palette, dy);
        if self.about_visible {
            self.paint_about(ctx, palette);
        }
        self.paint_tooltip_and_toast(ctx, palette);
    }

    /// 画诊断卡片的标题行（收起态的全部内容，展开态的可点击抬头）。
    fn paint_diag_head(&self, ctx: &RenderCtx, palette: &Palette, r: RectDip) {
        // 左侧：一行实时摘要，收起时也能确认采样在跑。
        let summary = if self.diag.has_data {
            format!(
                "Y {:.3}   {:.1}:1",
                self.diag.y_milli as f32 / 1000.0,
                self.diag.contrast_milli as f32 / 1000.0
            )
        } else {
            i18n::text("diag.no_data", self.lang).to_string()
        };
        ctx.draw_text(
            &summary,
            RectDip::new(r.x + 16.0, r.y + 12.0, (r.w - 160.0).max(60.0), 20.0),
            TextRole::Body,
            palette.text_secondary,
        );
        // 右侧：展开 / 收起。
        let hint = if self.diag_expanded {
            i18n::text("diag.collapse", self.lang)
        } else {
            i18n::text("diag.expand", self.lang)
        };
        let arrow = if self.diag_expanded { "  ▾" } else { "  ▸" };
        ctx.draw_text(
            &format!("{hint}{arrow}"),
            RectDip::new(r.x + r.w - 136.0, r.y + 12.0, 120.0, 20.0),
            TextRole::Body,
            palette.accent,
        );
    }

    fn paint_popup(&self, ctx: &RenderCtx, palette: &Palette, dy: f32) {
        match &self.popup {
            PopupState::None => {}
            PopupState::Dropdown { key, rect } => {
                let meta = key.meta();
                let h = meta.options.len() as f32 * POPUP_ITEM_H + 8.0;
                let r = RectDip::new(rect.x, rect.y + dy, POPUP_W.min(rect.w.max(120.0)), h);
                ctx.fill_rounded_rect(r, 6.0, palette.card_bg);
                ctx.stroke_rounded_rect(r, 6.0, palette.accent, 1.0);
                for (i, k) in meta.options.iter().enumerate() {
                    let label = if k.starts_with('#') { *k } else { i18n::text(k, self.lang) };
                    ctx.draw_text(
                        label,
                        RectDip::new(r.x + 8.0, r.y + 4.0 + i as f32 * POPUP_ITEM_H, r.w - 16.0, POPUP_ITEM_H),
                        TextRole::Body,
                        palette.text_primary,
                    );
                }
            }
            PopupState::ColorPicker { rect, hex, .. } => {
                let r = RectDip::new(rect.x, rect.y + dy, POPUP_W + 40.0, 150.0);
                ctx.fill_rounded_rect(r, 6.0, palette.card_bg);
                ctx.stroke_rounded_rect(r, 6.0, palette.accent, 1.0);
                ctx.draw_text(
                    hex,
                    RectDip::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, 24.0),
                    TextRole::Body,
                    palette.text_primary,
                );
                for (i, p) in i18n::OPT_COLOR_PRESETS.iter().enumerate() {
                    let col = i % 3;
                    let row = i / 3;
                    let sw = RectDip::new(
                        r.x + 8.0 + col as f32 * 78.0,
                        r.y + 40.0 + row as f32 * 40.0,
                        70.0,
                        32.0,
                    );
                    let c = parse_hex_color(p).unwrap_or(palette.accent);
                    ctx.fill_rounded_rect(sw, 3.0, c);
                    ctx.draw_text(p, RectDip::new(sw.x + 2.0, sw.y + 8.0, sw.w, 18.0), TextRole::Body, palette.text_primary);
                }
            }
            PopupState::Confirm { rect } => {
                let r = RectDip::new(rect.x, rect.y + dy, rect.w, 120.0);
                ctx.fill_rounded_rect(r, 8.0, palette.card_bg);
                ctx.stroke_rounded_rect(r, 8.0, palette.warning, 1.5);
                ctx.draw_text(
                    i18n::text("dlg.restore.title", self.lang),
                    RectDip::new(r.x + 12.0, r.y + 10.0, r.w - 24.0, 24.0),
                    TextRole::Title,
                    palette.text_primary,
                );
                ctx.draw_text(
                    i18n::text("dlg.restore.body", self.lang),
                    RectDip::new(r.x + 12.0, r.y + 36.0, r.w - 24.0, 24.0),
                    TextRole::Body,
                    palette.text_secondary,
                );
                let bw = (r.w - 36.0) / 2.0;
                ctx.fill_rounded_rect(RectDip::new(r.x + 12.0, r.y + 76.0, bw, 32.0), 4.0, palette.accent);
                ctx.draw_text(
                    i18n::text("dlg.confirm", self.lang),
                    RectDip::new(r.x + 12.0, r.y + 82.0, bw, 22.0),
                    TextRole::Body,
                    palette.accent_text,
                );
                ctx.fill_rounded_rect(RectDip::new(r.x + 24.0 + bw, r.y + 76.0, bw, 32.0), 4.0, palette.disabled_fill);
                ctx.draw_text(
                    i18n::text("dlg.cancel", self.lang),
                    RectDip::new(r.x + 24.0 + bw, r.y + 82.0, bw, 22.0),
                    TextRole::Body,
                    palette.text_primary,
                );
            }
        }
    }

    fn paint_about(&self, ctx: &RenderCtx, palette: &Palette) {
        let (cw, ch) = self.client;
        let w = 420.0_f32.min(cw - 40.0);
        let h = 220.0_f32;
        let r = RectDip::new((cw - w) / 2.0, (ch - h) / 2.0, w, h);
        ctx.fill_rounded_rect(r, 10.0, palette.card_bg);
        ctx.stroke_rounded_rect(r, 10.0, palette.accent, 1.5);
        let version_line = format!("{}: v{}", i18n::text("about.version", self.lang), shadebar_core::VERSION);
        let lines = [
            i18n::text("about.title", self.lang),
            version_line.as_str(),
            i18n::text("about.license", self.lang),
            i18n::text("about.known_limits", self.lang),
        ];
        for (i, line) in lines.iter().enumerate() {
            ctx.draw_text(
                line,
                RectDip::new(r.x + 20.0, r.y + 20.0 + i as f32 * 34.0, r.w - 40.0, 28.0),
                if i == 0 { TextRole::Title } else { TextRole::Body },
                palette.text_primary,
            );
        }
        ctx.draw_text(
            i18n::text("about.copy_version", self.lang),
            RectDip::new(r.x + 20.0, r.y + h - 44.0, r.w - 40.0, 24.0),
            TextRole::Body,
            palette.accent,
        );
    }

    fn paint_tooltip_and_toast(&mut self, ctx: &RenderCtx, palette: &Palette) {
        if let Some((text, x, y)) = self.tooltip.clone() {
            let w = (text.chars().count() as f32 * 8.0).min(self.client.0 - 20.0);
            let r = RectDip::new(x + 10.0, y + 14.0, w + 16.0, 26.0);
            ctx.fill_rounded_rect(r, 4.0, palette.card_bg);
            ctx.stroke_rounded_rect(r, 4.0, palette.card_border, 1.0);
            ctx.draw_text(&text, RectDip::new(r.x + 8.0, r.y + 4.0, r.w - 16.0, 20.0), TextRole::Body, palette.text_primary);
        }
        let now = self.now_ms();
        if let Some((key, until)) = self.toast {
            if now < until {
                let text = i18n::text(key, self.lang);
                let r = RectDip::new(self.metrics.margin, self.client.1 - 40.0, self.client.0 - self.metrics.margin * 2.0, 28.0);
                ctx.fill_rounded_rect(r, 4.0, palette.card_bg);
                ctx.draw_text(text, RectDip::new(r.x + 10.0, r.y + 4.0, r.w - 20.0, 22.0), TextRole::Body, palette.text_primary);
            } else {
                self.toast = None;
            }
        }
    }

    fn px_to_content(&self, px: i32, py: i32) -> (f32, f32) {
        let scale = 96.0 / self.dpi.max(96) as f32;
        (
            px as f32 * scale,
            py as f32 * scale - self.metrics.heading_h + self.scroll.offset,
        )
    }

    fn low_word(v: isize) -> i32 {
        (v & 0xFFFF) as i16 as i32
    }

    fn high_word(v: isize) -> i32 {
        ((v >> 16) & 0xFFFF) as i16 as i32
    }

    /// 鼠标悬停 / 光标判定（§6.4 签名；坐标为客户区物理像素）。
    pub fn hit_test_and_update_cursor(&mut self, px: i32, py: i32) -> bool {
        let (x, y) = self.px_to_content(px, py);
        let mut interactive = false;
        self.tooltip = None;
        for r in &mut self.rows {
            let hit = r.control_hit(x, y);
            r.hover = hit;
            if hit {
                interactive = true;
            }
            if r.row_hit(x, y) {
                let label = i18n::text(r.meta.label, self.lang);
                let help = i18n::text(r.meta.help, self.lang);
                let label_w = (r.control.x - r.rect.x - 8.0).max(40.0);
                if crate::ui::widgets::needs_tooltip(label, label_w) {
                    self.tooltip = Some((label.to_string(), x, y));
                } else if crate::ui::widgets::needs_tooltip(help, r.rect.w) {
                    self.tooltip = Some((help.to_string(), x, y));
                }
            }
        }
        interactive || !matches!(self.popup, PopupState::None) || self.about_visible
            // 诊断抬头可点：给了手型光标用户才知道能展开。
            || Rect::new(self.diag_rect.x, self.diag_rect.y, self.diag_rect.w, DIAG_HEAD_H)
                .contains(x, y)
    }

    fn set_focus(&mut self, idx: Option<usize>) {
        if let Some(old) = self.focus_index {
            if let Some(r) = self.rows.get_mut(old) {
                r.focus = false;
            }
        }
        self.focus_index = idx;
        if let Some(i) = idx {
            if let Some(r) = self.rows.get_mut(i) {
                r.focus = true;
            }
        }
    }

    fn open_popup(&mut self, kind: PopupKind, x: f32, y: f32) {
        match kind {
            PopupKind::Dropdown(key) => {
                self.popup = PopupState::Dropdown { key, rect: Rect::new(x, y, POPUP_W, 0.0) };
            }
            PopupKind::ColorPicker(key) => {
                let current = self.rows.iter().find(|r| r.key == key).map_or(0xFFFF_FFFF, |r| r.value);
                self.popup = PopupState::ColorPicker {
                    key,
                    rect: Rect::new(x, y, POPUP_W + 40.0, 0.0),
                    hex: format!("#{:08X}", current as u32),
                };
            }
            PopupKind::ConfirmRestore => {
                let w = 360.0_f32.min(self.client.0 - 40.0);
                self.popup = PopupState::Confirm {
                    rect: Rect::new((self.client.0 - w) / 2.0, self.client.1 / 2.0 - 60.0, w, 0.0),
                };
            }
            PopupKind::About => self.show_about(),
            PopupKind::ExportDiagnostics => {
                self.actions.push(ViewAction::ExportDiagnostics);
                self.show_toast("common.export_ok");
            }
            PopupKind::CopyVersion => {
                self.actions.push(ViewAction::CopyVersion);
                self.show_toast("common.copy_ok");
            }
        }
    }

    fn handle_popup_click(&mut self, x: f32, y: f32) -> bool {
        let state = self.popup.clone();
        match state {
            PopupState::None => false,
            PopupState::Dropdown { key, rect } => {
                let meta = key.meta();
                let n = meta.options.len();
                let top = rect.y;
                let right = rect.x + POPUP_W.min(rect.w.max(120.0));
                let bottom = top + n as f32 * POPUP_ITEM_H + 8.0;
                if !(rect.x..=right).contains(&x) || !(top..=bottom).contains(&y) {
                    self.popup = PopupState::None;
                    return true;
                }
                let idx = ((y - top - 4.0) / POPUP_ITEM_H).floor().max(0.0) as usize;
                if idx < n {
                    self.actions.push(ViewAction::Set(key, idx as i64));
                }
                self.popup = PopupState::None;
                true
            }
            PopupState::ColorPicker { key, rect, hex } => {
                let rx = x - rect.x;
                let ry = y - rect.y;
                if (8.0..=8.0 + 3.0 * 78.0).contains(&rx)
                    && (40.0..=40.0 + 2.0 * 40.0).contains(&ry)
                {
                    let col = ((rx - 8.0) / 78.0).floor() as usize;
                    let row = ((ry - 40.0) / 40.0).floor() as usize;
                    let idx = row * 3 + col;
                    if let Some(p) = i18n::OPT_COLOR_PRESETS.get(idx) {
                        if let Some(v) = parse_hex_color(p) {
                            self.actions.push(ViewAction::Set(key, rgba_to_argb(v)));
                        }
                        self.popup = PopupState::None;
                        return true;
                    }
                }
                if !(0.0..=POPUP_W + 40.0).contains(&rx) || !(0.0..=150.0).contains(&ry) {
                    self.popup = PopupState::None;
                } else {
                    // Hex 输入区：保留弹层，等 WM_CHAR / Enter。
                    self.popup = PopupState::ColorPicker { key, rect, hex };
                }
                true
            }
            PopupState::Confirm { rect } => {
                let ry = y - rect.y;
                let bw = (rect.w - 36.0) / 2.0;
                if (76.0..=108.0).contains(&ry) {
                    if (rect.x + 12.0..=rect.x + 12.0 + bw).contains(&x) {
                        self.actions.push(ViewAction::RestoreDefaults);
                    }
                    self.popup = PopupState::None;
                } else if !(rect.x..=rect.x + rect.w).contains(&x) || !(0.0..=120.0).contains(&ry) {
                    self.popup = PopupState::None;
                }
                true
            }
        }
    }

    fn handle_row_event(&mut self, ev: &UiEvent, x: f32, y: f32) -> bool {
        // 诊断卡片抬头：只响应标题行的按下，展开/收起切换后立即重排。
        if let UiEvent::LButtonDown { .. } = *ev {
            let head = Rect::new(self.diag_rect.x, self.diag_rect.y, self.diag_rect.w, DIAG_HEAD_H);
            if head.contains(x, y) {
                self.diag_expanded = !self.diag_expanded;
                self.rebuild_layout();
                // 展开后正文会伸到视口下方：直接滚到底，让用户立刻看到面板，
                // 而不是点开一个"看起来没反应"的卡片。
                if self.diag_expanded {
                    self.scroll.offset = self.scroll.max_offset();
                } else {
                    self.scroll.clamp();
                }
                return true;
            }
        }
        for idx in 0..self.rows.len() {
            let result = self.rows[idx].event(ev);
            match result {
                WidgetResult::Ignored => {}
                WidgetResult::Handled => return true,
                WidgetResult::Changed(key, v) => {
                    self.actions.push(ViewAction::Set(key, v));
                    self.rows[idx].sync_value(v);
                    return true;
                }
                WidgetResult::OpenPopup(kind) => {
                    self.open_popup(kind, x, y);
                    return true;
                }
            }
        }
        false
    }

    /// 处理一条 Win32 消息（§6.4 签名）。返回 `Some(LRESULT)` 表示已消费。
    pub fn on_message(
        &mut self,
        _hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            MSG_MOUSEMOVE | MSG_LBUTTONDOWN | MSG_LBUTTONUP => {
                let px = Self::low_word(lp.0);
                let py = Self::high_word(lp.0);
                let (x, y) = self.px_to_content(px, py);
                if !matches!(self.popup, PopupState::None)
                    && msg == MSG_LBUTTONDOWN
                    && self.handle_popup_click(x, y)
                {
                    return Some(LRESULT(0));
                }
                if self.about_visible && msg == MSG_LBUTTONDOWN {
                    self.about_visible = false;
                    return Some(LRESULT(0));
                }
                let ev = match msg {
                    MSG_MOUSEMOVE => UiEvent::MouseMove { x, y },
                    MSG_LBUTTONDOWN => UiEvent::LButtonDown { x, y },
                    _ => UiEvent::LButtonUp { x, y },
                };
                let consumed = self.handle_row_event(&ev, x, y);
                if consumed {
                    Some(LRESULT(0))
                } else {
                    None
                }
            }
            MSG_MOUSEWHEEL => {
                let delta = Self::high_word(wp.0 as isize) as f32;
                let _ = self.scroll.scroll_by(-delta / 120.0 * 60.0);
                Some(LRESULT(0))
            }
            MSG_KEYDOWN => {
                let vk = (wp.0 & 0xFFFF) as u16;
                if vk == VK_ESCAPE {
                    self.popup = PopupState::None;
                    self.about_visible = false;
                    return Some(LRESULT(0));
                }
                if vk == VK_TAB {
                    let next = match self.focus_index {
                        None => Some(0usize),
                        Some(i) => Some((i + 1) % self.rows.len().max(1)),
                    };
                    self.set_focus(next);
                    return Some(LRESULT(0));
                }
                if vk == VK_PRIOR || vk == VK_NEXT {
                    let _ = self.scroll.page(if vk == VK_NEXT { 1.0 } else { -1.0 });
                    return Some(LRESULT(0));
                }
                // 颜色弹层的 Hex 输入回退（Enter 提交）。
                if vk == VK_RETURN {
                    let state = self.popup.clone();
                    if let PopupState::ColorPicker { key, hex, .. } = state {
                        if let Some(v) = parse_hex_color(&hex) {
                            self.actions.push(ViewAction::Set(key, rgba_to_argb(v)));
                        }
                        self.popup = PopupState::None;
                        return Some(LRESULT(0));
                    }
                }
                let consumed = self.handle_row_event(&UiEvent::Key { vk }, 0.0, 0.0);
                if consumed { Some(LRESULT(0)) } else { None }
            }
            MSG_CHAR => {
                let ch = char::from_u32((wp.0 & 0xFFFF) as u32).unwrap_or('\0');
                if ch == '\0' {
                    return None;
                }
                let state = self.popup.clone();
                if let PopupState::ColorPicker { key, rect, mut hex } = state {
                    if ch == '\u{0008}' {
                        hex.pop();
                    } else if (ch == '#' || ch.is_ascii_hexdigit()) && hex.chars().count() < 10 {
                        hex.push(ch.to_ascii_uppercase());
                    }
                    self.popup = PopupState::ColorPicker { key, rect, hex };
                    return Some(LRESULT(0));
                }
                let consumed = self.handle_row_event(&UiEvent::Char { ch }, 0.0, 0.0);
                if consumed { Some(LRESULT(0)) } else { None }
            }
            _ => None,
        }
    }
}

/// `#RRGGBB` / `#AARRGGBB` → Rgba；非法输入返回 `None`（T03.3 判据④）。
#[must_use]
pub fn parse_hex_color(text: &str) -> Option<shadebar_core::color::Rgba> {
    let t = text.trim().trim_start_matches('#');
    let (a, rgb) = match t.len() {
        6 => (255u8, t),
        8 => (u8::from_str_radix(&t[..2], 16).ok()?, &t[2..]),
        _ => return None,
    };
    if rgb.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&rgb[..2], 16).ok()?;
    let g = u8::from_str_radix(&rgb[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rgb[4..6], 16).ok()?;
    // alpha=0 视为非法（§6 禁用全透明）；a 存进值表但不用于判定色。
    let a = a.max(1);
    Some(shadebar_core::color::Rgba { r, g, b, a })
}

fn rgba_to_argb(c: shadebar_core::color::Rgba) -> i64 {
    i64::from(
        (u32::from(c.a) << 24)
            | (u32::from(c.r) << 16)
            | (u32::from(c.g) << 8)
            | u32::from(c.b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View::new(&Config::default(), Lang::Zh, false)
    }

    /// 当前设置页可见项数（配置项总数 - 尚未落地的隐藏项）。
    fn visible_rows() -> usize {
        crate::config::SettingKey::ALL.iter().filter(|k| k.ui_visible()).count()
    }

    #[test]
    fn view_builds_one_row_per_visible_setting_and_7_group_headers() {
        let v = view();
        assert_eq!(v.rows.len(), visible_rows());
        assert_eq!(v.headers.len(), 7);
        assert_eq!(v.row_ranges.len(), 7);
    }

    #[test]
    fn layout_produces_7_cards_and_positive_content_height() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        assert_eq!(v.cards.len(), 7);
        assert!(v.scroll.content_h > 560.0, "全部可见项必然需要滚动");
        assert!(v.scroll.max_offset() > 0.0);
    }

    #[test]
    fn diag_card_is_collapsed_by_default_and_toggles_on_head_click() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        assert!(!v.diag_expanded, "诊断默认必须收起");
        assert_eq!(v.diag_rect.h, DIAG_HEAD_H, "收起态只占标题行");
        let collapsed_h = v.scroll.content_h;

        let (x, y) = (v.diag_rect.x + 12.0, v.diag_rect.y + 8.0);
        assert!(v.handle_row_event(&UiEvent::LButtonDown { x, y }, x, y), "抬头应消费点击");
        assert!(v.diag_expanded);
        assert_eq!(v.diag_rect.h, DIAG_HEAD_H + DIAG_CARD_H);
        assert!(
            v.scroll.content_h > collapsed_h + DIAG_CARD_H - 1.0,
            "展开后内容高度必须增加一整块正文"
        );
        assert_eq!(v.scroll.offset, v.scroll.max_offset(), "展开后应自动滚到底，露出正文");

        let (x2, y2) = (v.diag_rect.x + 12.0, v.diag_rect.y + 8.0);
        assert!(v.handle_row_event(&UiEvent::LButtonDown { x: x2, y: y2 }, x2, y2));
        assert!(!v.diag_expanded, "再点一次应收回");
        assert_eq!(v.scroll.content_h, collapsed_h);
    }

    #[test]
    fn rebuild_syncs_values_and_greys_only_degraded_elements() {
        let mut cfg = Config::default();
        assert!(cfg.set(SettingKey::S16, 3000));
        let mut v = View::new(&cfg, Lang::Zh, true);
        v.rebuild(&cfg, &DiagView::default(), true);
        assert_eq!(
            v.rows.iter().find(|r| r.key == SettingKey::S16).map_or(0, |r| r.value),
            3000
        );
        assert!(!v.rows.iter().find(|r| r.key == SettingKey::S30).is_none_or(|r| r.enabled));
        assert!(v.rows.iter().find(|r| r.key == SettingKey::S16).is_some_and(|r| r.enabled));
    }

    #[test]
    fn language_switch_is_instant_and_keeps_pid_state() {
        let mut v = view();
        v.rebuild_texts(Lang::En);
        assert_eq!(v.language(), Lang::En);
        assert_eq!(v.rows.len(), visible_rows(), "语言切换不重建进程/不清空控件树");
    }

    #[test]
    fn tab_moves_focus_through_all_rows() {
        let mut v = view();
        let wp = WPARAM(VK_TAB as usize);
        for _ in 0..visible_rows() {
            assert!(v.on_message(HWND::default(), MSG_KEYDOWN, wp, LPARAM(0)).is_some());
        }
        assert!(v.focus_index.is_some());
    }

    #[test]
    fn wheel_scrolls_and_clamps() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        let wp = WPARAM(((-120i32) as u16 as usize) << 16);
        assert!(v.on_message(HWND::default(), MSG_MOUSEWHEEL, wp, LPARAM(0)).is_some());
        assert!(v.scroll.offset > 0.0);
    }

    #[test]
    fn hex_parser_accepts_rgb_and_argb_and_rejects_garbage() {
        assert_eq!(
            parse_hex_color("#FFFFFF"),
            Some(shadebar_core::color::Rgba::opaque(255, 255, 255))
        );
        assert_eq!(parse_hex_color("38FFFFFF").map(|c| c.a), Some(0x38));
        assert!(parse_hex_color("#GGGGGG").is_none());
        assert!(parse_hex_color("#FFF").is_none());
        assert_eq!(
            rgba_to_argb(parse_hex_color("#38FFFFFF").unwrap_or(shadebar_core::color::Rgba::opaque(0, 0, 0))),
            0x38FF_FFFF
        );
    }

    #[test]
    fn clicking_restore_defaults_requires_confirmation_popup() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        let (cx, cy) = v
            .rows
            .iter()
            .find(|r| r.key == SettingKey::S06)
            .map_or((0.0, 0.0), |r| (r.control.x + 5.0, r.control.y + 5.0));
        let handled = v.handle_row_event(&UiEvent::LButtonDown { x: cx, y: cy }, cx, cy);
        assert!(handled);
        assert!(matches!(v.popup, PopupState::Confirm { .. }), "必须有二次确认");
        assert!(v.take_actions().is_empty(), "确认前不得写回默认值");
    }

    #[test]
    fn overlay_escape_closes_popups() {
        let mut v = view();
        v.open_popup(PopupKind::About, 0.0, 0.0);
        assert!(v.about_visible);
        let wp = WPARAM(VK_ESCAPE as usize);
        let _ = v.on_message(HWND::default(), MSG_KEYDOWN, wp, LPARAM(0));
        assert!(!v.about_visible);
    }
}
