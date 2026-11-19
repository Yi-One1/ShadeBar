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
use shadebar_core::color::Rgba;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

use crate::config::{Config, Group, Lang, SettingKey};
use crate::render::{RectDip, RenderCtx, TextRole};
use crate::ui::layout::{Metrics, Rect, Scroll};
use crate::ui::pages::diagnostics::{DiagView, paint as paint_diagnostics};
use crate::ui::pages::{SETTING_GROUPS, keys_of};
use crate::ui::theme::{Palette, ThemeMode, CARD_RADIUS_DIP};
use crate::ui::widgets::{
    Banner, GroupHeader, PopupKind, SettingWidget, UiEvent, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP, Widget, WidgetResult,
};

// Win32 消息号（避免为几个常量再开 windows feature）。
const MSG_MOUSEMOVE: u32 = 0x0200;
const MSG_LBUTTONDOWN: u32 = 0x0201;
const MSG_LBUTTONUP: u32 = 0x0202;
const MSG_MOUSEWHEEL: u32 = 0x020A;
const MSG_KEYDOWN: u32 = 0x0100;
const MSG_CHAR: u32 = 0x0102;

/// 诊断卡片展开后的正文高度（dip）。
/// 8 行指标 + 折线 + 两条预警的实际内容高度约 346 dip，留一点余量。
const DIAG_CARD_H: f32 = 360.0;
/// 诊断卡片标题行高度（dip）。收起态只占这一行，点它展开。
const DIAG_HEAD_H: f32 = 44.0;
/// 下拉单项高度（dip）。
const POPUP_ITEM_H: f32 = 28.0;
/// 弹层宽度（dip）。
const POPUP_W: f32 = 220.0;

// ---- 颜色弹层几何 ----------------------------------------------------------
//
// 渲染与命中测试共用下面几个函数，不允许两边各写一份公式：
// 历史事故里"按钮显示区域与交互区域不一致"就是这么来的。
//
// 宽度验算：12 + 6×32 + 5×8 + 12 = 256
// 高度验算（两组）：12 +(16+6+32) +12 +(16+6+32) +10 +32 +8 +32 +12 = 226
// 高度验算（一组）：12 +(16+6+32) +10 +32 +8 +32 +12 = 160
/// 弹层宽度：内边距 + 6 列色块 + 5 个间距 + 内边距。
const PICKER_W: f32 = 256.0;
/// 内边距。
const PICKER_PAD: f32 = 12.0;
/// 单个色块边长（圆角 [`SWATCH_RADIUS`]）。
const SWATCH: f32 = 32.0;
/// 色块间距，横竖同一个值。
const SWATCH_GAP: f32 = 8.0;
/// 色块圆角。
const SWATCH_RADIUS: f32 = 4.0;
/// 焦点环半径（36×36、外扩 2、圆角 8，与色块本身区分开）。
const SWATCH_FOCUS_RADIUS: f32 = 8.0;
/// 焦点环外扩量（32 + 2×2 = 36）。
const SWATCH_FOCUS_OUT: f32 = 2.0;
/// 组标题高度（正文 12 px 那一档）。
const SWATCH_LABEL_H: f32 = 16.0;
/// 组标题 ↔ 色块。
const SWATCH_LABEL_GAP: f32 = 6.0;
/// 组间距。
const SWATCH_GROUP_GAP: f32 = 12.0;
/// 一组占的纵向高度：标题 + 间隙 + 色块。
const SWATCH_ROW_H: f32 = SWATCH_LABEL_H + SWATCH_LABEL_GAP + SWATCH;
/// 色板 ↔ HEX 输入框。
const PICKER_HEX_GAP: f32 = 10.0;
/// HEX 输入框高度。
const PICKER_HEX_H: f32 = 32.0;
/// HEX 标签列宽。
const PICKER_HEX_LABEL_W: f32 = 36.0;
/// HEX ↔「更多颜色…」。
const PICKER_MORE_GAP: f32 = 8.0;
/// 「更多颜色…」按钮高度。
const PICKER_MORE_H: f32 = 32.0;

/// 某组在弹层内的纵向起点。
fn group_top(group: usize) -> f32 {
    PICKER_PAD + group as f32 * (SWATCH_ROW_H + SWATCH_GROUP_GAP)
}

