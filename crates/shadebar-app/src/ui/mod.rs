//! 设置窗口（G6 重做：方案 1「任务栏直控」）。
//!
//! # 结构
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │ 任务栏  采样  托盘  诊断        Y 0.183·暗端  ●运行中 着色 ◉  暂停 │ 48
//! ├──────────────────────────────────────────────────────────────┤
//! │ 内容区（按页手工排布，720×560 一屏放下，不滚动）                  │
//! ├──────────────────────────────────────────────────────────────┤
//! │ 恢复默认                              导入配置  导出配置  关于   │ 48
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # 为什么不再有自绘标题栏
//!
//! 窗口本身是 `WS_OVERLAPPEDWINDOW`，DWM 已经提供标题栏与关闭按钮；
//! 再自绘一条 40 px 标题区只会形成"双层标题栏"。这里只画页面头。
//!
//! # 为什么没有"标签 + 说明 + 控件"三行模板
//!
//! 布局本身承担解释职责：任务栏预览直接把三区、颜色、阈值画出来，
//! 开关就贴在被影响的位置上；因此不再有每行一句灰色说明文字。
//!
//! # 坐标
//!
//! 本文件所有坐标都是 **dip，且等于客户区坐标**（无侧栏偏移、无滚动）。
//! 浮层锚点直接用控件矩形，命中与绘制共用同一份几何。

pub mod i18n;
pub mod layout;
pub mod pages;
pub mod theme;
pub mod widgets;

use std::time::Instant;

use shadebar_core::color::Rgba;
use shadebar_core::SbResult;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, SYSTEMTIME, WPARAM};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{LoadCursorW, SetCursor, IDC_ARROW, IDC_HAND};

use crate::config::{argb_to_rgba, Config, ControlKind, Lang, SettingKey};
use crate::render::{RectDip, RenderCtx, TextRole};
use crate::ui::i18n::text;
use crate::ui::layout::{ellipsize, hex_rgb, text_width, Rect};
use crate::ui::pages::diagnostics::DiagView;
use crate::ui::theme::{Palette, ThemeMode};
use crate::ui::widgets::{
    PopupKind, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

const HEADER_H: f32 = theme::HEADER_HEIGHT_DIP;
const FOOTER_H: f32 = theme::FOOTER_HEIGHT_DIP;
const PAD: f32 = theme::PAGE_PAD_DIP;
const GAP: f32 = theme::PAGE_GAP_DIP;
const ROW_H: f32 = theme::ROW_HEIGHT_DIP;

/// 分区独立配色的 2×4 矩阵：行 0 = 背景偏暗（浅色），行 1 = 背景偏亮（深色）。
const ITEM_KEYS: [[SettingKey; 4]; 2] = [
    [SettingKey::S52, SettingKey::S54, SettingKey::S56, SettingKey::S58],
    [SettingKey::S53, SettingKey::S55, SettingKey::S57, SettingKey::S59],
];
/// 矩阵四列的标题（4 项内容）。
const ITEM_LABELS: [&str; 4] = ["item.indicator", "item.indicator_sel", "item.tray", "item.clock"];
const TOGGLE_W: f32 = theme::TOGGLE_W_DIP;
const TOGGLE_H: f32 = theme::TOGGLE_H_DIP;
const RADIUS: f32 = theme::TILE_RADIUS_DIP;

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KILLFOCUS: u32 = 0x0008;

/// 诊断页上的固定动作行（不参与页面设置项归属）。
const DIAG_ROWS: &[SettingKey] = &[SettingKey::S45, SettingKey::S05, SettingKey::S46];
const NO_ROWS: &[SettingKey] = &[];

/// 版本号（编译期常量，点击即复制）。
const VERSION_TEXT: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// 页面。顺序即页签顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// 任务栏：预览 + 三区 + 颜色与判定。
    Taskbar,
    /// 采样：采样带 + 间隔/平滑/区域。
    Sample,
    /// 托盘：托盘行为 + 启动 + 语言。
    Tray,
    /// 诊断：实时判定 + 日志级别 + 版本 + 诊断包。
    Diag,
}

impl Page {
    const ALL: [Self; 4] = [Self::Taskbar, Self::Sample, Self::Tray, Self::Diag];

    fn title_key(self) -> &'static str {
        match self {
            Self::Taskbar => "tab.taskbar",
            Self::Sample => "tab.sample",
            Self::Tray => "tab.tray",
            Self::Diag => "tab.diag",
        }
    }

}

/// 命中目标。命中与绘制共用同一份矩形，测试据此核对"视觉 == 命中"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// 无。
    None,
    /// 页签。
    Tab(usize),
    /// 顶栏「着色」开关（S-01）。
    Tint,
    /// 顶栏「暂停 / 继续」（S-42）。
    Pause,
    /// 预览上的背景模式（S-10）。
    Mode(usize),
    /// 三区卡片（0 图标 / 1 托盘 / 2 时钟）。
    Zone(usize),
    /// 配色矩阵（行, 列）。
    ItemWell(usize, usize),
    /// 正常指示条不透明度滑条（S-60）。
    Opacity,
    /// 判定粒度（S-47）。
    Judge(usize),
    /// 亮度阈值滑条（S-12）。
    Threshold,
    /// 设置行。
    Row(usize),
    /// 底栏（0 恢复默认 / 1 导入 / 2 导出 / 3 关于）。
    Footer(usize),
}

/// 行控件。
struct RowWidget {
    key: SettingKey,
    /// 整行命中区。
    rect: Rect,
    value: i64,
    enabled: bool,
}

/// 浮层状态。
struct PopupState {
    kind: PopupKind,
    /// 浮层矩形（客户区坐标）。
    frame: Rect,
    /// 下拉当前选中项。
    selected: usize,
}

/// 视图动作（由 `window.rs` 执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewAction {
    /// 写一项配置。
    Set(SettingKey, i64),
    /// 恢复默认（已二次确认）。
    RestoreDefaults,
    /// 导出诊断包。
    ExportDiagnostics,
    /// 复制版本号。
    CopyVersion,
    /// 打开系统取色对话框。
    PickColor(SettingKey),
    /// 导出配置。
    ExportConfig,
    /// 导入配置。
    ImportConfig,
}

/// 运行状态 chip。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Running,
    Paused,
    Limited,
}

/// 设置窗口视图。
pub struct View {
    cfg: Config,
    diag: DiagView,
    lang: Lang,
    mode: ThemeMode,
    dpi: u32,
    degraded: bool,
    page: Page,
    rows: Vec<RowWidget>,
    actions: Vec<ViewAction>,
    popup: Option<PopupState>,
    toast: Option<(String, Instant)>,
    hover: Hit,
    focus: usize,
    focus_visible: bool,
    dragging: Option<SettingKey>,
    client: (f32, f32),

    tab_rects: [Rect; 4],
    tint_toggle: Rect,
    pause_btn: Rect,
    preview: Rect,
    preview_bar: Rect,
    preview_clock: Rect,
    tray_chevron: Rect,
    tray_glyphs: [Rect; 3],
    mode_rects: [Rect; 3],
    zone_rects: [Rect; 3],
    colors_card: Rect,
    /// 4 项内容 × 2 背景态的色块矩阵（行 0 背景偏暗 / 行 1 背景偏亮）。
    item_wells: [[Rect; 4]; 2],
    /// 正常指示条不透明度滑条（S-60）。
    opacity_track: Rect,
    judge_rects: [Rect; 2],
    thr_track: Rect,
    sample_card: Rect,
    sample_band: Rect,
    sample_bar: Rect,
    rows_card: Rect,
    diag_card: Rect,
    diag_spark: Rect,
    foot_rects: [Rect; 4],
}

impl View {
    /// 新建视图。
    #[must_use]
    pub fn new(cfg: &Config, lang: Lang, degraded: bool) -> Self {
        let mut v = Self {
            cfg: cfg.clone(),
            diag: DiagView::default(),
            lang,
            mode: ThemeMode::Light,
            dpi: 96,
            degraded,
            page: Page::Taskbar,
            rows: Vec::new(),
            actions: Vec::new(),
            popup: None,
            toast: None,
            hover: Hit::None,
            focus: usize::MAX,
            focus_visible: false,
            dragging: None,
            client: (theme::MIN_WIDTH_DIP, theme::MIN_HEIGHT_DIP),
            tab_rects: [Rect::default(); 4],
            tint_toggle: Rect::default(),
            pause_btn: Rect::default(),
            preview: Rect::default(),
            preview_bar: Rect::default(),
            preview_clock: Rect::default(),
            tray_chevron: Rect::default(),
            tray_glyphs: [Rect::default(); 3],
            mode_rects: [Rect::default(); 3],
            zone_rects: [Rect::default(); 3],
            colors_card: Rect::default(),
            item_wells: [[Rect::default(); 4]; 2],
            opacity_track: Rect::default(),
            judge_rects: [Rect::default(); 2],
            thr_track: Rect::default(),
            sample_card: Rect::default(),
            sample_band: Rect::default(),
            sample_bar: Rect::default(),
            rows_card: Rect::default(),
            diag_card: Rect::default(),
            diag_spark: Rect::default(),
            foot_rects: [Rect::default(); 4],
        };
        v.rebuild_rows();
        v.rebuild_layout();
        v
    }

    /// 用最新配置 / 诊断数据刷新（每帧 `WM_PAINT` 前由 `app.sync_view` 调用）。
    pub fn rebuild(&mut self, cfg: &Config, diag: &DiagView, degraded: bool) {
        self.cfg = cfg.clone();
        self.diag = diag.clone();
        self.degraded = degraded;
        self.rebuild_rows();
        self.rebuild_layout();
    }

    /// 语言变化：文案即时切换，浮层关闭（旧语言的选项文字不能留在屏幕上）。
    pub fn rebuild_texts(&mut self, lang: Lang) {
        if lang == self.lang {
            return;
        }
        self.lang = lang;
        self.popup = None;
        self.rebuild_rows();
        self.rebuild_layout();
    }

    /// 取动作队列（由 `window.rs` 消费）。
    pub fn take_actions(&mut self) -> Vec<ViewAction> {
        std::mem::take(&mut self.actions)
    }

    /// 提示条（i18n key）。
    pub fn show_toast(&mut self, key: &'static str) {
        self.show_toast_text(text(key, self.lang).to_string());
    }

    /// 提示条（现成文本）。
    pub fn show_toast_text(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    /// 设置 DPI。
    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
    }

    /// 当前 DPI。
    #[must_use]
    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// 设置深 / 浅色。
    pub fn set_theme(&mut self, mode: ThemeMode) {
        self.mode = mode;
    }

    /// 当前主题。
    #[must_use]
    pub fn theme(&self) -> ThemeMode {
        self.mode
    }

    /// 当前语言。
    #[must_use]
    pub fn language(&self) -> Lang {
        self.lang
    }

    /// 客户区尺寸（dip）。
    pub fn set_client(&mut self, w: f32, h: f32) {
        if (self.client.0 - w).abs() < 0.01 && (self.client.1 - h).abs() < 0.01 {
            return;
        }
        self.client = (w, h);
        self.rebuild_layout();
    }

    // -----------------------------------------------------------------
    // 数据
    // -----------------------------------------------------------------

    fn tint_on(&self) -> bool {
        self.cfg.get(SettingKey::S01) != 0
    }

    fn paused(&self) -> bool {
        !self.tint_on() || self.cfg.get(SettingKey::S42) != 0
    }

    fn run_state(&self) -> RunState {
        if self.paused() {
            RunState::Paused
        } else if self.degraded {
            RunState::Limited
        } else {
            RunState::Running
        }
    }

    /// 预览用的"背景是否偏亮"（S-10 强制端色优先）。
    fn bg_is_light(&self) -> bool {
        match self.cfg.get(SettingKey::S10) {
            1 => false,
            2 => true,
            _ => {
                let thr = self.cfg.get(SettingKey::S12) as f32 / 1000.0;
                let y = self.diag.y_milli as f32 / 1000.0;
                y >= thr
            }
        }
    }

    /// 预览条是否用浅色（亮端）背景。
    ///
    /// 「亮端」= 亮端色 = 浅色文字，所以任务栏条必须**深色**才看得见；
    /// 强制暗端色则反过来。这里曾把两者画反（问题 4）。
    ///
    /// * `S-10` 强制亮端色 → 深色条 + 浅色字；
    /// * `S-10` 强制暗端色 → 浅色条 + 深色字；
    /// * 自动 → 与 [`Self::bg_is_light`] 同源：亮度高用浅色条（配深色文字），
    ///   亮度低用深色条（配浅色文字）；
    /// * 暂停 / 关闭 / 降级时元素回落系统白，条必须保持深色，否则白字看不见。
    fn preview_bar_light(&self) -> bool {
        if self.degraded || self.paused() {
            return false;
        }
        match self.cfg.get(SettingKey::S10) {
            1 => false,
            2 => true,
            _ => self.bg_is_light(),
        }
    }

    /// 当前预览条的三件套：底色 / 前景 / 描边。
    fn bar_colors(&self, p: &Palette) -> (Rgba, Rgba, Rgba) {
        if self.preview_bar_light() {
            (p.preview_bar_light, p.preview_ink_dark, p.preview_line_light)
        } else {
            (p.preview_bar, p.preview_ink, p.preview_line)
        }
    }

    /// 某一区实际会用的前景色（关闭 / 暂停 / 降级时回落到条上的前景色）。
    ///
    /// 颜色始终按内容取 S-52~S-59（问题 2：配色与判定粒度解耦）；
    /// 预览只展示图标区的采样亮度，用 [`Self::bg_is_light`] 决定取哪一列。
    fn zone_color(&self, key: SettingKey, p: &Palette) -> Rgba {
        if self.degraded || self.paused() || self.cfg.get(key) == 0 {
            let (_, ink, _) = self.bar_colors(p);
            return ink;
        }
        let (dark_key, light_key) = match key {
            SettingKey::S48 => (SettingKey::S58, SettingKey::S59),
            SettingKey::S49 => (SettingKey::S56, SettingKey::S57),
            _ => (SettingKey::S52, SettingKey::S53),
        };
        argb_to_rgba(if self.bg_is_light() {
            self.cfg.get(light_key)
        } else {
            self.cfg.get(dark_key)
        })
    }