/// 某组某个色块的矩形（内容坐标）。
fn swatch_rect(frame: Rect, group: usize, idx: usize) -> Rect {
    Rect::new(
        frame.x + PICKER_PAD + idx as f32 * (SWATCH + SWATCH_GAP),
        frame.y + group_top(group) + SWATCH_LABEL_H + SWATCH_LABEL_GAP,
        SWATCH,
        SWATCH,
    )
}

/// 某组标题的矩形。
fn swatch_label_rect(frame: Rect, group: usize) -> Rect {
    Rect::new(
        frame.x + PICKER_PAD,
        frame.y + group_top(group),
        frame.w - 2.0 * PICKER_PAD,
        SWATCH_LABEL_H,
    )
}

/// 色板最下沿（最后一组色块的下边），HEX 行从这里往下排。
fn swatches_bottom(groups: usize) -> f32 {
    PICKER_PAD
        + groups as f32 * SWATCH_ROW_H
        + groups.saturating_sub(1) as f32 * SWATCH_GROUP_GAP
}

/// HEX 输入框矩形。`groups` 是当前 key 显示的色板组数（1 或 2）。
fn picker_hex_rect(frame: Rect, groups: usize) -> Rect {
    Rect::new(
        frame.x + PICKER_PAD,
        frame.y + swatches_bottom(groups) + PICKER_HEX_GAP,
        frame.w - 2.0 * PICKER_PAD,
        PICKER_HEX_H,
    )
}

/// "更多颜色"按钮矩形。
fn picker_more_rect(frame: Rect, groups: usize) -> Rect {
    let hr = picker_hex_rect(frame, groups);
    Rect::new(
        frame.x + PICKER_PAD,
        hr.y + PICKER_HEX_H + PICKER_MORE_GAP,
        frame.w - 2.0 * PICKER_PAD,
        PICKER_MORE_H,
    )
}

/// 弹层总高：由上面同一组常量推出来，改一个数整体跟着走（不再手写魔数和）。
fn picker_height(groups: usize) -> f32 {
    let frame = Rect::new(0.0, 0.0, PICKER_W, 0.0);
    picker_more_rect(frame, groups).y + PICKER_MORE_H + PICKER_PAD
}

/// 某个颜色项该看哪几组色板。
///
/// **按 key 分流是这张卡的核心**：端色（S-14/S-15）要的是"文字色"（A 组浅色 +
/// B 组深色），填充色（S-35/S-37）要的是"叠在任务栏上的填充/描边色"（C 组）。
/// 混用会直接出事故：深色背景上的文字拿到 `#FFC857` 对比度只有 1.39:1，
/// 用户根本看不见自己选的文字色。
///
/// 渲染与命中测试都调这一个函数，不许两处各写一份。
#[must_use]
pub fn swatch_groups_for(key: SettingKey) -> &'static [(&'static str, [&'static str; 6])] {
    match key {
        SettingKey::S14 | SettingKey::S15 => &i18n::COLOR_SWATCH_GROUPS[..2],
        _ => &i18n::COLOR_SWATCH_GROUPS[2..],
    }
}

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
    /// 打开系统调色板挑任意颜色（由窗口层执行，因为它需要 HWND）。
    PickColor(SettingKey),
}

/// 弹层状态。
#[derive(Debug, Clone)]
enum PopupState {
    /// 无弹层。
    None,
    /// 下拉选项。
    Dropdown { key: SettingKey, rect: Rect },
    /// 颜色选择（Hex 输入 + 按 key 分流的预设色板）。
    ColorPicker {
        /// 正在编辑的配置项。
        key: SettingKey,
        /// 弹层矩形（内容坐标）。
        rect: Rect,
        /// 用户已敲入的 HEX 文本（初值留空，空时显示当前值灰字提示）。
        hex: String,
        /// 键盘焦点所在色格（`组×6 + 列` 的线性下标）。
        focus: Option<usize>,
    },
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
    /// 颜色策略（S-13）当前是否已是「自定义」。
    ///
    /// 端色改动会由 `Config::set` 自动把 S-13 切到自定义；这里留一份镜像，
    /// 只用于决定"要不要弹一次 `picker.switched` 提示"，不参与任何判定。
    s13_custom: bool,
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
            s13_custom: cfg.get(SettingKey::S13) != 0,
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
        self.s13_custom = cfg.get(SettingKey::S13) != 0;
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

    /// 某个颜色项当前的值（`0xAARRGGBB`）。
    ///
    /// 行被 UI 收起时（S-35/S-37 目前就收着）回退到该键的默认值，不能用白色兜底：
    /// 那样会把"22% 不透明"的 alpha 语义悄悄换成全不透明。
    fn current_color(&self, key: SettingKey) -> u32 {
        self.rows
            .iter()
            .find(|r| r.key == key)
            .map_or(key.meta().default_value as u32, |r| r.value as u32)
    }

    /// 写回一个颜色值；端色第一次落到「自定义」策略时提示一次。
    fn write_color(&mut self, key: SettingKey, argb: i64) {
        if matches!(key, SettingKey::S14 | SettingKey::S15)
            && !self.s13_custom
            && argb != i64::from(self.current_color(key))
        {
            self.show_toast("picker.switched");
        }
        self.actions.push(ViewAction::Set(key, argb));
    }

    /// 点色块：**只换 RGB**，alpha 取该 key 当前值。
    ///
    /// 色块只有 RGB；若直接当 `0xFFRRGGBB` 写回去，S-35（默认 `0x38FFFFFF`，22%）
    /// 会被一次点击突变成全不透明——指示条瞬间从"淡淡的"变成"糊住任务栏"。
    fn apply_swatch(&mut self, key: SettingKey, hex: &str) {
        let Some(c) = Rgba::parse_hex(hex) else {
            return;
        };
        let alpha = self.current_color(key) & 0xFF00_0000;
        let argb = i64::from(alpha | rgb_of(c));
        self.write_color(key, argb);
    }

    /// 提交 HEX 输入。非法 → 不写配置，弹 `picker.invalid`。
    ///
    /// 6 位输入对填充色同样保留当前 alpha（与点色块一致）；8 位输入以用户写的为准。
    fn submit_hex(&mut self, key: SettingKey, hex: &str) {
        let Some(c) = Rgba::parse_hex(hex) else {
            self.show_toast("picker.invalid");
            return;
        };
        let digits = hex.strip_prefix('#').unwrap_or(hex).len();
        let c = if digits == 6 {
            Rgba::new(c.r, c.g, c.b, hex_alpha(self.current_color(key)))
        } else {
            c
        };
        let argb = rgba_to_argb(c);
        self.write_color(key, argb);
    }