    /// 采样段：与 recolor 的真实生效范围一致 —— 起点 ≤50%、宽度 10~84%，
    /// 末端不超过托盘区起点（84%）。预览不能画出实际采不到的区域。
    fn sample_segment(&self) -> Rect {
        let bar = self.sample_bar;
        let start = (self.cfg.get(SettingKey::S20) as f32 / 100.0).clamp(0.0, 0.5);
        let width = (self.cfg.get(SettingKey::S21) as f32 / 100.0).clamp(0.1, 0.84);
        let dy = band_inset(bar, self.cfg.get(SettingKey::S22));
        let x0 = bar.x + bar.w * start;
        let end = (bar.x + bar.w * (start + width)).min(bar.x + bar.w * 0.84);
        Rect::new(x0, bar.y + dy, (end - x0).max(4.0), (bar.h - 2.0 * dy).max(2.0))
    }

    fn row_enabled(&self, key: SettingKey) -> bool {
        match self.page {
            Page::Sample => self.tint_on() && !self.degraded && key.meta().kind == ControlKind::Slider,
            _ => key != SettingKey::S46 || self.tint_on(),
        }
    }

    // -----------------------------------------------------------------
    // 布局
    // -----------------------------------------------------------------

    fn rebuild_rows(&mut self) {
        let keys: &[SettingKey] = match self.page {
            Page::Taskbar => NO_ROWS,
            Page::Sample => pages::SAMPLE_KEYS,
            Page::Tray => pages::TRAY_KEYS,
            Page::Diag => DIAG_ROWS,
        };
        self.rows.clear();
        let (card, pad_top) = self.rows_geometry();
        for (i, &key) in keys.iter().enumerate() {
            self.rows.push(RowWidget {
                key,
                rect: Rect::new(card.x, card.y + pad_top + i as f32 * ROW_H, card.w, ROW_H),
                value: self.cfg.get(key),
                enabled: self.row_enabled(key),
            });
        }
        if self.focus >= self.rows.len() {
            self.focus = usize::MAX;
        }
    }

    fn rows_geometry(&self) -> (Rect, f32) {
        let w = self.client.0;
        let cw = w - PAD * 2.0;
        match self.page {
            Page::Sample => (Rect::new(PAD, 236.0, cw, 244.0), 12.0),
            Page::Tray => {
                // 卡片高度跟着行数走：新增「左键双击」后是 6 行，
                // 固定 244 会把最后一行挤到卡片外面。
                let n = pages::TRAY_KEYS.len() as f32;
                (Rect::new(PAD, HEADER_H + PAD, cw, 24.0 + n * ROW_H), 12.0)
            }
            Page::Diag => (Rect::new(PAD, 272.0, cw, 144.0), 6.0),
            Page::Taskbar => (Rect::default(), 0.0),
        }
    }

    fn rebuild_layout(&mut self) {
        let w = self.client.0.max(theme::MIN_WIDTH_DIP);
        let h = self.client.1.max(theme::MIN_HEIGHT_DIP);

        // 页签
        let mut x = PAD;
        for (i, page) in Page::ALL.iter().enumerate() {
            let tw = text_width(text(page.title_key(), self.lang), TextRole::Subtitle.size())
                .max(26.0);
            self.tab_rects[i] = Rect::new(x, 0.0, tw + 28.0, HEADER_H);
            x += tw + 28.0 + 2.0;
        }

        // 顶栏右侧：暂停 → 着色开关
        let pause_label = text(
            if self.paused() { "ui.resume" } else { "ui.pause" },
            self.lang,
        );
        let pw = text_width(pause_label, TextRole::Body.size()) + 30.0;
        self.pause_btn = Rect::new(w - PAD - pw, 10.0, pw, 28.0);
        self.tint_toggle = Rect::new(self.pause_btn.x - 16.0 - TOGGLE_W, 15.0, TOGGLE_W, TOGGLE_H);

        // 底栏
        let fy = h - FOOTER_H + 10.0;
        let body = TextRole::Body.size();
        let restore_w = text_width(text("action.restore", self.lang), body) + 26.0;
        self.foot_rects[0] = Rect::new(PAD, fy, restore_w, 28.0);
        let about_w = text_width(text("action.about", self.lang), body) + 26.0;
        let export_w = text_width(text("action.export_config", self.lang), body) + 26.0;
        let import_w = text_width(text("action.import_config", self.lang), body) + 26.0;
        let mut rx = w - PAD - about_w;
        self.foot_rects[3] = Rect::new(rx, fy, about_w, 28.0);
        rx -= export_w + 4.0;
        self.foot_rects[2] = Rect::new(rx, fy, export_w, 28.0);
        rx -= import_w + 4.0;
        self.foot_rects[1] = Rect::new(rx, fy, import_w, 28.0);

        // 各页几何
        let cw = w - PAD * 2.0;
        let top = HEADER_H + PAD;
        self.preview = Rect::default();
        self.preview_bar = Rect::default();
        self.preview_clock = Rect::default();
        self.tray_chevron = Rect::default();
        self.tray_glyphs = [Rect::default(); 3];
        self.colors_card = Rect::default();
        self.sample_card = Rect::default();
        self.sample_band = Rect::default();
        self.sample_bar = Rect::default();
        self.diag_card = Rect::default();
        self.diag_spark = Rect::default();
        match self.page {
            Page::Taskbar => {
                self.preview = Rect::new(PAD, top, cw, 150.0);
                self.preview_bar = Rect::new(PAD + 10.0, top + 150.0 - 54.0, cw - 20.0, 44.0);
                let bar = self.preview_bar;
                // 时钟区固定在最右；托盘组（^ + 3 个图标）整组贴在其左侧，
                // 与时钟之间只留 10 dip 和一条 1 px 分界线，界线上不放任何内容。
                self.preview_clock = Rect::new(bar.right() - 12.0 - 62.0, bar.y + 8.0, 62.0, 28.0);
                let chev_w = 16.0;
                let glyph_w = 12.0;
                let glyph_gap = 8.0;
                // 末尾 6 dip 是图标与分界线之间的留白，避免最后一个图标贴线。
                let tray_w = chev_w + glyph_gap + 3.0 * glyph_w + 2.0 * glyph_gap + 6.0;
                let tray_x = self.preview_clock.x - 10.0 - tray_w;
                // 与 12 px 托盘图标光学对齐（中心同高），避免箭头"浮"在图标上方。
                self.tray_chevron = Rect::new(tray_x, bar.y + 17.0, chev_w, 10.0);
                for (i, g) in self.tray_glyphs.iter_mut().enumerate() {
                    *g = Rect::new(
                        tray_x + chev_w + glyph_gap + i as f32 * (glyph_w + glyph_gap),
                        bar.y + 16.0,
                        glyph_w,
                        glyph_w,
                    );
                }

                let chip_w = 48.0;
                let chip_gap = 6.0;
                let chips_w = chip_w * 3.0 + chip_gap * 2.0;
                let chips_x = self.preview.right() - 10.0 - chips_w;
                for i in 0..3 {
                    self.mode_rects[i] = Rect::new(
                        chips_x + i as f32 * (chip_w + chip_gap),
                        top + 10.0,
                        chip_w,
                        22.0,
                    );
                }

                let zw = (cw - 2.0 * GAP) / 3.0;
                let zy = self.preview.bottom() + GAP;
                for i in 0..3 {
                    self.zone_rects[i] = Rect::new(PAD + i as f32 * (zw + GAP), zy, zw, 44.0);
                }

                self.colors_card = Rect::new(PAD, self.zone_rects[0].bottom() + GAP, cw, 214.0);
                let inner = self.colors_card.inset(PAD);
                // 4 项内容 × 2 背景态矩阵：左侧 52 dip 留给行标题。
                let label_col = 52.0;
                let col_gap = 8.0;
                let cw4 = ((inner.w - label_col - 3.0 * col_gap) / 4.0).max(40.0);
                for (r, row) in self.item_wells.iter_mut().enumerate() {
                    for (c, cell) in row.iter_mut().enumerate() {
                        *cell = Rect::new(
                            inner.x + label_col + c as f32 * (cw4 + col_gap),
                            inner.y + 42.0 + r as f32 * 23.0,
                            cw4,
                            18.0,
                        );
                    }
                }
                // 与阈值滑条同款高度（10 dip），视觉风格才一致。
                self.opacity_track = Rect::new(inner.x, inner.y + 114.0, inner.w, 10.0);
                let seg_w = 76.0;
                self.judge_rects[0] = Rect::new(inner.right() - seg_w * 2.0 - 2.0, inner.y, seg_w, 26.0);
                self.judge_rects[1] = Rect::new(inner.right() - seg_w, inner.y, seg_w, 26.0);
                self.thr_track = Rect::new(inner.x, inner.y + 152.0, inner.w, 10.0);
            }
            Page::Sample => {
                self.sample_card = Rect::new(PAD, top, cw, 160.0);
                self.sample_band = Rect::new(PAD + PAD, top + 46.0, cw - PAD * 2.0, 60.0);
                // 40 px 高的迷你任务栏在 60 px 的带内垂直居中（上下各 10）。
                self.sample_bar = Rect::new(
                    self.sample_band.x,
                    self.sample_band.y + (self.sample_band.h - 40.0) / 2.0,
                    self.sample_band.w,
                    40.0,
                );
            }
            Page::Diag => {
                self.diag_card = Rect::new(PAD, top, cw, 196.0);
                self.diag_spark = Rect::new(PAD + PAD, top + 118.0, cw - PAD * 2.0, 46.0);
            }
            Page::Tray => {}
        }

        let (card, first) = self.rows_geometry();
        self.rows_card = card;
        for (i, row) in self.rows.iter_mut().enumerate() {
            row.rect = Rect::new(card.x, card.y + first + i as f32 * ROW_H, card.w, ROW_H);
        }
    }

    // -----------------------------------------------------------------
    // 命中
    // -----------------------------------------------------------------

    fn hit(&self, x: f32, y: f32) -> Hit {
        for (i, r) in self.foot_rects.iter().enumerate() {
            if r.contains(x, y) {
                return Hit::Footer(i);
            }
        }
        for (i, r) in self.tab_rects.iter().enumerate() {
            if r.contains(x, y) {
                return Hit::Tab(i);
            }
        }
        if self.tint_toggle.contains(x, y) {
            return Hit::Tint;
        }
        if self.pause_btn.contains(x, y) {
            return Hit::Pause;
        }
        match self.page {
            Page::Taskbar => {
                for (i, r) in self.mode_rects.iter().enumerate() {
                    if r.contains(x, y) {
                        return Hit::Mode(i);
                    }
                }
                for (i, r) in self.zone_rects.iter().enumerate() {
                    if r.contains(x, y) {
                        return Hit::Zone(i);
                    }
                }
                for (r, row) in self.item_wells.iter().enumerate() {
                    for (c, rect) in row.iter().enumerate() {
                        if rect.contains(x, y) {
                            return Hit::ItemWell(r, c);
                        }
                    }
                }
                // 不透明度滑条两种判定模式都会显示，命中检测必须在模式分支之外。
                let op_hit = Rect::new(
                    self.opacity_track.x,
                    self.opacity_track.y - 8.0,
                    self.opacity_track.w,
                    self.opacity_track.h + 16.0,
                );
                if op_hit.contains(x, y) {
                    return Hit::Opacity;
                }
                for (i, r) in self.judge_rects.iter().enumerate() {
                    if r.contains(x, y) {
                        return Hit::Judge(i);
                    }
                }
                let thr_hit = Rect::new(
                    self.thr_track.x,
                    self.thr_track.y - 8.0,
                    self.thr_track.w,
                    self.thr_track.h + 16.0,
                );
                if thr_hit.contains(x, y) {
                    return Hit::Threshold;
                }
            }
            Page::Sample | Page::Tray | Page::Diag => {
                for (i, row) in self.rows.iter().enumerate() {
                    if row.rect.contains(x, y) {
                        return Hit::Row(i);
                    }
                }
            }
        }
        Hit::None
    }

    fn hit_is_interactive(hit: Hit) -> bool {
        !matches!(hit, Hit::None)
    }

    // -----------------------------------------------------------------
    // 输入
    // -----------------------------------------------------------------

    /// 鼠标移动时的光标形状（`window.rs` 在无浮层时调用）。
    pub fn hit_test_and_update_cursor(&mut self, x_px: i32, y_px: i32) -> bool {
        let (x, y) = self.px_to_dip(x_px, y_px);
        if self.popup.is_some() {
            self.set_cursor(true);
            return true;
        }
        let hit = self.hit(x, y);
        self.hover = hit;
        self.set_cursor(Self::hit_is_interactive(hit));
        Self::hit_is_interactive(hit)
    }

    fn set_cursor(&self, interactive: bool) {
        let name = if interactive { IDC_HAND } else { IDC_ARROW };
        // SAFETY: 无前置条件；加载失败时保持系统默认光标。
        if let Ok(cursor) = unsafe { LoadCursorW(None, name) } {
            // SAFETY: cursor 有效。
            unsafe { SetCursor(cursor) };
        }
    }

    fn px_to_dip(&self, x_px: i32, y_px: i32) -> (f32, f32) {
        let s = self.dpi as f32 / 96.0;
        (x_px as f32 / s, y_px as f32 / s)
    }

    /// 处理窗口消息。`Some` 表示已消费。
    ///
    /// # Errors
    ///
    /// 当前实现不产生错误；保留 `SbResult` 与窗口层签名一致。
    pub fn on_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> SbResult<Option<LRESULT>> {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let (dx, dy) = self.px_to_dip(x, y);

        match msg {
            WM_MOUSEMOVE => {
                if let Some(key) = self.dragging {
                    self.drag_slider(key, dx);
                } else {
                    self.hover = if self.popup.is_some() {
                        Hit::None
                    } else {
                        self.hit(dx, dy)
                    };
                }
                Ok(Some(LRESULT(0)))
            }
            WM_LBUTTONDOWN => {
                self.focus_visible = false;
                if self.popup.is_some() {
                    self.handle_popup_click(dx, dy);
                    Ok(Some(LRESULT(0)))
                } else {
                    self.handle_click(hwnd, dx, dy);
                    Ok(Some(LRESULT(0)))
                }
            }
            WM_LBUTTONUP => {
                if self.dragging.take().is_some() {
                    // SAFETY: 与 SetCapture 配对。
                    unsafe { ReleaseCapture().ok() };
                }
                Ok(Some(LRESULT(0)))
            }
            WM_KEYDOWN => {
                self.handle_key(wp.0 as u16);
                Ok(Some(LRESULT(0)))
            }
            WM_KILLFOCUS => {
                self.dragging = None;
                Ok(Some(LRESULT(0)))
            }
            _ => Ok(None),
        }
    }