    /// 颜色弹层内的按键。返回是否消费。
    fn handle_picker_key(&mut self, vk: u16) -> bool {
        let PopupState::ColorPicker { key, rect, hex, focus } = self.popup.clone() else {
            return false;
        };
        let groups = swatch_groups_for(key);
        let cells = groups.len() * 6;
        match vk {
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
                let cur = focus.unwrap_or(0).min(cells.saturating_sub(1));
                let col = cur % 6;
                let next = match vk {
                    VK_LEFT => cur.saturating_sub(1).max(cur - col),
                    VK_RIGHT => (cur + 1).min(cur - col + 5),
                    // 已在第一排就不动，别把焦点弹回左上角。
                    VK_UP => {
                        if cur < 6 {
                            cur
                        } else {
                            cur - 6
                        }
                    }
                    _ => (cur + 6).min(cells - 1),
                };
                self.popup = PopupState::ColorPicker { key, rect, hex, focus: Some(next) };
                true
            }
            VK_RETURN | VK_SPACE => {
                if !hex.is_empty() {
                    self.submit_hex(key, &hex);
                } else if let Some(idx) = focus {
                    if let Some((_, colors)) = groups.get(idx / 6) {
                        if let Some(p) = colors.get(idx % 6) {
                            self.apply_swatch(key, p);
                        }
                    }
                }
                self.popup = PopupState::None;
                true
            }
            _ => false,
        }
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
                // 采样区过小的预警由 `paint_diagnostics` 自己在正文末尾画，
                // 这里不再叠加（v1.2.0 曾重复画两遍）。
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
            PopupState::ColorPicker { key, rect, hex, focus } => {
                let groups = swatch_groups_for(*key);
                let n = groups.len();
                let f = Rect::new(rect.x, rect.y + dy, PICKER_W, picker_height(n));
                let rd = |q: Rect| RectDip::new(q.x, q.y, q.w, q.h);
                ctx.fill_rounded_rect(rd(f), 8.0, palette.card_bg);
                ctx.stroke_rounded_rect(rd(f), 8.0, palette.accent, 1.0);

                let current = self.current_color(*key);
                let cur_rgb = current & 0x00FF_FFFF;

                // HEX 输入框：标签列 36 宽 + 等宽值列。
                // 当前色不在板内时，标签列改画 16×16 当前色块——用户得看见
                // "我现在是什么色"，否则只能靠猜。位置固定，两种状态零跳动。
                let hr = picker_hex_rect(f, n);
                ctx.stroke_rounded_rect(rd(hr), 4.0, palette.card_border, 1.0);
                let label_x = hr.x + 8.0;
                if in_palette(groups, cur_rgb) {
                    ctx.draw_text(
                        i18n::text("picker.hex", self.lang),
                        RectDip::new(label_x, hr.y + 7.0, PICKER_HEX_LABEL_W, 18.0),
                        TextRole::Body,
                        palette.text_secondary,
                    );
                } else {
                    let chip = RectDip::new(label_x + 10.0, hr.y + 8.0, 16.0, 16.0);
                    ctx.fill_rounded_rect(chip, 2.0, Rgba::from_rgb(cur_rgb));
                    ctx.stroke_rounded_rect(chip, 1.0, palette.card_border, 1.0);
                }
                let value_x = hr.x + 8.0 + PICKER_HEX_LABEL_W;
                let value_w = hr.w - PICKER_HEX_LABEL_W - 16.0;
                // 预填 `#FFFFFFFF` 要退格 9 次才能重输，所以初值留空、
                // 当前值只做灰字提示；第一次敲键提示自然消失。
                if hex.is_empty() {
                    ctx.draw_text(
                        &format!("#{current:08X}"),
                        RectDip::new(value_x + 10.0, hr.y + 7.0, value_w - 10.0, 18.0),
                        TextRole::Body,
                        palette.text_secondary,
                    );
                } else {
                    ctx.draw_text(
                        hex,
                        RectDip::new(value_x, hr.y + 7.0, value_w, 18.0),
                        TextRole::Body,
                        palette.text_primary,
                    );
                }
                ctx.draw_text(
                    "|",
                    RectDip::new(value_x + hex.chars().count() as f32 * 7.4, hr.y + 7.0, 8.0, 18.0),
                    TextRole::Body,
                    palette.accent,
                );

                for (gi, (label_key, colors)) in groups.iter().enumerate() {
                    // 组标题行按施工单是 16 高；正文 14 dip 的行盒约 18.6，
                    // 画在 16 高的框里会把 g/p 的下沿裁掉，所以绘制框放到 20
                    // （只吃 6 dip 的标题↔色块间隙，不占布局高度）。
                    let lr = swatch_label_rect(f, gi);
                    ctx.draw_text(
                        i18n::text(label_key, self.lang),
                        RectDip::new(lr.x, lr.y, lr.w, 20.0),
                        TextRole::Body,
                        palette.text_secondary,
                    );
                    for (i, p) in colors.iter().enumerate() {
                        let sr = swatch_rect(f, gi, i);
                        let Some(c) = Rgba::parse_hex(p) else {
                            continue;
                        };
                        ctx.fill_rounded_rect(rd(sr), SWATCH_RADIUS, c);
                        // 白色块在白卡片上必须描边，否则看不出边界。
                        ctx.stroke_rounded_rect(rd(sr), SWATCH_RADIUS, palette.card_border, 1.0);
                        // 选中态：中心画 12×12 圆角矩形填 ink，再嵌 8×8 原色圆点，
                        // 形成 2 px 环。必须画图形——任何字符（含 U+2713）都可能
                        // 落在 emoji 区段，禁作功能图标。ink 按色块自身亮度取黑/白。
                        if rgb_of(c) == cur_rgb {
                            let ink = if c.relative_luminance() > 0.5 {
                                Rgba::opaque(0, 0, 0)
                            } else {
                                Rgba::opaque(0xFF, 0xFF, 0xFF)
                            };
                            ctx.fill_rounded_rect(
                                RectDip::new(sr.x + 10.0, sr.y + 10.0, 12.0, 12.0),
                                6.0,
                                ink,
                            );
                            ctx.fill_rounded_rect(
                                RectDip::new(sr.x + 12.0, sr.y + 12.0, 8.0, 8.0),
                                4.0,
                                c,
                            );
                        }
                        // 键盘焦点环：2 px accent、外扩 2（36×36、圆角 8）。
                        // 形状与对勾完全不同，两者叠加也分得清。
                        if *focus == Some(gi * 6 + i) {
                            let fr = sr.inset(-SWATCH_FOCUS_OUT);
                            ctx.stroke_rounded_rect(rd(fr), SWATCH_FOCUS_RADIUS, palette.accent, 2.0);
                        }
                    }
                }

                let mr = picker_more_rect(f, n);
                ctx.stroke_rounded_rect(rd(mr), 4.0, palette.card_border, 1.0);
                ctx.draw_text(
                    i18n::text("picker.more", self.lang),
                    RectDip::new(mr.x + 8.0, mr.y + 7.0, mr.w - 16.0, 18.0),
                    TextRole::Body,
                    palette.accent,
                );
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
                let groups = swatch_groups_for(key);
                let n = groups.len();
                let (w, h) = (PICKER_W, picker_height(n));
                // 初始焦点：当前色在板内就落在那一格，否则落在左上角第一格。
                let focus = groups
                    .iter()
                    .enumerate()
                    .flat_map(|(gi, (_, colors))| {
                        colors.iter().enumerate().map(move |(i, p)| (gi * 6 + i, *p))
                    })
                    .find(|(_, p)| {
                        Rgba::parse_hex(p)
                            .is_some_and(|c| rgb_of(c) == self.current_color(key) & 0x00FF_FFFF)
                    })
                    .map_or(0, |(idx, _)| idx);
                // 吸附在窗口内：右侧越界就左移，下方放不下就翻到锚点上方。
                let left = x.min(self.client.0 - w - 12.0).max(8.0);
                let below_ok = (y - self.scroll.offset) + h + 12.0 <= self.client.1;
                let top = if below_ok {
                    y + 6.0
                } else {
                    (y - h - 6.0).max(self.scroll.offset + 8.0)
                };
                self.popup = PopupState::ColorPicker {
                    key,
                    rect: Rect::new(left, top, w, h),
                    // 初值留空：预填 `#FFFFFFFF` 要退格 9 次才能重输，
                    // 当前值只做灰字提示（见 `paint_popup`）。
                    hex: String::new(),
                    focus: Some(focus),
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
            PopupState::ColorPicker { key, rect, hex, focus } => {
                // 命中与渲染共用同一组几何函数：色块走 swatch_rect、按钮走
                // picker_more_rect，绝不另写一份坐标公式（否则就会出现
                // "看到的位置点不中、点中的地方没按钮"这类问题）。
                let groups = swatch_groups_for(key);
                let n = groups.len();
                for (gi, (_, colors)) in groups.iter().enumerate() {
                    for (i, p) in colors.iter().enumerate() {
                        if swatch_rect(rect, gi, i).contains(x, y) {
                            self.apply_swatch(key, p);
                            self.popup = PopupState::None;
                            return true;
                        }
                    }
                }
                if picker_more_rect(rect, n).contains(x, y) {
                    self.actions.push(ViewAction::PickColor(key));
                    self.popup = PopupState::None;
                    return true;
                }
                // 点弹层内部（HEX 输入区）保持打开，等键盘输入；点外面关闭。
                if !Rect::new(rect.x, rect.y, PICKER_W, picker_height(n)).contains(x, y) {
                    self.popup = PopupState::None;
                } else {
                    self.popup = PopupState::ColorPicker { key, rect, hex, focus };
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
                // 颜色弹层优先吃掉方向键 / Enter / Space：打开弹层时这四类键
                // 属于弹层，不能再往下走给页面控件。
                if matches!(self.popup, PopupState::ColorPicker { .. })
                    && self.handle_picker_key(vk)
                {
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
                let consumed = self.handle_row_event(&UiEvent::Key { vk }, 0.0, 0.0);
                if consumed { Some(LRESULT(0)) } else { None }
            }
            MSG_CHAR => {
                let ch = char::from_u32((wp.0 & 0xFFFF) as u32).unwrap_or('\0');
                if ch == '\0' {
                    return None;
                }
                let state = self.popup.clone();
                if let PopupState::ColorPicker { key, rect, mut hex, focus } = state {
                    if ch == '\u{0008}' {
                        hex.pop();
                    } else if (ch == '#' || ch.is_ascii_hexdigit()) && hex.chars().count() < 9 {
                        // `#` + 8 位 = 9；3 位简写不支持（§11.5 只认 6/8 位）。
                        hex.push(ch.to_ascii_uppercase());
                    }
                    self.popup = PopupState::ColorPicker { key, rect, hex, focus };
                    return Some(LRESULT(0));
                }
                let consumed = self.handle_row_event(&UiEvent::Char { ch }, 0.0, 0.0);
                if consumed { Some(LRESULT(0)) } else { None }
            }
            _ => None,
        }
    }
}

/// `Rgba` → `0xAARRGGBB`（配置值口径）。
fn rgba_to_argb(c: Rgba) -> i64 {
    i64::from((u32::from(c.a) << 24) | rgb_of(c))
}

/// `Rgba` → `0x00RRGGBB`（丢 alpha，用于"当前色是否在板内"与命中比较）。
fn rgb_of(c: Rgba) -> u32 {
    (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
}

/// 取 `0xAARRGGBB` 的 alpha 字节。
fn hex_alpha(argb: u32) -> u8 {
    (argb >> 24) as u8
}

/// 某个 RGB 是否出现在给定色板里（决定 HEX 行左侧画「HEX」标签还是当前色块）。
fn in_palette(groups: &[(&'static str, [&'static str; 6])], rgb: u32) -> bool {
    groups
        .iter()
        .flat_map(|(_, colors)| colors.iter())
        .any(|p| Rgba::parse_hex(p).is_some_and(|c| rgb_of(c) == rgb))
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
        let argb = |s: &str| Rgba::parse_hex(s).map(rgba_to_argb);
        // 6 位 → a = 255
        assert_eq!(argb("#FFFFFF"), Some(0x00FF_FFFF | 0xFF00_0000));
        assert_eq!(Rgba::parse_hex("112233").map(|c| c.a), Some(255));
        // 8 位 → 自带 alpha
        assert_eq!(argb("#80112233"), Some(0x8011_2233));
        assert_eq!(Rgba::parse_hex("38FFFFFF").map(|c| c.a), Some(0x38));
        // 非法：3 位简写 / 7 位 / 空串 / 非十六进制
        assert_eq!(argb("#FFF"), None);
        assert_eq!(argb("#FFFFFFF"), None);
        assert_eq!(argb(""), None);
        assert_eq!(argb("#GGGGGG"), None);
        // 非 ASCII 不得 panic（本地副本曾无守卫，切字符串时会崩）
        assert_eq!(argb("颜颜"), None);
        assert_eq!(argb("#颜色颜色"), None);
    }

    #[test]
    fn picker_groups_are_selected_by_key_not_shared() {
        // 端色 → 文字色两组（A 深底 / B 浅底）
        assert_eq!(swatch_groups_for(SettingKey::S14).len(), 2);
        assert_eq!(swatch_groups_for(SettingKey::S15).len(), 2);
        // 填充色 → 一组
        assert_eq!(swatch_groups_for(SettingKey::S35).len(), 1);
        assert_eq!(swatch_groups_for(SettingKey::S37).len(), 1);
        // 两组必须真的是 A + B，且不能混进填充色（C 组的 #E5484D 等）
        let text_keys: Vec<&str> = swatch_groups_for(SettingKey::S14).iter().map(|(k, _)| *k).collect();
        assert_eq!(text_keys, vec!["picker.on_dark", "picker.on_light"]);
        assert_eq!(swatch_groups_for(SettingKey::S35)[0].0, "picker.fill");
        assert!(!swatch_groups_for(SettingKey::S14)
            .iter()
            .any(|(_, cs)| cs.contains(&"#E5484D")));
        // C 组必须是设计师定稿的 6 个具体色值，且一个都不能漏进文字色板
        let fill: Vec<&str> = swatch_groups_for(SettingKey::S35)[0].1.to_vec();
        assert_eq!(
            fill,
            vec!["#E5484D", "#C96A00", "#12855A", "#0E8D8D", "#3B82F6", "#8B5CF6"]
        );
        let text: Vec<&str> = swatch_groups_for(SettingKey::S14)
            .iter()
            .flat_map(|(_, cs)| cs.iter().copied())
            .collect();
        for c in fill {
            assert!(!text.contains(&c), "填充色 {c} 混进了文字色板");
        }
    }

    #[test]
    fn picker_geometry_fits_inside_and_never_overlaps() {
        let check = |frame: Rect, groups: usize| {
            let mut rects: Vec<Rect> = Vec::new();
            for gi in 0..groups {
                for i in 0..6 {
                    let r = swatch_rect(frame, gi, i);
                    assert!(
                        r.x >= frame.x && r.y >= frame.y
                            && r.x + r.w <= frame.x + frame.w
                            && r.y + r.h <= frame.y + frame.h,
                        "色块越出弹层: {r:?}"
                    );
                    for other in &rects {
                        let overlap_x = r.x < other.x + other.w && other.x < r.x + r.w;
                        let overlap_y = r.y < other.y + other.h && other.y < r.y + r.h;
                        assert!(!(overlap_x && overlap_y), "色块重叠: {r:?} / {other:?}");
                    }
                    rects.push(r);
                }
            }
            // 一行 6 列刚好铺满内容宽度：12 + 6×32 + 5×8 + 12 = 256
            let last = swatch_rect(frame, 0, 5);
            assert!((last.x + last.w - (frame.x + frame.w - PICKER_PAD)).abs() < 1e-3);
            let bottom = rects.iter().map(|r| r.y + r.h).fold(0.0_f32, f32::max);
            let hr = picker_hex_rect(frame, groups);
            let mr = picker_more_rect(frame, groups);
            assert!((hr.y - bottom - PICKER_HEX_GAP).abs() < 1e-3, "色板↔HEX 必须是 10");
            assert!((mr.y - hr.y - PICKER_HEX_H - PICKER_MORE_GAP).abs() < 1e-3);
            assert!(mr.y >= bottom, "「更多颜色…」必须在色块下方");
            assert!(mr.y + mr.h <= frame.y + frame.h, "底部按钮越出弹层");
        };
        // frame 用规格常量，不能用 picker_height() 自己当期望值（那是自我证明）
        check(Rect::new(0.0, 0.0, 256.0, 226.0), 2);
        check(Rect::new(0.0, 0.0, 256.0, 160.0), 1);
        assert!((picker_height(2) - 226.0).abs() < 1e-3);
        assert!((picker_height(1) - 160.0).abs() < 1e-3);
    }

    #[test]
    fn every_swatch_click_writes_that_exact_colour_and_keeps_alpha() {
        for key in [SettingKey::S14, SettingKey::S35] {
            let mut v = view();
            v.set_client(720.0, 560.0);
            // 规格常量，不取运行时值：端色恒 0xFF，填充色默认 0x38
            let alpha = if key == SettingKey::S14 {
                0xFF00_0000_u32
            } else {
                0x3800_0000_u32
            };
            assert_eq!(v.current_color(key) & 0xFF00_0000, alpha);
            let groups = swatch_groups_for(key);
            for (gi, (_, colors)) in groups.iter().enumerate() {
                for (i, p) in colors.iter().enumerate() {
                    v.open_popup(PopupKind::ColorPicker(key), 80.0, 120.0);
                    let rect = match &v.popup {
                        PopupState::ColorPicker { rect, .. } => *rect,
                        other => panic!("弹层没打开: {other:?}"),
                    };
                    let r = swatch_rect(rect, gi, i);
                    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    assert!(v.handle_popup_click(cx, cy), "色块中心必须命中");
                    let acts = v.take_actions();
                    assert_eq!(acts.len(), 1, "点一格只产生一个动作");
                    let want = i64::from(
                        alpha | rgb_of(Rgba::parse_hex(p).unwrap_or(Rgba::opaque(0, 0, 0))),
                    );
                    assert_eq!(acts[0], ViewAction::Set(key, want), "{key:?} 的第 {gi}/{i} 格");
                    let got = match acts[0] {
                        ViewAction::Set(_, v) => v as u32,
                        _ => 0,
                    };
                    assert_eq!(
                        (got >> 24) as u8,
                        if key == SettingKey::S14 { 0xFF } else { 0x38 },
                        "点击后 alpha 不能被写成 0xFF"
                    );
                }
            }
        }
    }

    #[test]
    fn picker_keys_move_focus_and_apply() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        v.open_popup(PopupKind::ColorPicker(SettingKey::S14), 80.0, 120.0);
        // 默认焦点落在 #FFFFFF（A 组第 1 格），右移一格到 #FFB4AB
        let wp = |vk: u16| WPARAM(vk as usize);
        assert!(v.on_message(HWND::default(), MSG_KEYDOWN, wp(VK_RIGHT), LPARAM(0)).is_some());
        assert!(matches!(v.popup, PopupState::ColorPicker { focus: Some(1), .. }));
        // 下移到 B 组同一列
        assert!(v.on_message(HWND::default(), MSG_KEYDOWN, wp(VK_DOWN), LPARAM(0)).is_some());
        assert!(matches!(v.popup, PopupState::ColorPicker { focus: Some(7), .. }));
        assert!(v.on_message(HWND::default(), MSG_KEYDOWN, wp(VK_RETURN), LPARAM(0)).is_some());
        assert!(matches!(v.popup, PopupState::None), "应用后必须关闭");
        assert_eq!(
            v.take_actions(),
            vec![ViewAction::Set(SettingKey::S14, 0xFF00_0000 | 0x007A_1F2B)]
        );
    }

    #[test]
    fn invalid_hex_closes_popup_with_toast_and_writes_nothing() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        v.open_popup(PopupKind::ColorPicker(SettingKey::S14), 80.0, 120.0);
        // 敲 3 位（不支持简写）后回车
        for ch in ['F', 'F', 'F'] {
            let _ = v.on_message(HWND::default(), MSG_CHAR, WPARAM(ch as usize), LPARAM(0));
        }
        let _ = v.on_message(
            HWND::default(),
            MSG_KEYDOWN,
            WPARAM(VK_RETURN as usize),
            LPARAM(0),
        );
        assert!(matches!(v.popup, PopupState::None));
        assert!(v.take_actions().is_empty(), "非法输入绝不写配置");
        assert_eq!(v.toast.map(|t| t.0), Some("picker.invalid"));
    }

    #[test]
    fn valid_hex_submits_and_keeps_fill_alpha() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        // S-35 当前是 0x38FFFFFF（22%），输入 6 位后 alpha 必须还是 0x38
        v.open_popup(PopupKind::ColorPicker(SettingKey::S35), 80.0, 120.0);
        for ch in "#E5484D".chars() {
            let _ = v.on_message(HWND::default(), MSG_CHAR, WPARAM(ch as usize), LPARAM(0));
        }
        let _ = v.on_message(
            HWND::default(),
            MSG_KEYDOWN,
            WPARAM(VK_RETURN as usize),
            LPARAM(0),
        );
        assert_eq!(
            v.take_actions(),
            vec![ViewAction::Set(SettingKey::S35, 0x38E5_484D)]
        );
        // 8 位则完全以用户输入为准
        v.open_popup(PopupKind::ColorPicker(SettingKey::S35), 80.0, 120.0);
        for ch in "80E5484D".chars() {
            let _ = v.on_message(HWND::default(), MSG_CHAR, WPARAM(ch as usize), LPARAM(0));
        }
        let _ = v.on_message(
            HWND::default(),
            MSG_KEYDOWN,
            WPARAM(VK_RETURN as usize),
            LPARAM(0),
        );
        assert_eq!(
            v.take_actions(),
            vec![ViewAction::Set(SettingKey::S35, 0x80E5_484D)]
        );
    }

    #[test]
    fn end_colour_change_prompts_once_for_strategy_switch() {
        let mut v = view();
        v.set_client(720.0, 560.0);
        v.open_popup(PopupKind::ColorPicker(SettingKey::S14), 80.0, 120.0);
        let rect = match &v.popup {
            PopupState::ColorPicker { rect, .. } => *rect,
            other => panic!("弹层没打开: {other:?}"),
        };
        let r = swatch_rect(rect, 0, 1); // #FFB4AB
        assert!(v.handle_popup_click(r.x + r.w / 2.0, r.y + r.h / 2.0));
        assert_eq!(v.take_actions().len(), 1);
        assert_eq!(v.toast.map(|t| t.0), Some("picker.switched"), "策略自动切换时提示一次");
        // 已是自定义之后不再提示
        v.toast = None;
        v.s13_custom = true;
        v.open_popup(PopupKind::ColorPicker(SettingKey::S14), 80.0, 120.0);
        let rect = match &v.popup {
            PopupState::ColorPicker { rect, .. } => *rect,
            other => panic!("弹层没打开: {other:?}"),
        };
        let r = swatch_rect(rect, 1, 1); // #7A1F2B
        assert!(v.handle_popup_click(r.x + r.w / 2.0, r.y + r.h / 2.0));
        assert_ne!(v.toast.map(|t| t.0), Some("picker.switched"));
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