    fn handle_click(&mut self, hwnd: HWND, x: f32, y: f32) {
        match self.hit(x, y) {
            Hit::Tab(i) => {
                let page = Page::ALL[i];
                if page != self.page {
                    self.page = page;
                    self.focus = usize::MAX;
                    self.rebuild_rows();
                    self.rebuild_layout();
                }
            }
            Hit::Tint => {
                let v = i64::from(self.cfg.get(SettingKey::S01) == 0);
                self.push_set(SettingKey::S01, v);
            }
            Hit::Pause => {
                let v = i64::from(self.cfg.get(SettingKey::S42) == 0);
                self.push_set(SettingKey::S42, v);
            }
            Hit::Mode(i) => self.push_set(SettingKey::S10, i as i64),
            Hit::Zone(i) => {
                // 任务栏从左到右：图标 → 托盘 → 时钟。
                let key = [
                    SettingKey::S50,
                    SettingKey::S49,
                    SettingKey::S48,
                ][i];
                let v = i64::from(self.cfg.get(key) == 0);
                self.push_set(key, v);
            }
            Hit::ItemWell(r, c) => {
                let key = ITEM_KEYS[r.min(1)][c.min(3)];
                self.open_popup(PopupKind::ColorPicker(key));
            }
            Hit::Opacity => {
                self.set_opacity_from_x(x);
                self.dragging = Some(SettingKey::S60);
                // SAFETY: hwnd 有效；捕获后拖出窗口也能收到抬起消息。
                unsafe { SetCapture(hwnd) };
            }
            Hit::Judge(i) => self.push_set(SettingKey::S47, i as i64),
            Hit::Threshold => {
                self.set_threshold(x);
                self.dragging = Some(SettingKey::S12);
                // SAFETY: hwnd 有效；捕获后拖出窗口也能收到抬起消息。
                unsafe { SetCapture(hwnd) };
            }
            Hit::Row(i) => self.press_row(hwnd, i, x),
            Hit::Footer(i) => match i {
                0 => self.open_popup(PopupKind::ConfirmRestore),
                1 => self.actions.push(ViewAction::ImportConfig),
                2 => self.actions.push(ViewAction::ExportConfig),
                _ => self.open_popup(PopupKind::About),
            },
            Hit::None => {}
        }
    }

    fn press_row(&mut self, hwnd: HWND, index: usize, x: f32) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let key = row.key;
        let kind = key.meta().kind;
        let enabled = row.enabled;
        if !enabled {
            return;
        }
        match kind {
            ControlKind::Toggle => {
                let v = i64::from(self.cfg.get(key) == 0);
                self.push_set(key, v);
            }
            ControlKind::Dropdown => {
                self.focus = index;
                self.open_popup(PopupKind::Dropdown(key));
            }
            ControlKind::Slider => {
                self.focus = index;
                self.set_slider_from_x(key, x);
                self.dragging = Some(key);
                // SAFETY: hwnd 有效。
                unsafe { SetCapture(hwnd) };
            }
            ControlKind::ReadOnly => {
                self.focus = index;
                if key == SettingKey::S05 {
                    self.actions.push(ViewAction::CopyVersion);
                    self.show_toast("common.copy_ok");
                }
            }
            ControlKind::Button => {
                self.focus = index;
                if key == SettingKey::S46 {
                    self.actions.push(ViewAction::ExportDiagnostics);
                }
            }
            ControlKind::Color => {}
        }
    }

    fn drag_slider(&mut self, key: SettingKey, x: f32) {
        if key == SettingKey::S12 {
            self.set_threshold(x);
            return;
        }
        if key == SettingKey::S60 {
            self.set_opacity_from_x(x);
            return;
        }
        self.set_slider_from_x(key, x);
    }

    fn set_opacity_from_x(&mut self, x: f32) {
        let r = self.opacity_track;
        if r.w <= 0.0 {
            return;
        }
        let ratio = ((x - r.x) / r.w).clamp(0.0, 1.0);
        self.push_set(SettingKey::S60, (ratio * 100.0).round() as i64);
    }

    fn set_threshold(&mut self, x: f32) {
        let r = self.thr_track;
        if r.w <= 0.0 {
            return;
        }
        let ratio = ((x - r.x) / r.w).clamp(0.0, 1.0);
        let raw = 300.0 + ratio * 400.0;
        self.push_set(SettingKey::S12, (raw / 10.0).round() as i64 * 10);
    }

    fn set_slider_from_x(&mut self, key: SettingKey, x: f32) {
        let Some(row) = self.rows.iter().find(|r| r.key == key) else {
            return;
        };
        let track = slider_track(row.rect);
        let meta = key.meta();
        let ratio = ((x - track.x) / track.w.max(1.0)).clamp(0.0, 1.0);
        let raw = meta.min as f32 + ratio * (meta.max - meta.min) as f32;
        let step = meta.step.max(1) as f32;
        let v = ((raw - meta.min as f32) / step).round() * step + meta.min as f32;
        self.push_set(key, v as i64);
    }

    /// 取色板分组：偏暗行用浅色组 0，偏亮行用深色组 1。
    fn picker_group(key: SettingKey) -> usize {
        match key {
            SettingKey::S15
            | SettingKey::S53
            | SettingKey::S55
            | SettingKey::S57
            | SettingKey::S59 => 1,
            _ => 0,
        }
    }

    /// 颜色键对应的色块矩形（浮层定位用）。
    fn color_anchor(&self, key: SettingKey) -> Rect {
        for (r, row) in ITEM_KEYS.iter().enumerate() {
            for (c, k) in row.iter().enumerate() {
                if *k == key {
                    return self.item_wells[r][c];
                }
            }
        }
        self.item_wells[0][0]
    }

    fn push_set(&mut self, key: SettingKey, value: i64) {
        if self.cfg.get(key) == value {
            return;
        }
        self.actions.push(ViewAction::Set(key, value));
    }

    fn handle_key(&mut self, vk: u16) {
        if self.popup.is_some() {
            match vk {
                VK_ESCAPE => self.popup = None,
                VK_DOWN => self.move_popup(1),
                VK_UP => self.move_popup(-1),
                VK_RETURN => self.apply_popup_selection(),
                _ => {}
            }
            return;
        }
        match vk {
            VK_TAB | VK_DOWN => self.move_focus(1),
            VK_UP => self.move_focus(-1),
            VK_LEFT | VK_RIGHT => self.nudge_focused(if vk == VK_LEFT { -1 } else { 1 }),
            VK_RETURN | VK_SPACE => {
                if self.focus != usize::MAX {
                    let x = self.rows[self.focus].rect.x + 40.0;
                    self.press_row(HWND::default(), self.focus, x);
                }
            }
            _ => {}
        }
    }

    fn move_focus(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        self.focus_visible = true;
        let n = self.rows.len() as i32;
        let cur = if self.focus == usize::MAX { -1 } else { self.focus as i32 };
        let next = (cur + delta).rem_euclid(n);
        self.focus = next as usize;
    }

    fn nudge_focused(&mut self, dir: i32) {
        if self.focus == usize::MAX {
            return;
        }
        let Some(row) = self.rows.get(self.focus) else {
            return;
        };
        let key = row.key;
        let meta = key.meta();
        if meta.kind != ControlKind::Slider {
            return;
        }
        let v = self.cfg.get(key) + i64::from(dir) * meta.step.max(1);
        self.push_set(key, v);
    }

    fn move_popup(&mut self, delta: i32) {
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        let PopupKind::Dropdown(key) = popup.kind else {
            return;
        };
        let n = key.meta().options.len() as i32;
        if n == 0 {
            return;
        }
        popup.selected = (popup.selected as i32 + delta).rem_euclid(n) as usize;
    }

    fn apply_popup_selection(&mut self) {
        let Some(popup) = self.popup.take() else {
            return;
        };
        if let PopupKind::Dropdown(key) = popup.kind {
            self.push_set(key, popup.selected as i64);
        }
    }

    // -----------------------------------------------------------------
    // 浮层
    // -----------------------------------------------------------------

    fn open_popup(&mut self, kind: PopupKind) {
        let (w, h) = self.client;
        let frame = match kind {
            PopupKind::Dropdown(key) => {
                let Some(row) = self.rows.iter().find(|r| r.key == key) else {
                    return;
                };
                let n = key.meta().options.len() as i64;
                let fw = 180.0;
                let fh = n as f32 * 28.0 + 8.0;
                let mut y = row.rect.bottom() + 2.0;
                if y + fh > h - FOOTER_H {
                    y = row.rect.y - fh - 2.0;
                }
                Rect::new(row.rect.right() - 16.0 - fw, y, fw, fh)
            }
            PopupKind::ColorPicker(key) => {
                let anchor = self.color_anchor(key);
                let fw = 256.0;
                let fh = 112.0;
                // 最右一列（时间与日期）的锚点靠右，直接左对齐会顶出窗口，
                // 右半边色块既看不见也点不到（问题 3）。这里把浮层夹在窗口内。
                let x = anchor.x.min(w - 8.0 - fw).max(8.0);
                let mut y = anchor.bottom() + 6.0;
                if y + fh > h - 8.0 {
                    y = (h - 8.0 - fh).max(8.0);
                }
                Rect::new(x, y, fw, fh)
            }
            PopupKind::ConfirmRestore => Rect::new((w - 380.0) / 2.0, (h - 132.0) / 2.0, 380.0, 132.0),
            PopupKind::About => Rect::new((w - 420.0) / 2.0, (h - 200.0) / 2.0, 420.0, 200.0),
        };
        let selected = match kind {
            PopupKind::Dropdown(key) => self.cfg.get(key).clamp(0, i64::MAX) as usize,
            _ => 0,
        };
        self.popup = Some(PopupState {
            kind,
            frame,
            selected,
        });
    }

    fn handle_popup_click(&mut self, x: f32, y: f32) {
        let Some(popup) = self.popup.as_ref() else {
            return;
        };
        let frame = popup.frame;
        let kind = popup.kind;
        if !frame.contains(x, y) {
            self.popup = None;
            return;
        }
        match kind {
            PopupKind::Dropdown(key) => {
                let n = key.meta().options.len();
                let item_h = 28.0;
                for i in 0..n {
                    let r = Rect::new(frame.x + 4.0, frame.y + 4.0 + i as f32 * item_h, frame.w - 8.0, item_h);
                    if r.contains(x, y) {
                        self.popup = None;
                        self.push_set(key, i as i64);
                        return;
                    }
                }
            }
            PopupKind::ColorPicker(key) => {
                let group = Self::picker_group(key);
                let colors = i18n::COLOR_SWATCH_GROUPS[group].1;
                let sw = 32.0;
                let gap = 6.0;
                let y0 = frame.y + 32.0;
                for (i, hex) in colors.iter().enumerate() {
                    let r = Rect::new(frame.x + 12.0 + i as f32 * (sw + gap), y0, sw, 26.0);
                    if r.contains(x, y) {
                        self.popup = None;
                        let argb = parse_hex(hex) | 0xFF00_0000;
                        self.push_set(key, argb);
                        return;
                    }
                }
                let more = Rect::new(frame.x + 12.0, frame.y + 70.0, frame.w - 24.0, 30.0);
                if more.contains(x, y) {
                    self.popup = None;
                    self.actions.push(ViewAction::PickColor(key));
                }
            }
            PopupKind::ConfirmRestore => {
                let by = frame.bottom() - 46.0;
                let cancel = Rect::new(frame.right() - 16.0 - 88.0, by, 88.0, 30.0);
                let confirm = Rect::new(cancel.x - 8.0 - 96.0, by, 96.0, 30.0);
                if confirm.contains(x, y) {
                    self.popup = None;
                    self.actions.push(ViewAction::RestoreDefaults);
                } else if cancel.contains(x, y) {
                    self.popup = None;
                }
            }
            PopupKind::About => {
                let btn = Rect::new(frame.x + 16.0, frame.bottom() - 44.0, 140.0, 28.0);
                if btn.contains(x, y) {
                    self.actions.push(ViewAction::CopyVersion);
                    self.show_toast("common.copy_ok");
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // 绘制
    // -----------------------------------------------------------------

    /// 画一帧。
    pub fn paint(&self, ctx: &RenderCtx, p: &Palette, _mode: ThemeMode) {
        let w = self.client.0.max(theme::MIN_WIDTH_DIP);
        let h = self.client.1.max(theme::MIN_HEIGHT_DIP);
        ctx.fill_rect(rd(Rect::new(0.0, 0.0, w, h)), p.window_bg);

        self.paint_header(ctx, p, w);
        match self.page {
            Page::Taskbar => self.paint_taskbar_page(ctx, p),
            Page::Sample => self.paint_sample_page(ctx, p),
            Page::Tray => self.paint_tray_page(ctx, p),
            Page::Diag => self.paint_diag_page(ctx, p),
        }
        self.paint_footer(ctx, p, w, h);

        if let Some((msg, at)) = &self.toast {
            if at.elapsed().as_secs_f32() < 3.0 {
                self.paint_toast(ctx, p, msg, w, h);
            }
        }
        if self.popup.is_some() {
            self.paint_popup(ctx, p);
        }
    }

    fn paint_header(&self, ctx: &RenderCtx, p: &Palette, w: f32) {
        ctx.fill_rect(rd(Rect::new(0.0, HEADER_H - 1.0, w, 1.0)), p.card_border);

        for (i, page) in Page::ALL.iter().enumerate() {
            let r = self.tab_rects[i];
            let active = *page == self.page;
            let role = if active { TextRole::Subtitle } else { TextRole::Body };
            let color = if active { p.text_primary } else { p.text_secondary };
            self.label_center(ctx, text(page.title_key(), self.lang), Rect::new(r.x, r.y + 15.0, r.w, 18.0), role, color);
            if active {
                ctx.fill_rect(rd(Rect::new(r.x + 14.0, HEADER_H - 2.0, r.w - 28.0, 2.0)), p.accent);
            }
        }

        // 暂停 / 继续
        let paused = self.paused();
        let label = text(if paused { "ui.resume" } else { "ui.pause" }, self.lang);
        let hovered = self.hover == Hit::Pause;
        let bg = if paused {
            p.accent
        } else if hovered {
            p.card_alt
        } else {
            p.card_bg
        };
        let fg = if paused { p.accent_text } else { p.text_primary };
        ctx.fill_rounded_rect(rd(self.pause_btn), 6.0, bg);
        ctx.stroke_rounded_rect(rd(self.pause_btn), 6.0, if paused { p.accent } else { p.card_border }, 1.0);
        self.label_center(
            ctx,
            label,
            Rect::new(self.pause_btn.x, self.pause_btn.y + 7.0, self.pause_btn.w, 16.0),
            TextRole::Body,
            fg,
        );

        // 着色开关
        let on = self.tint_on();
        ctx.fill_rounded_rect(rd(self.tint_toggle), TOGGLE_H / 2.0, if on { p.accent } else { p.track_off });
        let knob_x = if on {
            self.tint_toggle.right() - TOGGLE_H + 2.0
        } else {
            self.tint_toggle.x + 2.0
        };
        ctx.fill_rounded_rect(
            rd(Rect::new(knob_x, self.tint_toggle.y + 2.0, TOGGLE_H - 4.0, TOGGLE_H - 4.0)),
            (TOGGLE_H - 4.0) / 2.0,
            p.card_bg,
        );
        let tint_label = text("ui.tint", self.lang);
        let tw = text_width(tint_label, 11.0);
        self.label(
            ctx,
            tint_label,
            Rect::new(self.tint_toggle.x - 8.0 - tw, 16.0, tw, 16.0),
            TextRole::Caption,
            p.text_secondary,
        );

        // Y 读数 + 状态 chip
        let y_text = self.y_text();
        let yw = text_width(&y_text, 11.0);
        let chip = self.run_state();
        let (chip_label, chip_color) = match chip {
            RunState::Running => (text("ui.running", self.lang), p.ok),
            RunState::Paused => (text("ui.paused", self.lang), p.text_secondary),
            RunState::Limited => (text("ui.restricted", self.lang), p.warning),
        };
        let chip_w = text_width(chip_label, 11.0) + 22.0;
        let chip_x = self.tint_toggle.x - 8.0 - tw - 14.0 - chip_w;
        ctx.fill_rounded_rect(rd(Rect::new(chip_x, 14.0, chip_w, 20.0)), 10.0, p.card_alt);
        ctx.fill_rounded_rect(rd(Rect::new(chip_x + 8.0, 21.5, 5.0, 5.0)), 2.5, chip_color);
        self.label(
            ctx,
            chip_label,
            Rect::new(chip_x + 17.0, 17.0, chip_w - 20.0, 15.0),
            TextRole::Caption,
            chip_color,
        );
        self.label(
            ctx,
            &y_text,
            Rect::new(chip_x - 14.0 - yw, 17.0, yw, 15.0),
            TextRole::Caption,
            p.text_secondary,
        );
    }

    fn y_text(&self) -> String {
        if !self.diag.has_data {
            return text("diag.no_data", self.lang).to_string();
        }
        let thr = self.cfg.get(SettingKey::S12) as f32 / 1000.0;
        let y = self.diag.y_milli as f32 / 1000.0;
        let side = match self.cfg.get(SettingKey::S10) {
            1 => text("preview.light", self.lang),
            2 => text("preview.dark", self.lang),
            _ => {
                // 亮度低于阈值 = 背景偏暗 = 用亮端色（浅色文字），标签必须跟着文字端走
                if y < thr {
                    text("preview.light", self.lang)
                } else {
                    text("preview.dark", self.lang)
                }
            }
        };
        format!("Y {y:.3} · {side}")
    }

    fn paint_taskbar_page(&self, ctx: &RenderCtx, p: &Palette) {
        let enabled = self.tint_on() && !self.degraded;

        // 预览
        let (bar_bg, bar_ink, bar_line) = self.bar_colors(p);
        ctx.fill_rounded_rect(rd(self.preview), RADIUS, p.preview_wall);
        let bar = self.preview_bar;
        ctx.fill_rounded_rect(rd(bar), 7.0, bar_bg);
        ctx.stroke_rounded_rect(rd(bar), 7.0, bar_line, 1.0);

        // 背景模式下拉（自动 / 亮端 / 暗端）
        let mode = self.cfg.get(SettingKey::S10);
        for (i, label_key) in ["preview.auto", "preview.light", "preview.dark"].iter().enumerate() {
            let r = self.mode_rects[i];
            let active = mode == i as i64;
            let bg = if active {
                p.preview_ink
            } else if enabled {
                p.preview_line
            } else {
                p.preview_bar
            };
            let fg = if active { p.preview_bar } else { p.preview_muted };
            ctx.fill_rounded_rect(rd(r), 6.0, bg);
            self.label_center(
                ctx,
                text(label_key, self.lang),
                Rect::new(r.x, r.y + 4.0, r.w, 14.0),
                TextRole::Caption,
                fg,
            );
        }

        // 开始按钮（2×2 方格）
        let sx = bar.x + 14.0;
        let sy = bar.y + 14.0;
        for row in 0..2 {
            for col in 0..2 {
                ctx.fill_rounded_rect(
                    rd(Rect::new(
                        sx + col as f32 * 9.0,
                        sy + row as f32 * 9.0,
                        7.0,
                        7.0,
                    )),
                    1.0,
                    bar_ink,
                );
            }
        }

        // 应用图标 + 图标区指示条
        let icon_tint = self.zone_color(SettingKey::S50, p);
        let icon_x = sx + 30.0;
        for i in 0..4 {
            let r = Rect::new(icon_x + i as f32 * 30.0, bar.y + 11.0, 22.0, 22.0);
            let fill = if i % 2 == 0 {
                Rgba::opaque(0x4A, 0x55, 0x63)
            } else {
                Rgba::opaque(0x2B, 0x32, 0x3D)
            };
            ctx.fill_rounded_rect(rd(r), 5.0, fill);
            ctx.stroke_rounded_rect(rd(r), 5.0, bar_line, 1.0);
            ctx.fill_rounded_rect(
                rd(Rect::new(r.x + 7.0, r.y + 7.0, 8.0, 8.0)),
                2.0,
                icon_tint,
            );
        }
        ctx.fill_rounded_rect(
            rd(Rect::new(icon_x + 6.0, bar.bottom() - 5.0, 10.0, 2.0)),
            1.0,
            icon_tint,
        );

        // 托盘（^ 在最左）+ 时钟（最右）
        let chev = self.tray_chevron;
        // 两条抗锯齿实线画 ^：方块拼斜边在任何非 100% DPI 下都会出现锯齿和毛边，
        // 线段交给 D2D 做几何抗锯齿，斜边连续、尖顶不用平顶色块凑。
        let cx = chev.x + chev.w / 2.0;
        let top = chev.y + 1.0;
        let bottom = chev.bottom() - 1.0;
        // 两段在顶点交叉 0.5 dip，平头线帽不会在尖端留缺口。
        ctx.draw_line(chev.x + 2.0, bottom, cx + 0.5, top - 0.5, 2.5, bar_ink);
        ctx.draw_line(cx - 0.5, top - 0.5, chev.right() - 2.0, bottom, 2.5, bar_ink);
        let tray_tint = self.zone_color(SettingKey::S49, p);
        for (i, g) in self.tray_glyphs.iter().enumerate() {
            match i {
                0 => ctx.stroke_rounded_rect(rd(*g), 2.0, tray_tint, 1.0),
                1 => {
                    ctx.fill_rect(rd(Rect::new(g.x + 1.0, g.y + 7.0, 3.0, 4.0)), tray_tint);
                    ctx.fill_rect(rd(Rect::new(g.x + 5.0, g.y + 4.0, 3.0, 7.0)), tray_tint);
                    ctx.fill_rect(rd(Rect::new(g.x + 9.0, g.y + 1.0, 3.0, 10.0)), tray_tint);
                }
                _ => ctx.fill_rounded_rect(rd(Rect::new(g.x + 1.0, g.y + 1.0, 10.0, 10.0)), 5.0, tray_tint),
            }
        }
        let clock = self.preview_clock;
        ctx.fill_rect(
            rd(Rect::new(clock.x - 10.0, bar.y + 9.0, 1.0, bar.h - 18.0)),
            bar_line,
        );

        let (time, date) = local_time();
        let clock_tint = self.zone_color(SettingKey::S48, p);
        ctx.draw_text_right(
            &time,
            rd(Rect::new(clock.x, clock.y, clock.w, 16.0)),
            TextRole::Body,
            clock_tint,
        );
        ctx.draw_text_right(
            &date,
            rd(Rect::new(clock.x, clock.y + 16.0, clock.w, 14.0)),
            TextRole::Caption,
            clock_tint,
        );

        // 区域标签（贴着预览，不额外解释）
        let ly = bar.y - 18.0;
        self.label(ctx, text("zone.icons", self.lang), Rect::new(bar.x + 8.0, ly, 80.0, 14.0), TextRole::Caption, p.preview_muted);
        // 托盘区标签与 ^ 左边缘对齐，让"托盘区从箭头开始"一眼可见。
        self.label(
            ctx,
            text("zone.tray", self.lang),
            Rect::new(self.tray_chevron.x, ly, 120.0, 14.0),
            TextRole::Caption,
            p.preview_muted,
        );
        let clock_label = text("zone.clock", self.lang);
        let cl_w = text_width(clock_label, 11.0);
        self.label(ctx, clock_label, Rect::new(bar.right() - 8.0 - cl_w, ly, cl_w, 14.0), TextRole::Caption, p.preview_muted);

        // 三区卡片：图标 → 托盘 → 时钟
        let zones = [
            (SettingKey::S50, "zone.icons"),
            (SettingKey::S49, "zone.tray"),
            (SettingKey::S48, "zone.clock"),
        ];
        for (i, (key, label_key)) in zones.iter().enumerate() {
            let r = self.zone_rects[i];
            let on = self.cfg.get(*key) != 0;
            ctx.fill_rounded_rect(rd(r), 7.0, p.card_bg);
            ctx.stroke_rounded_rect(rd(r), 7.0, if self.hover == Hit::Zone(i) { p.accent } else { p.card_border }, 1.0);
            let dot_bg = p.preview_bar;
            let dot = self.zone_color(*key, p);
            ctx.fill_rounded_rect(rd(Rect::new(r.x + 12.0, r.y + 14.0, 22.0, 16.0)), 4.0, dot_bg);
            ctx.fill_rounded_rect(rd(Rect::new(r.x + 19.0, r.y + 19.0, 8.0, 6.0)), 2.0, dot);
            self.label(
                ctx,
                text(label_key, self.lang),
                Rect::new(r.x + 42.0, r.y + 13.0, r.w - 42.0 - TOGGLE_W - 24.0, 18.0),
                TextRole::Body,
                p.control_text(enabled),
            );
            let tr = Rect::new(r.right() - 12.0 - TOGGLE_W, r.y + (r.h - TOGGLE_H) / 2.0, TOGGLE_W, TOGGLE_H);
            self.paint_toggle(ctx, p, tr, on, enabled);
        }

        self.paint_colors_card(ctx, p, enabled);
    }

    fn paint_colors_card(&self, ctx: &RenderCtx, p: &Palette, enabled: bool) {
        let card = self.colors_card;
        if card.w <= 0.0 {
            return;
        }
        ctx.fill_rounded_rect(rd(card), RADIUS, p.card_bg);
        ctx.stroke_rounded_rect(rd(card), RADIUS, p.card_border, 1.0);
        let inner = card.inset(PAD);

        self.label(
            ctx,
            text("card.colors", self.lang),
            Rect::new(inner.x, inner.y + 3.0, 200.0, 16.0),
            TextRole::Caption,
            p.text_secondary,
        );

        // 判定粒度
        let judge = self.cfg.get(SettingKey::S47);
        let seg_total = self.judge_rects[1].right() - self.judge_rects[0].x;
        ctx.fill_rounded_rect(rd(Rect::new(self.judge_rects[0].x, inner.y, seg_total, 26.0)), 6.0, p.card_alt);
        for (i, key) in ["opt.judge.region", "opt.judge.global"].iter().enumerate() {
            let r = self.judge_rects[i];
            let active = judge == i as i64;
            if active {
                ctx.fill_rounded_rect(rd(r), 5.0, p.accent);
            }
            self.label_center(
                ctx,
                text(key, self.lang),
                Rect::new(r.x, r.y + 5.0, r.w, 16.0),
                TextRole::Caption,
                if active { p.accent_text } else { p.text_secondary },
            );
        }

        // 2 行 × 4 列的色块矩阵：两种判定粒度都显示、都生效（问题 2）。
        {
            for (c, label_key) in ITEM_LABELS.iter().enumerate() {
                let cell = self.item_wells[0][c];
                self.label_center(
                    ctx,
                    text(label_key, self.lang),
                    Rect::new(cell.x, cell.y - 15.0, cell.w, 12.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
            }
            for (r, row_label) in ["color.on_dark", "color.on_light"].iter().enumerate() {
                let cell = self.item_wells[r][0];
                self.label(
                    ctx,
                    text(row_label, self.lang),
                    Rect::new(inner.x, cell.y + 5.0, 46.0, 14.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
            }
            for (r, row) in self.item_wells.iter().enumerate() {
                for (c, box_r) in row.iter().enumerate() {
                    let box_r = *box_r;
                    ctx.fill_rounded_rect(rd(box_r), 5.0, p.card_alt);
                    ctx.stroke_rounded_rect(
                        rd(box_r),
                        5.0,
                        if self.hover == Hit::ItemWell(r, c) {
                            p.accent
                        } else {
                            p.divider
                        },
                        1.0,
                    );
                    let argb = self.cfg.get(ITEM_KEYS[r][c]);
                    let chip = Rect::new(box_r.x + 5.0, box_r.y + 2.0, 16.0, 14.0);
                    ctx.fill_rounded_rect(rd(chip), 3.0, argb_to_rgba(argb));
                    ctx.stroke_rounded_rect(rd(chip), 3.0, p.card_border, 1.0);
                    self.label(
                        ctx,
                        &hex_rgb(argb),
                        Rect::new(chip.right() + 6.0, box_r.y + 2.0, box_r.w - 32.0, 14.0),
                        TextRole::Caption,
                        p.control_text(enabled),
                    );
                }
            }
        }

        // 正常态指示条不透明度（S-60，两种判定模式都生效）。
        // 外观与下面的亮度阈值滑条保持一致：10 dip 圆角轨 + 12×14 药丸滑块。
        let opacity = self.cfg.get(SettingKey::S60);
        let ot = self.opacity_track;
        let label_y = ot.y - 18.0;
        self.label(
            ctx,
            text("s60", self.lang),
            Rect::new(inner.x, label_y, 280.0, 14.0),
            TextRole::Caption,
            p.control_text(enabled),
        );
        self.label_right(
            ctx,
            &format!("{opacity}%"),
            Rect::new(inner.right() - 50.0, label_y, 50.0, 14.0),
            TextRole::Caption,
            p.control_text(enabled),
        );
        ctx.fill_rounded_rect(rd(ot), 5.0, p.card_alt);
        let filled = Rect::new(ot.x, ot.y, ot.w * opacity as f32 / 100.0, ot.h);
        ctx.fill_rounded_rect(rd(filled), 5.0, p.accent);
        ctx.stroke_rounded_rect(rd(ot), 5.0, p.card_border, 1.0);
        let kx = ot.x + ot.w * opacity as f32 / 100.0 - 6.0;
        ctx.fill_rounded_rect(rd(Rect::new(kx, ot.y - 2.0, 12.0, 14.0)), 7.0, p.card_bg);
        ctx.stroke_rounded_rect(rd(Rect::new(kx, ot.y - 2.0, 12.0, 14.0)), 7.0, p.accent, 2.0);

        // 阈值
        let thr = self.cfg.get(SettingKey::S12);
        let track = self.thr_track;
        let label_y = track.y - 20.0;
        self.label(
            ctx,
            text("thr.title", self.lang),
            Rect::new(inner.x, label_y, 200.0, 16.0),
            TextRole::Caption,
            p.control_text(enabled),
        );
        self.label_right(
            ctx,
            &format!("{:.2}", thr as f32 / 1000.0),
            Rect::new(inner.right() - 60.0, label_y, 60.0, 16.0),
            TextRole::Caption,
            p.control_text(enabled),
        );

        let seg_r = 5.0_f32;
        let (cap_l, mid, cap_r) = gradient_spans(track, seg_r);
        let segments = 32u32;
        for i in 0..segments {
            let t = (i as f32 + 0.5) / segments as f32;
            let v = (t * 255.0).round() as u8;
            let seg = Rect::new(
                mid.x + mid.w * i as f32 / segments as f32,
                mid.y,
                mid.w / segments as f32 + 0.5,
                mid.h,
            );
            ctx.fill_rect(rd(seg), Rgba::opaque(v, v, v));
        }
        // 两端用圆帽收口，避免方形色段盖住圆角描边后露出两个黑角。
        let v_l = ((0.5 / segments as f32) * 255.0).round() as u8;
        let v_r = (255.0 - (0.5 / segments as f32) * 255.0).round() as u8;
        ctx.fill_rounded_rect(rd(cap_l), seg_r, Rgba::opaque(v_l, v_l, v_l));
        ctx.fill_rounded_rect(rd(cap_r), seg_r, Rgba::opaque(v_r, v_r, v_r));
        ctx.stroke_rounded_rect(rd(track), seg_r, p.card_border, 1.0);

        let ratio = ((thr - 300) as f32 / 400.0).clamp(0.0, 1.0);
        let kx = track.x + track.w * ratio - 6.0;
        ctx.fill_rounded_rect(rd(Rect::new(kx, track.y - 2.0, 12.0, 14.0)), 7.0, p.card_bg);
        ctx.stroke_rounded_rect(rd(Rect::new(kx, track.y - 2.0, 12.0, 14.0)), 7.0, p.accent, 2.0);

        let (lo, hi) = threshold_range_labels();
        self.label(ctx, &lo, Rect::new(track.x, track.bottom() + 3.0, 40.0, 14.0), TextRole::Caption, p.text_secondary);
        self.label_right(ctx, &hi, Rect::new(track.right() - 40.0, track.bottom() + 3.0, 40.0, 14.0), TextRole::Caption, p.text_secondary);
    }

    fn paint_sample_page(&self, ctx: &RenderCtx, p: &Palette) {
        let card = self.sample_card;
        ctx.fill_rounded_rect(rd(card), RADIUS, p.card_bg);
        ctx.stroke_rounded_rect(rd(card), RADIUS, p.card_border, 1.0);
        let inner = card.inset(PAD);
        self.label(
            ctx,
            text("sample.title", self.lang),
            Rect::new(inner.x, inner.y + 2.0, 200.0, 16.0),
            TextRole::Caption,
            p.text_secondary,
        );

        // 采样带与任务栏预览同款：壁纸底 + 居中的迷你任务栏 + 采样区外遮罩。
        let band = self.sample_band;
        ctx.fill_rounded_rect(rd(band), 6.0, p.preview_wall);
        let (bar_bg, ink, line) = self.bar_colors(p);
        let bar = self.sample_bar;
        ctx.fill_rounded_rect(rd(bar), 7.0, bar_bg);
        ctx.stroke_rounded_rect(rd(bar), 7.0, line, 1.0);

        // 开始按钮 + 应用图标
        let sx = bar.x + 12.0;
        let sy = bar.y + 12.0;
        for row in 0..2 {
            for col in 0..2 {
                ctx.fill_rounded_rect(
                    rd(Rect::new(sx + col as f32 * 8.0, sy + row as f32 * 8.0, 6.0, 6.0)),
                    1.0,
                    ink,
                );
            }
        }
        let icon_x = sx + 26.0;
        for i in 0..4 {
            let r = Rect::new(icon_x + i as f32 * 26.0, bar.y + 10.0, 20.0, 20.0);
            let fill = if i % 2 == 0 {
                Rgba::opaque(0x4A, 0x55, 0x63)
            } else {
                Rgba::opaque(0x2B, 0x32, 0x3D)
            };
            ctx.fill_rounded_rect(rd(r), 5.0, fill);
            ctx.stroke_rounded_rect(rd(r), 5.0, line, 1.0);
        }

        // 托盘 + 分界线 + 时钟
        let clock_w = 58.0;
        let div_x = bar.right() - 12.0 - clock_w - 8.0;
        for i in 0..3 {
            let g = Rect::new(div_x - 10.0 - (3 - i) as f32 * 20.0, bar.y + 14.0, 12.0, 12.0);
            match i {
                0 => ctx.stroke_rounded_rect(rd(g), 2.0, ink, 1.0),
                1 => {
                    ctx.fill_rect(rd(Rect::new(g.x + 1.0, g.y + 7.0, 3.0, 4.0)), ink);
                    ctx.fill_rect(rd(Rect::new(g.x + 5.0, g.y + 4.0, 3.0, 7.0)), ink);
                    ctx.fill_rect(rd(Rect::new(g.x + 9.0, g.y + 1.0, 3.0, 10.0)), ink);
                }
                _ => ctx.fill_rounded_rect(rd(Rect::new(g.x + 1.0, g.y + 1.0, 10.0, 10.0)), 5.0, ink),
            }
        }
        ctx.fill_rect(
            rd(Rect::new(div_x, bar.y + 9.0, 1.0, bar.h - 18.0)),
            line,
        );
        let (time, _) = local_time();
        ctx.draw_text_right(
            &time,
            rd(Rect::new(bar.right() - 12.0 - clock_w, bar.y + 12.0, clock_w, 16.0)),
            TextRole::Body,
            ink,
        );

        // 采样区：起点 / 宽度 / 偏移都作用在迷你任务栏上；区外盖回壁纸色，
        // 只留下被采样的那一段任务栏，比一条纯色块更能说明"采的是哪一块"。
        let seg = self.sample_segment();
        let dy = seg.y - bar.y;
        ctx.fill_rect(
            rd(Rect::new(bar.x, bar.y, seg.x - bar.x, bar.h)),
            p.preview_wall,
        );
        ctx.fill_rect(
            rd(Rect::new(seg.right(), bar.y, bar.right() - seg.right(), bar.h)),
            p.preview_wall,
        );
        if dy > 0.0 {
            // 引擎是"上下各内缩同样比例"，预览也必须上下同时收。
            ctx.fill_rect(
                rd(Rect::new(seg.x, bar.y, seg.w, dy)),
                p.preview_wall,
            );
            let bottom = bar.bottom() - seg.bottom();
            if bottom > 0.0 {
                ctx.fill_rect(
                    rd(Rect::new(seg.x, seg.bottom(), seg.w, bottom)),
                    p.preview_wall,
                );
            }
        }
        ctx.stroke_rounded_rect(rd(seg), 3.0, p.accent, 1.5);
        let live = if self.diag.has_data {
            self.diag.rep_color
        } else {
            p.card_alt
        };

        // 实时色块与亮度
        let chip = Rect::new(inner.x, card.y + 120.0, 20.0, 20.0);
        ctx.fill_rounded_rect(rd(chip), 4.0, live);
        ctx.stroke_rounded_rect(rd(chip), 4.0, p.card_border, 1.0);
        let live_text = if self.diag.has_data {
            format!(
                "{} · {} {:.3}",
                hex_rgb(i64::from(self.diag.rep_color.r) << 16
                    | i64::from(self.diag.rep_color.g) << 8
                    | i64::from(self.diag.rep_color.b)),
                text("diag.y", self.lang),
                self.diag.y_milli as f32 / 1000.0
            )
        } else {
            text("diag.no_data", self.lang).to_string()
        };
        self.label(
            ctx,
            &live_text,
            Rect::new(chip.right() + 10.0, card.y + 122.0, inner.w - 40.0, 18.0),
            TextRole::Caption,
            p.text_secondary,
        );

        self.paint_rows(ctx, p);
    }

    fn paint_tray_page(&self, ctx: &RenderCtx, p: &Palette) {
        self.paint_rows(ctx, p);
    }

    fn paint_diag_page(&self, ctx: &RenderCtx, p: &Palette) {
        let card = self.diag_card;
        ctx.fill_rounded_rect(rd(card), RADIUS, p.card_bg);
        ctx.stroke_rounded_rect(rd(card), RADIUS, p.card_border, 1.0);
        let inner = card.inset(PAD);

        self.label(
            ctx,
            text("diag.live", self.lang),
            Rect::new(inner.x, inner.y + 2.0, 200.0, 16.0),
            TextRole::Caption,
            p.text_secondary,
        );
        self.label_right(
            ctx,
            &format!("{} {}", self.diag.samples, text("diag.sample_unit", self.lang)),
            Rect::new(inner.right() - 160.0, inner.y + 2.0, 160.0, 16.0),
            TextRole::Caption,
            p.text_secondary,
        );

        if !self.diag.has_data {
            self.label(
                ctx,
                text("diag.no_data", self.lang),
                Rect::new(inner.x, inner.y + 42.0, inner.w, 24.0),
                TextRole::Body,
                p.text_secondary,
            );
        } else {
            ctx.draw_text(
                &format!("{:.3}", self.diag.y_milli as f32 / 1000.0),
                rd(Rect::new(inner.x, inner.y + 26.0, 220.0, 30.0)),
                TextRole::Metric,
                p.text_primary,
            );
            self.label(
                ctx,
                text("diag.y", self.lang),
                Rect::new(inner.x, inner.y + 58.0, 120.0, 14.0),
                TextRole::Caption,
                p.text_secondary,
            );

            let contrast_color = if self.diag.contrast_low() { p.danger } else { p.ok };
            ctx.draw_text_right(
                &format!("{:.1}:1", self.diag.contrast_milli as f32 / 1000.0),
                rd(Rect::new(inner.right() - 220.0, inner.y + 26.0, 220.0, 30.0)),
                TextRole::Metric,
                contrast_color,
            );
            self.label_right(
                ctx,
                text("diag.contrast", self.lang),
                Rect::new(inner.right() - 220.0, inner.y + 58.0, 220.0, 14.0),
                TextRole::Caption,
                p.text_secondary,
            );

            // 代表色 / 判定色
            let rep_argb = (i64::from(self.diag.rep_color.r) << 16)
                | (i64::from(self.diag.rep_color.g) << 8)
                | i64::from(self.diag.rep_color.b);
            let dec_argb = (i64::from(self.diag.decision_color.r) << 16)
                | (i64::from(self.diag.decision_color.g) << 8)
                | i64::from(self.diag.decision_color.b);
            for (i, (color, argb, key)) in [
                (self.diag.rep_color, rep_argb, "diag.rep_color"),
                (self.diag.decision_color, dec_argb, "diag.decision_color"),
            ]
            .iter()
            .enumerate()
            {
                let x = inner.x + i as f32 * 240.0;
                let chip = Rect::new(x, inner.y + 78.0, 18.0, 18.0);
                ctx.fill_rounded_rect(rd(chip), 4.0, *color);
                ctx.stroke_rounded_rect(rd(chip), 4.0, p.card_border, 1.0);
                self.label(
                    ctx,
                    &format!("{} {}", text(key, self.lang), hex_rgb(*argb)),
                    Rect::new(chip.right() + 8.0, inner.y + 80.0, 210.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
            }

            // 折线（10 根柱）
            let spark = self.diag_spark;
            let n = self.diag.recent_len.max(1);
            let bar_w = 8.0;
            let gap = ((spark.w - (n as f32 * bar_w)) / (n as f32 - 1.0).max(1.0)).clamp(2.0, 24.0);
            for i in 0..n {
                let v = self.diag.recent_y[i] as f32 / 1000.0;
                let bh = (v * spark.h).max(2.0);
                let r = Rect::new(
                    spark.x + i as f32 * (bar_w + gap),
                    spark.bottom() - bh,
                    bar_w,
                    bh,
                );
                let color = if self.diag.contrast_low() { p.danger } else { p.accent };
                ctx.fill_rounded_rect(rd(r), 2.0, color);
            }

            let warn = if self.diag.region_suspect() {
                Some((text("diag.region_suspect", self.lang), p.danger))
            } else if self.diag.region_small() {
                Some((i18n::text("diag.region_small", self.lang), p.warning))
            } else if self.diag.contrast_low() {
                Some((text("diag.contrast_low", self.lang), p.danger))
            } else {
                None
            };
            if let Some((msg, color)) = warn {
                self.label(
                    ctx,
                    msg,
                    Rect::new(inner.x, card.bottom() - 22.0, inner.w, 15.0),
                    TextRole::Caption,
                    color,
                );
            }
        }

        self.paint_rows(ctx, p);
    }

    fn paint_rows(&self, ctx: &RenderCtx, p: &Palette) {
        let card = self.rows_card;
        if card.w <= 0.0 || self.rows.is_empty() {
            return;
        }
        ctx.fill_rounded_rect(rd(card), RADIUS, p.card_bg);
        ctx.stroke_rounded_rect(rd(card), RADIUS, p.card_border, 1.0);

        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                ctx.fill_rect(
                    rd(Rect::new(row.rect.x + PAD, row.rect.y, row.rect.w - PAD * 2.0, 1.0)),
                    p.divider,
                );
            }
            if self.focus_visible && self.focus == i {
                ctx.stroke_rounded_rect(
                    rd(Rect::new(row.rect.x + 4.0, row.rect.y + 3.0, row.rect.w - 8.0, row.rect.h - 6.0)),
                    6.0,
                    p.accent,
                    1.0,
                );
            }
            self.paint_row(ctx, p, row, i);
        }
    }

    fn paint_row(&self, ctx: &RenderCtx, p: &Palette, row: &RowWidget, index: usize) {
        let key = row.key;
        let meta = key.meta();
        let enabled = row.enabled;
        let label_color = p.control_text(enabled);
        let hovered = self.hover == Hit::Row(index);

        self.label(
            ctx,
            text(meta.label, self.lang),
            Rect::new(row.rect.x + PAD, row.rect.y + 13.0, row.rect.w - PAD * 2.0 - 200.0, 18.0),
            TextRole::Body,
            label_color,
        );

        match meta.kind {
            ControlKind::Toggle => {
                let tr = Rect::new(
                    row.rect.right() - PAD - TOGGLE_W,
                    row.rect.y + (ROW_H - TOGGLE_H) / 2.0,
                    TOGGLE_W,
                    TOGGLE_H,
                );
                self.paint_toggle(ctx, p, tr, row.value != 0, enabled);
            }
            ControlKind::Dropdown => {
                let bw = 180.0;
                let br = Rect::new(row.rect.right() - PAD - bw, row.rect.y + 8.0, bw, 28.0);
                ctx.fill_rounded_rect(rd(br), 6.0, p.card_alt);
                ctx.stroke_rounded_rect(
                    rd(br),
                    6.0,
                    if hovered { p.accent } else { p.card_border },
                    1.0,
                );
                let option_key = meta.options.get(row.value as usize).copied().unwrap_or("");
                self.label(
                    ctx,
                    text(option_key, self.lang),
                    Rect::new(br.x + 9.0, br.y + 7.0, bw - 30.0, 16.0),
                    TextRole::Body,
                    label_color,
                );
                self.label(
                    ctx,
                    "v",
                    Rect::new(br.right() - 17.0, br.y + 7.0, 10.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
            }
            ControlKind::Slider => {
                let track = slider_track(row.rect);
                let meta = key.meta();
                let ratio = ((row.value - meta.min) as f32 / (meta.max - meta.min).max(1) as f32)
                    .clamp(0.0, 1.0);
                ctx.fill_rounded_rect(rd(track), 5.0, p.track_off);
                ctx.fill_rounded_rect(
                    rd(Rect::new(track.x, track.y, track.w * ratio, track.h)),
                    5.0,
                    if enabled { p.accent } else { p.disabled_fill },
                );
                let kx = track.x + track.w * ratio - 6.0;
                ctx.fill_rounded_rect(rd(Rect::new(kx, track.y - 2.0, 12.0, 14.0)), 7.0, p.card_bg);
                ctx.stroke_rounded_rect(
                    rd(Rect::new(kx, track.y - 2.0, 12.0, 14.0)),
                    7.0,
                    if enabled { p.accent } else { p.disabled_text },
                    2.0,
                );
                self.label_right(
                    ctx,
                    &self.row_value_text(key, row.value),
                    Rect::new(row.rect.right() - PAD - 120.0, row.rect.y + 12.0, 120.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
            }
            ControlKind::Button => {
                let br = Rect::new(row.rect.right() - PAD - 72.0, row.rect.y + 8.0, 72.0, 28.0);
                ctx.fill_rounded_rect(rd(br), 6.0, p.accent);
                self.label_center(
                    ctx,
                    text("action.export", self.lang),
                    Rect::new(br.x, br.y + 7.0, br.w, 16.0),
                    TextRole::Body,
                    p.accent_text,
                );
            }
            ControlKind::ReadOnly => {
                self.label_right(
                    ctx,
                    VERSION_TEXT,
                    Rect::new(row.rect.right() - PAD - 140.0, row.rect.y + 13.0, 140.0, 18.0),
                    TextRole::Body,
                    label_color,
                );
                if hovered {
                    self.label_right(
                        ctx,
                        text("action.copy", self.lang),
                        Rect::new(row.rect.right() - PAD - 210.0, row.rect.y + 14.0, 60.0, 15.0),
                        TextRole::Caption,
                        p.text_secondary,
                    );
                }
            }
            ControlKind::Color => {}
        }
    }

    fn row_value_text(&self, key: SettingKey, value: i64) -> String {
        match key {
            SettingKey::S16 => {
                if value < 1000 {
                    format!("{value} ms")
                } else {
                    format!("{:.1} s", value as f32 / 1000.0)
                }
            }
            SettingKey::S18 => format!("{:.2}", value as f32 / 1000.0),
            SettingKey::S20 | SettingKey::S21 | SettingKey::S22 => format!("{value}%"),
            _ => value.to_string(),
        }
    }

    fn paint_toggle(&self, ctx: &RenderCtx, p: &Palette, r: Rect, on: bool, enabled: bool) {
        let bg = if !enabled {
            p.disabled_fill
        } else if on {
            p.accent
        } else {
            p.track_off
        };
        ctx.fill_rounded_rect(rd(r), r.h / 2.0, bg);
        let knob = r.h - 4.0;
        let kx = if on { r.right() - knob - 2.0 } else { r.x + 2.0 };
        ctx.fill_rounded_rect(
            rd(Rect::new(kx, r.y + 2.0, knob, knob)),
            knob / 2.0,
            if enabled { p.card_bg } else { p.disabled_text },
        );
    }

    fn paint_footer(&self, ctx: &RenderCtx, p: &Palette, w: f32, h: f32) {
        ctx.fill_rect(rd(Rect::new(0.0, h - FOOTER_H, w, 1.0)), p.card_border);
        let labels = ["action.restore", "action.import_config", "action.export_config", "action.about"];
        for (i, key) in labels.iter().enumerate() {
            let r = self.foot_rects[i];
            let hovered = self.hover == Hit::Footer(i);
            if hovered {
                ctx.fill_rounded_rect(rd(r), 6.0, p.card_alt);
            }
            self.label_center(
                ctx,
                text(key, self.lang),
                Rect::new(r.x, r.y + 6.0, r.w, 16.0),
                TextRole::Body,
                if hovered { p.text_primary } else { p.text_secondary },
            );
        }
    }

    fn paint_toast(&self, ctx: &RenderCtx, p: &Palette, msg: &str, w: f32, h: f32) {
        // 诊断包路径可能很长：限制提示条宽度，避免超出客户区被直接裁掉。
        let tw = (text_width(msg, TextRole::Body.size()) + 32.0).min((w - 24.0).max(120.0));
        let r = Rect::new((w - tw) / 2.0, h - FOOTER_H - 44.0, tw, 30.0);
        ctx.fill_rounded_rect(rd(r), 6.0, p.card_bg);
        ctx.stroke_rounded_rect(rd(r), 6.0, p.accent, 1.0);
        self.label_center(
            ctx,
            msg,
            Rect::new(r.x, r.y + 7.0, r.w, 16.0),
            TextRole::Caption,
            p.text_primary,
        );
    }

    fn paint_popup(&self, ctx: &RenderCtx, p: &Palette) {
        let Some(popup) = self.popup.as_ref() else {
            return;
        };
        let frame = popup.frame;
        ctx.fill_rounded_rect(rd(frame), 7.0, p.card_bg);
        ctx.stroke_rounded_rect(rd(frame), 7.0, p.accent, 1.5);

        match popup.kind {
            PopupKind::Dropdown(key) => {
                let meta = key.meta();
                for (i, option) in meta.options.iter().enumerate() {
                    let r = Rect::new(frame.x + 4.0, frame.y + 4.0 + i as f32 * 28.0, frame.w - 8.0, 28.0);
                    if i == popup.selected {
                        ctx.fill_rounded_rect(rd(r), 4.0, p.card_alt);
                        ctx.fill_rounded_rect(rd(Rect::new(r.x + 3.0, r.y + 7.0, 3.0, 14.0)), 1.5, p.accent);
                    }
                    self.label(
                        ctx,
                        text(option, self.lang),
                        Rect::new(r.x + 12.0, r.y + 6.0, r.w - 20.0, 16.0),
                        TextRole::Body,
                        p.text_primary,
                    );
                }
            }
            PopupKind::ColorPicker(key) => {
                let group = Self::picker_group(key);
                let (title, colors) = i18n::COLOR_SWATCH_GROUPS[group];
                self.label(
                    ctx,
                    text(title, self.lang),
                    Rect::new(frame.x + 12.0, frame.y + 10.0, frame.w - 24.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
                let current = self.cfg.get(key);
                for (i, hex) in colors.iter().enumerate() {
                    let r = Rect::new(frame.x + 12.0 + i as f32 * 38.0, frame.y + 32.0, 32.0, 26.0);
                    let argb = parse_hex(hex) | 0xFF00_0000;
                    ctx.fill_rounded_rect(rd(r), 5.0, argb_to_rgba(argb));
                    let selected = (current | 0xFF00_0000) == argb;
                    ctx.stroke_rounded_rect(
                        rd(r),
                        5.0,
                        if selected { p.accent } else { p.card_border },
                        if selected { 2.0 } else { 1.0 },
                    );
                }
                let more = Rect::new(frame.x + 12.0, frame.y + 70.0, frame.w - 24.0, 30.0);
                ctx.fill_rounded_rect(rd(more), 6.0, p.card_alt);
                self.label(
                    ctx,
                    text("picker.more", self.lang),
                    Rect::new(more.x + 10.0, more.y + 7.0, more.w - 20.0, 16.0),
                    TextRole::Body,
                    p.text_primary,
                );
            }
            PopupKind::ConfirmRestore => {
                self.label(
                    ctx,
                    text("dlg.restore.title", self.lang),
                    Rect::new(frame.x + 18.0, frame.y + 18.0, frame.w - 36.0, 20.0),
                    TextRole::Subtitle,
                    p.text_primary,
                );
                self.label(
                    ctx,
                    text("dlg.restore.body", self.lang),
                    Rect::new(frame.x + 18.0, frame.y + 48.0, frame.w - 36.0, 18.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
                let by = frame.bottom() - 46.0;
                let cancel = Rect::new(frame.right() - 16.0 - 88.0, by, 88.0, 30.0);
                let confirm = Rect::new(cancel.x - 8.0 - 96.0, by, 96.0, 30.0);
                ctx.fill_rounded_rect(rd(confirm), 6.0, p.accent);
                self.label(
                    ctx,
                    text("dlg.confirm", self.lang),
                    Rect::new(confirm.x + 8.0, confirm.y + 7.0, confirm.w - 16.0, 16.0),
                    TextRole::Body,
                    p.accent_text,
                );
                ctx.fill_rounded_rect(rd(cancel), 6.0, p.card_alt);
                ctx.stroke_rounded_rect(rd(cancel), 6.0, p.card_border, 1.0);
                self.label(
                    ctx,
                    text("dlg.cancel", self.lang),
                    Rect::new(cancel.x + 8.0, cancel.y + 7.0, cancel.w - 16.0, 16.0),
                    TextRole::Body,
                    p.text_primary,
                );
            }
            PopupKind::About => {
                self.label(
                    ctx,
                    text("about.title", self.lang),
                    Rect::new(frame.x + 20.0, frame.y + 18.0, frame.w - 40.0, 22.0),
                    TextRole::Title,
                    p.text_primary,
                );
                self.label(
                    ctx,
                    &format!("{} {}", text("about.version", self.lang), VERSION_TEXT),
                    Rect::new(frame.x + 20.0, frame.y + 50.0, frame.w - 40.0, 18.0),
                    TextRole::Body,
                    p.text_secondary,
                );
                self.label(
                    ctx,
                    text("about.license", self.lang),
                    Rect::new(frame.x + 20.0, frame.y + 72.0, frame.w - 40.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
                self.label(
                    ctx,
                    text("about.known_limits", self.lang),
                    Rect::new(frame.x + 20.0, frame.y + 90.0, frame.w - 40.0, 16.0),
                    TextRole::Caption,
                    p.text_secondary,
                );
                let btn = Rect::new(frame.x + 16.0, frame.bottom() - 44.0, 140.0, 28.0);
                ctx.fill_rounded_rect(rd(btn), 6.0, p.card_alt);
                ctx.stroke_rounded_rect(rd(btn), 6.0, p.card_border, 1.0);
                self.label(
                    ctx,
                    text("about.copy_version", self.lang),
                    Rect::new(btn.x + 8.0, btn.y + 6.0, btn.w - 16.0, 16.0),
                    TextRole::Caption,
                    p.text_primary,
                );
            }
        }
    }

    // -----------------------------------------------------------------
    // 文本
    // -----------------------------------------------------------------

    fn label(&self, ctx: &RenderCtx, text: &str, r: Rect, role: TextRole, color: Rgba) {
        let size = role_size(role);
        let s = ellipsize(text, size, r.w);
        ctx.draw_text(&s, rd(r), role, color);
    }

    fn label_right(&self, ctx: &RenderCtx, text: &str, r: Rect, role: TextRole, color: Rgba) {
        let size = role_size(role);
        let s = ellipsize(text, size, r.w);
        ctx.draw_text_right(&s, rd(r), role, color);
    }

    fn label_center(&self, ctx: &RenderCtx, text: &str, r: Rect, role: TextRole, color: Rgba) {
        let size = role_size(role);
        let s = ellipsize(text, size, r.w);
        ctx.draw_text_center(&s, rd(r), role, color);
    }
}

// ---------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------

fn rd(r: Rect) -> RectDip {
    RectDip::new(r.x, r.y, r.w, r.h)
}

fn slider_track(row: Rect) -> Rect {
    // 与阈值 / 不透明度滑条同款：10 dip 高的圆角轨道。
    Rect::new(row.x + PAD, row.y + 30.0, row.w - PAD * 2.0, 10.0)
}

fn role_size(role: TextRole) -> f32 {
    role.size()
}

/// 阈值滑杆两端的刻度文案，必须来自 `S-12` 的真实范围（0.30~0.70），
/// 不能写死 0 / 1。
fn threshold_range_labels() -> (String, String) {
    let m = SettingKey::S12.meta();
    (
        format!("{:.2}", m.min as f32 / 1000.0),
        format!("{:.2}", m.max as f32 / 1000.0),
    )
}

/// 采样带上下内缩高度。`S-22` 是任务栏高度的百分比，引擎按
/// 上、下各内缩同样比例取样（见 `expected_sample_rect`），预览必须同口径。
fn band_inset(bar: Rect, pct: i64) -> f32 {
    (bar.h * pct.clamp(0, 100) as f32 / 100.0).clamp(0.0, bar.h * 0.45)
}

/// 阈值渐变滑条的三段矩形：左右圆帽各宽 `2r`，中段铺渐变。
/// 直接铺满整条会在圆角外露出方角色块（左侧 `v=0` 看起来就是两个黑点）。
fn gradient_spans(track: Rect, radius: f32) -> (Rect, Rect, Rect) {
    let r = radius.min(track.h * 0.5).min(track.w * 0.5);
    let mid = Rect::new(track.x + r, track.y, (track.w - 2.0 * r).max(0.0), track.h);
    let left = Rect::new(track.x, track.y, 2.0 * r, track.h);
    let right = Rect::new(track.right() - 2.0 * r, track.y, 2.0 * r, track.h);
    (left, mid, right)
}

fn parse_hex(s: &str) -> i64 {
    i64::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0) | 0xFF00_0000
}

fn local_time() -> (String, String) {
    // SAFETY: GetLocalTime 无前置条件，只读系统时间。
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    (
        format!("{:02}:{:02}", st.wHour, st.wMinute),
        format!("{}/{}/{}", st.wYear, st.wMonth, st.wDay),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view() -> View {
        make_view_lang(Lang::Zh)
    }

    fn make_view_lang(lang: Lang) -> View {
        let cfg = Config::default();
        let mut v = View::new(&cfg, lang, false);
        v.set_dpi(96);
        v.set_client(theme::MIN_WIDTH_DIP, theme::MIN_HEIGHT_DIP);
        v.rebuild(&cfg, &DiagView::default(), false);
        v
    }

    fn click(v: &mut View, x: f32, y: f32) {
        let lp = LPARAM(((y as i32) << 16 | (x as i32 & 0xFFFF)) as isize);
        v.on_message(HWND::default(), WM_LBUTTONDOWN, WPARAM(0), lp).unwrap();
        v.on_message(HWND::default(), WM_LBUTTONUP, WPARAM(0), lp).unwrap();
    }

    fn center(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn has_four_tabs_in_order() {
        let v = make_view();
        assert_eq!(v.tab_rects.len(), 4);
        let labels: Vec<&str> = Page::ALL.iter().map(|p| text(p.title_key(), v.lang)).collect();
        assert_eq!(labels, vec!["任务栏", "采样", "托盘", "诊断"]);
        let mut last = -1.0;
        for r in v.tab_rects {
            assert!(r.x > last, "页签必须从左到右排列");
            last = r.x;
        }
    }

    /// 组 1 回归：所有固定文案都必须完整放进它的控件矩形，不能出现「恢复默…」。
    #[test]
    fn control_labels_fit_without_ellipsis() {
        for lang in [Lang::Zh, Lang::En] {
            let v = make_view_lang(lang);
            let fits = |s: &str, role: TextRole, w: f32| ellipsize(s, role.size(), w) == s;

            let foot = [
                "action.restore",
                "action.import_config",
                "action.export_config",
                "action.about",
            ];
            for (i, key) in foot.iter().enumerate() {
                let s = text(key, lang);
                assert!(fits(s, TextRole::Body, v.foot_rects[i].w), "{lang:?} 底栏「{s}」被截断");
            }
            let pause = text("ui.pause", lang);
            assert!(fits(pause, TextRole::Body, v.pause_btn.w), "{lang:?} 暂停「{pause}」被截断");
            for (i, page) in Page::ALL.iter().enumerate() {
                let s = text(page.title_key(), lang);
                assert!(
                    fits(s, TextRole::Subtitle, v.tab_rects[i].w),
                    "{lang:?} 页签「{s}」被截断"
                );
            }
            for (i, key) in ["preview.auto", "preview.light", "preview.dark"]
                .iter()
                .enumerate()
            {
                let s = text(key, lang);
                assert!(
                    fits(s, TextRole::Caption, v.mode_rects[i].w),
                    "{lang:?} 模式胶囊「{s}」被截断"
                );
            }
            for (i, key) in ["opt.judge.region", "opt.judge.global"].iter().enumerate() {
                let s = text(key, lang);
                assert!(
                    fits(s, TextRole::Caption, v.judge_rects[i].w),
                    "{lang:?} 判定分段「{s}」被截断"
                );
            }
        }
    }

    #[test]
    fn clicking_tab_switches_page_and_rows() {
        let mut v = make_view();
        let (x, y) = center(v.tab_rects[2]);
        click(&mut v, x, y);
        assert_eq!(v.page, Page::Tray);
        assert_eq!(v.rows.len(), 6);
        assert_eq!(v.rows[0].key, SettingKey::S40);
        assert_eq!(v.rows[1].key, SettingKey::S51, "双击必须紧跟在单击下方");
        assert!(
            v.rows_card.h >= 24.0 + v.rows.len() as f32 * ROW_H,
            "卡片必须容纳全部行，最后一行不能掉到卡片外"
        );
    }

    #[test]
    fn tint_and_pause_write_expected_keys() {
        let mut v = make_view();
        let (x, y) = center(v.tint_toggle);
        click(&mut v, x, y);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S01, 0)]);

        let (x, y) = center(v.pause_btn);
        click(&mut v, x, y);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S42, 1)]);
    }

    #[test]
    fn zone_cards_follow_icons_tray_clock_order() {
        let mut v = make_view();
        let want = [SettingKey::S50, SettingKey::S49, SettingKey::S48];
        for (i, key) in want.iter().enumerate() {
            let (x, y) = center(v.zone_rects[i]);
            click(&mut v, x, y);
            assert_eq!(v.take_actions(), vec![ViewAction::Set(*key, 0)], "第 {i} 张卡片");
        }
        // 预览里托盘必须在时钟左侧
        assert!(v.mode_rects[0].y < v.zone_rects[0].y);
    }

    /// 组 2 回归：^ 必须在托盘组最左、足够宽，且分界线上不能压任何内容。
    #[test]
    fn tray_chevron_is_leftmost_and_clear_of_divider() {
        let v = make_view();
        let chev = v.tray_chevron;
        let divider_x = v.preview_clock.x - 10.0;
        assert!(chev.w >= 14.0, "^ 必须比原来的 10 px 宽");
        assert!(chev.x < v.tray_glyphs[0].x, "^ 必须在托盘图标左侧");
        assert!(
            chev.x + chev.w <= divider_x - 4.0,
            "^ 不能压到托盘/时钟分界线（chev right={}, divider={divider_x}）",
            chev.right()
        );
        for pair in v.tray_glyphs.windows(2) {
            assert!(pair[0].right() < pair[1].x, "托盘图标不能重叠");
        }
        assert!(
            v.tray_glyphs[2].right() <= divider_x - 4.0,
            "托盘图标不能越过分界线"
        );
        assert!(
            chev.y >= v.preview_bar.y && chev.bottom() <= v.preview_bar.bottom(),
            "^ 必须落在预览条内"
        );
        let icon_cy = v.tray_glyphs[0].y + v.tray_glyphs[0].h / 2.0;
        let chev_cy = chev.y + chev.h / 2.0;
        assert!(
            (icon_cy - chev_cy).abs() <= 1.0,
            "^ 必须与托盘图标垂直居中（icon={icon_cy}, chev={chev_cy}）"
        );
    }

    /// 组 1 回归（问题 4）：强制端色时，任务栏预览的背景必须跟着「文字端」反向。
    ///
    /// 「亮端」= 亮端色（浅色文字）→ 深色任务栏；「暗端」= 暗端色 → 浅色任务栏。
    /// 旧实现把两者画反，这里同时锁住背景、前景与判定色三处。
    #[test]
    fn preview_bar_follows_light_mode() {
        let mut cfg = Config::default();
        let mut v = View::new(&cfg, Lang::Zh, false);
        v.set_dpi(96);
        v.set_client(theme::MIN_WIDTH_DIP, theme::MIN_HEIGHT_DIP);
        let p = crate::ui::theme::ThemeMode::Dark.palette();

        // 强制亮端色 → 深色条 + 浅色字
        cfg.set(SettingKey::S10, 1);
        v.rebuild(&cfg, &DiagView::default(), false);
        assert!(!v.preview_bar_light(), "强制亮端色必须用深色任务栏");
        assert_eq!(v.bar_colors(p).0, p.preview_bar, "强制亮端色时条底色必须是深色");
        assert_eq!(v.bar_colors(p).1, p.preview_ink, "强制亮端色时文字必须是浅色");

        // 强制暗端色 → 浅色条 + 深色字
        cfg.set(SettingKey::S10, 2);
        v.rebuild(&cfg, &DiagView::default(), false);
        assert!(v.preview_bar_light(), "强制暗端色必须用浅色任务栏");
        assert_eq!(v.bar_colors(p).0, p.preview_bar_light, "强制暗端色时条底色必须是浅色");
        assert_eq!(v.bar_colors(p).1, p.preview_ink_dark, "强制暗端色时文字必须是深色");

        cfg.set(SettingKey::S10, 0);
        cfg.set(SettingKey::S12, 500);
        let mut d = DiagView::default();
        d.has_data = true;
        d.y_milli = 800;
        v.rebuild(&cfg, &d, false);
        assert!(v.preview_bar_light(), "自动模式亮度高于阈值应转浅色条");
        d.y_milli = 200;
        v.rebuild(&cfg, &d, false);
        assert!(!v.preview_bar_light(), "自动模式亮度低于阈值应保持深色条");

        // 强制亮端色时打开「暂停」：元素回落系统白，条必须仍为深色
        cfg.set(SettingKey::S10, 1);
        cfg.set(SettingKey::S42, 1);
        v.rebuild(&cfg, &d, false);
        assert!(!v.preview_bar_light(), "暂停时元素回落白色，条必须保持深色");
    }

    /// 组 1 回归（问题 3）：最右列「时间与日期」的取色浮层必须完整落在窗口内，
    /// 且右半边色块仍可点击。
    #[test]
    fn color_picker_popup_stays_inside_window_for_clock_column() {
        let mut v = make_view();
        let (w, _h) = v.client;
        // 偏亮行第 4 列 = 时间与日期（S-59），锚点最靠右
        let cell = v.item_wells[1][3];
        let (cx, cy) = center(cell);
        click(&mut v, cx, cy);
        let popup = v.popup.as_ref().expect("点击色块应打开取色浮层");
        let PopupKind::ColorPicker(SettingKey::S59) = popup.kind else {
            panic!("应打开 S-59 的取色浮层");
        };
        let f = popup.frame;
        assert!(f.x >= 8.0, "浮层左缘越界：{}", f.x);
        assert!(f.right() <= w - 8.0, "浮层右缘越界：{} > {}", f.right(), w - 8.0);

        // 最后一个色块（最靠右）必须可点并写入
        let sw = 32.0;
        let gap = 6.0;
        let last_x = f.x + 12.0 + 5.0 * (sw + gap) + sw / 2.0;
        let last_y = f.y + 32.0 + 13.0;
        assert!(last_x <= w, "最后一个色块中心越界：{last_x}");
        click(&mut v, last_x, last_y);
        assert!(v.popup.is_none(), "点中色块后浮层应关闭");
        assert!(
            matches!(v.take_actions().first(), Some(ViewAction::Set(SettingKey::S59, _))),
            "点中右侧色块必须写入 S-59"
        );
    }

    /// 组 3 回归：采样带里的迷你任务栏必须垂直居中。
    #[test]
    fn sample_bar_is_vertically_centered() {
        let mut v = make_view();
        let (tx, ty) = center(v.tab_rects[1]);
        click(&mut v, tx, ty);
        assert!(v.sample_bar.h > 0.0, "采样页应已布局迷你任务栏");
        assert_eq!(v.sample_bar.w, v.sample_band.w);
        let top_gap = v.sample_bar.y - v.sample_band.y;
        let bottom_gap = v.sample_band.bottom() - v.sample_bar.bottom();
        assert!((top_gap - bottom_gap).abs() <= 0.5, "上下留白不一致：{top_gap} vs {bottom_gap}");
        assert!((top_gap - 10.0).abs() <= 0.5, "留白应为 10 dip，实际 {top_gap}");
    }

    /// 组 4 回归：阈值刻度必须显示真实范围，而不是 0 / 1。
    #[test]
    fn threshold_scale_matches_actual_range() {
        assert_eq!(threshold_range_labels(), ("0.30".to_string(), "0.70".to_string()));
    }

    /// 组 4 回归：采样段范围必须与 recolor 的真实生效范围一致（10~84%，末端不越托盘）。
    #[test]
    fn sample_segment_matches_real_range() {
        let mut v = make_view();
        let (tx, ty) = center(v.tab_rects[1]);
        click(&mut v, tx, ty);
        let bar = v.sample_bar;

        let seg = v.sample_segment();
        assert!((seg.x - bar.x).abs() <= 0.5, "默认起点应为 0%");
        assert!(
            (seg.right() - (bar.x + bar.w * 0.84)).abs() <= 0.5,
            "默认宽度应在托盘区起点 84% 收口，实际 {}",
            (seg.right() - bar.x) / bar.w
        );

        let mut cfg = v.cfg.clone();
        cfg.set(SettingKey::S20, 40);
        cfg.set(SettingKey::S21, 84);
        v.rebuild(&cfg, &DiagView::default(), false);
        let seg = v.sample_segment();
        assert!((seg.right() - (bar.x + bar.w * 0.84)).abs() <= 0.5, "起点 40% 时末端仍不得越过 84%");

        cfg.set(SettingKey::S20, 0);
        cfg.set(SettingKey::S21, 10);
        v.rebuild(&cfg, &DiagView::default(), false);
        assert!((v.sample_segment().w - bar.w * 0.10).abs() <= 0.5, "宽度 10% 必须画成 10%");
    }

    /// 问题 3 回归：`S-22` 是百分比，28~45 区间预览必须继续变化，
    /// 且上下两边同时内缩（与引擎 `expected_sample_rect` 同口径）。
    #[test]
    fn sample_segment_offset_changes_across_28_to_45() {
        let mut v = make_view();
        let (tx, ty) = center(v.tab_rects[1]);
        click(&mut v, tx, ty);
        let bar = v.sample_bar;

        let mut cfg = v.cfg.clone();
        cfg.set(SettingKey::S22, 28);
        v.rebuild(&cfg, &DiagView::default(), false);
        let a = v.sample_segment();

        cfg.set(SettingKey::S22, 45);
        v.rebuild(&cfg, &DiagView::default(), false);
        let b = v.sample_segment();

        assert!(b.y > a.y + 1.0, "45% 顶边内缩必须比 28% 更深：y={} vs {}", b.y, a.y);
        assert!(b.h < a.h, "上下同时内缩，剩余高度必须变小");
        assert!((a.y - bar.y - bar.h * 0.28).abs() <= 0.6, "28% 顶边内缩应等于高度×0.28");
        assert!((b.y - bar.y - bar.h * 0.45).abs() <= 0.6, "45% 顶边内缩应等于高度×0.45");
        let a_bottom = bar.bottom() - a.bottom();
        let b_bottom = bar.bottom() - b.bottom();
        assert!(
            (a_bottom - bar.h * 0.28).abs() <= 0.6 && (b_bottom - bar.h * 0.45).abs() <= 0.6,
            "底边内缩必须与顶边对称"
        );
    }

    /// 问题 2 回归：渐变滑条中段两侧要留出圆帽，方形色段不得再盖住圆角。
    #[test]
    fn threshold_gradient_leaves_round_caps() {
        let track = Rect::new(10.0, 20.0, 100.0, 10.0);
        let (left, mid, right) = gradient_spans(track, 5.0);
        assert!((left.x - track.x).abs() < 0.01 && (left.w - 10.0).abs() < 0.01);
        assert!((mid.x - (track.x + 5.0)).abs() < 0.01);
        assert!((mid.w - 90.0).abs() < 0.01);
        assert!((right.right() - track.right()).abs() < 0.01 && (right.w - 10.0).abs() < 0.01);
        // 圆帽是整圆（宽 2r），中段从左右中线向内铺渐变：
        // 圆帽的直边半边被渐变覆盖，圆弧半边保住圆角。
        assert!((mid.x - (track.x + 5.0)).abs() < 0.01, "中段应从左侧中线开始");
        assert!((mid.right() - (track.right() - 5.0)).abs() < 0.01, "中段应止于右侧中线");
    }

    #[test]
    fn threshold_click_clamps_into_300_700() {
        let mut v = make_view();
        let t = v.thr_track;
        click(&mut v, t.x + 0.5, t.y + 4.0);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S12, 300)]);
        click(&mut v, t.right() - 0.5, t.y + 4.0);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S12, 700)]);
    }

    #[test]
    fn color_well_opens_picker_and_swatch_writes_opaque_color() {
        let mut v = make_view();
        // 问题 2 之后矩阵在两种判定粒度下都存在，默认配置即可点到 S-52。
        let (x, y) = center(v.item_wells[0][0]);
        click(&mut v, x, y);
        let popup = v.popup.as_ref().expect("色样应打开取色浮层");
        assert_eq!(popup.kind, PopupKind::ColorPicker(SettingKey::S52));
        let frame = popup.frame;

        // 第 2 格（默认白色已生效，选它不会产生写入，故取相邻色）。
        let swatch = Rect::new(frame.x + 12.0 + 38.0, frame.y + 32.0, 32.0, 26.0);
        let (sx, sy) = center(swatch);
        click(&mut v, sx, sy);
        assert!(v.popup.is_none());
        assert_eq!(
            v.take_actions(),
            vec![ViewAction::Set(SettingKey::S52, 0xFFFF_B4AB)]
        );
    }

    /// 问题 2：8 个色块与判定粒度解耦，两种模式都能点开对应键。
    #[test]
    fn matrix_wells_map_to_eight_keys_in_both_judge_modes() {
        let mut v = make_view();
        for mode in [0i64, 1] {
            let mut cfg = v.cfg.clone();
            cfg.set(SettingKey::S47, mode);
            v.rebuild(&cfg, &DiagView::default(), false);
            for r in 0..2 {
                for c in 0..4 {
                    let (x, y) = center(v.item_wells[r][c]);
                    click(&mut v, x, y);
                    let popup = v.popup.as_ref().expect("色块应打开取色浮层");
                    assert_eq!(
                        popup.kind,
                        PopupKind::ColorPicker(ITEM_KEYS[r][c]),
                        "S-47={mode} 时色块必须可点"
                    );
                    v.popup = None;
                }
            }
        }
    }

    /// 问题 1：偏暗行用浅色板、偏亮行用深色板，不能两组混用。
    #[test]
    fn picker_group_matches_background_state_row() {
        assert_eq!(View::picker_group(SettingKey::S52), 0);
        assert_eq!(View::picker_group(SettingKey::S58), 0);
        assert_eq!(View::picker_group(SettingKey::S53), 1);
        assert_eq!(View::picker_group(SettingKey::S59), 1);
    }

    /// 问题 1：不透明度滑条点最右端写 100%，点最左端写 0%。
    #[test]
    fn opacity_slider_writes_s60() {
        let mut v = make_view();
        let t = v.opacity_track;
        click(&mut v, t.right() - 0.5, t.y + 2.0);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S60, 100)]);
        click(&mut v, t.x + 0.2, t.y + 2.0);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S60, 0)]);
    }

    /// 问题 1：分区模式下预览按内容取各自的颜色。
    #[test]
    fn region_preview_uses_per_item_colors() {
        let mut v = make_view();
        let mut cfg = v.cfg.clone();
        cfg.set(SettingKey::S47, 0);
        cfg.set(SettingKey::S10, 2); // 强制暗端色 = 背景偏亮
        cfg.set(SettingKey::S59, 0xFF11_2233); // 时间日期·偏亮
        cfg.set(SettingKey::S57, 0xFF44_5566); // 托盘·偏亮
        cfg.set(SettingKey::S53, 0xFF77_8899); // 正常指示条·偏亮
        v.rebuild(&cfg, &DiagView::default(), false);
        let p = ThemeMode::Light.palette();
        assert_eq!(v.zone_color(SettingKey::S48, &p), Rgba::opaque(0x11, 0x22, 0x33));
        assert_eq!(v.zone_color(SettingKey::S49, &p), Rgba::opaque(0x44, 0x55, 0x66));
        assert_eq!(v.zone_color(SettingKey::S50, &p), Rgba::opaque(0x77, 0x88, 0x99));
    }

    /// 问题 1：不透明度滑条在「全任务栏统一」模式下也必须能拖（曾经只在分区模式命中）。
    #[test]
    fn opacity_slider_works_in_global_mode_too() {
        let mut v = make_view();
        let mut cfg = v.cfg.clone();
        cfg.set(SettingKey::S47, 1);
        v.rebuild(&cfg, &DiagView::default(), false);
        let t = v.opacity_track;
        click(&mut v, t.right() - 0.5, t.y + 2.0);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S60, 100)]);
    }

    /// 问题 2：设置行滑条与阈值滑条同款（10 dip 轨道）。
    #[test]
    fn row_slider_track_matches_threshold_style() {
        let row = Rect::new(0.0, 0.0, 400.0, ROW_H);
        let t = slider_track(row);
        assert!((t.h - 10.0).abs() < 0.01, "轨道高度必须与阈值滑条一致");
        assert!((t.y - (row.y + 30.0)).abs() < 0.01);
    }

    /// 问题 2：采样带内缩的读数单位必须与语义一致（%），不能再显示 px。
    #[test]
    fn sample_offset_value_uses_percent_unit() {
        let v = make_view();
        assert_eq!(v.row_value_text(SettingKey::S22, 45), "45%");
        assert_eq!(v.row_value_text(SettingKey::S20, 0), "0%");
    }

    #[test]
    fn footer_buttons_are_wired() {
        let mut v = make_view();
        let (x, y) = center(v.foot_rects[0]);
        click(&mut v, x, y);
        assert!(matches!(v.popup.as_ref().map(|p| p.kind), Some(PopupKind::ConfirmRestore)));
        v.popup = None;

        let (x, y) = center(v.foot_rects[1]);
        click(&mut v, x, y);
        assert_eq!(v.take_actions(), vec![ViewAction::ImportConfig]);

        let (x, y) = center(v.foot_rects[2]);
        click(&mut v, x, y);
        assert_eq!(v.take_actions(), vec![ViewAction::ExportConfig]);

        let (x, y) = center(v.foot_rects[3]);
        click(&mut v, x, y);
        assert!(matches!(v.popup.as_ref().map(|p| p.kind), Some(PopupKind::About)));
    }

    #[test]
    fn diagnostics_rows_export_and_copy() {
        let mut v = make_view();
        let (x, y) = center(v.tab_rects[3]);
        click(&mut v, x, y);
        assert_eq!(v.rows.len(), 3);
        assert_eq!(v.rows[0].key, SettingKey::S45);

        let (x, y) = center(v.rows[1].rect);
        click(&mut v, x, y);
        assert!(v.take_actions().contains(&ViewAction::CopyVersion));

        let (x, y) = center(v.rows[2].rect);
        click(&mut v, x, y);
        assert!(v.take_actions().contains(&ViewAction::ExportDiagnostics));
    }

    #[test]
    fn sample_sliders_map_endpoints_to_min_and_max() {
        let mut v = make_view();
        let (x, y) = center(v.tab_rects[1]);
        click(&mut v, x, y);
        let row = v.rows.iter().find(|r| r.key == SettingKey::S16).unwrap().rect;
        let track = slider_track(row);
        click(&mut v, track.x, track.y);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S16, 500)]);
        click(&mut v, track.right(), track.y);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S16, 5000)]);
    }

    #[test]
    fn dropdown_row_opens_list_and_selects() {
        let mut v = make_view();
        let (x, y) = center(v.tab_rects[2]);
        click(&mut v, x, y);
        let row = v.rows.iter().find(|r| r.key == SettingKey::S04).unwrap().rect;
        let (mx, my) = center(row);
        click(&mut v, mx, my);
        let popup = v.popup.as_ref().expect("下拉应打开列表");
        let frame = popup.frame;
        let item = Rect::new(frame.x + 4.0, frame.y + 4.0 + 28.0, frame.w - 8.0, 28.0);
        let (ix, iy) = center(item);
        click(&mut v, ix, iy);
        assert_eq!(v.take_actions(), vec![ViewAction::Set(SettingKey::S04, 1)]);
    }

    #[test]
    fn every_control_stays_inside_client_area() {
        let mut v = make_view();
        for (i, page) in Page::ALL.iter().enumerate() {
            let (x, y) = center(v.tab_rects[i]);
            click(&mut v, x, y);
            let mut rects: Vec<Rect> = v.rows.iter().map(|r| r.rect).collect();
            rects.extend_from_slice(&v.tab_rects);
            rects.extend_from_slice(&v.foot_rects);
            rects.extend_from_slice(&v.zone_rects);
            for row in v.item_wells.iter() {
                rects.extend_from_slice(row);
            }
            rects.extend_from_slice(&v.judge_rects);
            rects.extend_from_slice(&v.mode_rects);
            rects.push(v.tint_toggle);
            rects.push(v.pause_btn);
            rects.push(v.preview);
            rects.push(v.colors_card);
            rects.push(v.sample_card);
            rects.push(v.diag_card);
            for r in rects {
                assert!(r.x >= 0.0 && r.y >= 0.0, "控件越出左上角: {r:?}");
                assert!(
                    r.right() <= theme::MIN_WIDTH_DIP + 0.5 && r.bottom() <= theme::MIN_HEIGHT_DIP + 0.5,
                    "控件越出客户区: {r:?} (页 {:?})",
                    page
                );
            }
        }
    }

    #[test]
    fn keyboard_tab_moves_focus_and_arrows_nudge_slider() {
        let mut v = make_view();
        let (x, y) = center(v.tab_rects[1]);
        click(&mut v, x, y);
        v.handle_key(VK_TAB);
        assert_eq!(v.focus, 0);
        let before = v.rows[0].value;
        v.handle_key(VK_RIGHT);
        assert_eq!(
            v.take_actions(),
            vec![ViewAction::Set(SettingKey::S16, before + 100)]
        );
    }

    #[test]
    fn popup_escape_closes_without_writing() {
        let mut v = make_view();
        let (x, y) = center(v.item_wells[0][0]);
        click(&mut v, x, y);
        assert!(v.popup.is_some());
        v.handle_key(VK_ESCAPE);
        assert!(v.popup.is_none());
        assert!(v.take_actions().is_empty());
    }

    #[test]
    fn y_readout_reflects_threshold_side() {
        let mut v = make_view();
        // 标签跟「实际使用的端色」走：背景偏暗 → 亮端色（浅色文字）
        let diag = DiagView {
            has_data: true,
            y_milli: 100,
            ..DiagView::default()
        };
        v.rebuild(&Config::default(), &diag, false);
        assert!(v.y_text().contains("亮端"), "Y=0.1（暗背景）应使用亮端色: {}", v.y_text());
        let diag = DiagView {
            has_data: true,
            y_milli: 900,
            ..DiagView::default()
        };
        v.rebuild(&Config::default(), &diag, false);
        assert!(v.y_text().contains("暗端"), "Y=0.9（亮背景）应使用暗端色: {}", v.y_text());
    }
}
