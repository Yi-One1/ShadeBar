//! 主题与调色板（§4 #29）。
//!
//! # 为什么颜色值是这些数
//!
//! 客户区底色取自 Windows 11 自身在两种主题下的"应用背景"实测值
//! （SP-1 的像素采样里，`backdrop=4` 的客户区正是这两种色）：
//! 浅色 `#F3F3F3`、深色 `#202020`。
//! 用它们而不是自定义色，是为了与系统窗口并排时观感一致（FR-40）。
//!
//! # B3 定稿（docs/spike-report.md §3.8）
//!
//! 客户区**不透明纯色**，标题栏由 DWM 的 `backdrop = 4`（`DWMSBT_TABBEDWINDOW`）提供材质。
//! 因此 `Palette` 里没有"半透明"语义的颜色——所有颜色 α 恒为 255，
//! 一旦有人需要 α < 255，说明设计方案被改动了，应当先改这份文档而不是偷偷放宽。
//!
//! T03.3 控件库落地后，[`Palette`] 增加了控件级颜色（强调色 / 开关轨 /
//! 滑块轨 / 警告 / 危险 / 置灰）。它们同样全部 α = 255：客户区不透明是 B3 定稿，
//! 置灰靠换色而不是降透明度，避免重新引入 α 合成。

use shadebar_core::color::Rgba;
use windows::core::HSTRING;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
};

/// 系统深浅色模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// 浅色。
    Light,
    /// 深色。
    Dark,
}

/// 一套调色板。字段名与用途一一对应，**不做主题无关的抽象**（Fluent 的两套主题差异很小）。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 客户区主背景（不透明）。
    pub window_bg: Rgba,
    /// 页面标题区背景（不透明）。
    pub heading_bg: Rgba,
    /// 分组卡片填充（不透明）。
    pub card_bg: Rgba,
    /// 卡片内的凹陷底（色样槽、分段控件底）。
    pub card_alt: Rgba,
    /// 卡片 1 px 描边。
    pub card_border: Rgba,
    /// 卡片内分隔线（比描边更淡）。
    pub divider: Rgba,
    /// 任务栏预览的壁纸底。
    pub preview_wall: Rgba,
    /// 任务栏预览的任务栏条。
    pub preview_bar: Rgba,
    /// 任务栏预览的描边。
    pub preview_line: Rgba,
    /// 任务栏预览的前景色（未着色元素）。
    pub preview_ink: Rgba,
    /// 任务栏预览的次要文字（区域标签）。
    pub preview_muted: Rgba,
    /// 主文本。
    pub text_primary: Rgba,
    /// 次要文本。
    pub text_secondary: Rgba,
    /// 强调色（Windows 11 默认蓝）。
    pub accent: Rgba,
    /// 强调色上的文本。
    pub accent_text: Rgba,
    /// 关闭态开关 / 未选中滑块轨。
    pub track_off: Rgba,
    /// 置灰控件填充（NFR-05：置灰但保留设定值）。
    pub disabled_fill: Rgba,
    /// 置灰控件文本。
    pub disabled_text: Rgba,
    /// 警告（对比度不足、采样区过小）。
    pub warning: Rgba,
    /// 危险/标红（对比度 < 3:1）。
    pub danger: Rgba,
    /// 正常/达标。
    pub ok: Rgba,
}

/// 浅色调色板。
pub const LIGHT: Palette = Palette {
    window_bg: Rgba::opaque(0xF3, 0xF3, 0xF3),
    heading_bg: Rgba::opaque(0xF3, 0xF3, 0xF3),
    card_bg: Rgba::opaque(0xFF, 0xFF, 0xFF),
    card_alt: Rgba::opaque(0xF7, 0xF7, 0xF7),
    card_border: Rgba::opaque(0xE1, 0xE1, 0xE1),
    divider: Rgba::opaque(0xEF, 0xEF, 0xEF),
    preview_wall: Rgba::opaque(0x24, 0x28, 0x30),
    preview_bar: Rgba::opaque(0x14, 0x17, 0x1B),
    preview_line: Rgba::opaque(0x3A, 0x3F, 0x45),
    preview_ink: Rgba::opaque(0xF2, 0xF2, 0xF2),
    preview_muted: Rgba::opaque(0x9A, 0xA3, 0xAD),
    text_primary: Rgba::opaque(0x1A, 0x1A, 0x1A),
    text_secondary: Rgba::opaque(0x5C, 0x5C, 0x5C),
    accent: Rgba::opaque(0x00, 0x78, 0xD4),
    accent_text: Rgba::opaque(0xFF, 0xFF, 0xFF),
    track_off: Rgba::opaque(0xC8, 0xC8, 0xC8),
    disabled_fill: Rgba::opaque(0xEC, 0xEC, 0xEC),
    disabled_text: Rgba::opaque(0xA0, 0xA0, 0xA0),
    warning: Rgba::opaque(0xB8, 0x6A, 0x00),
    danger: Rgba::opaque(0xC4, 0x2B, 0x1C),
    ok: Rgba::opaque(0x0F, 0x7B, 0x0F),
};

/// 深色调色板。
pub const DARK: Palette = Palette {
    window_bg: Rgba::opaque(0x20, 0x20, 0x20),
    heading_bg: Rgba::opaque(0x20, 0x20, 0x20),
    card_bg: Rgba::opaque(0x2C, 0x2C, 0x2C),
    card_alt: Rgba::opaque(0x33, 0x33, 0x33),
    card_border: Rgba::opaque(0x3A, 0x3A, 0x3A),
    divider: Rgba::opaque(0x35, 0x35, 0x35),
    preview_wall: Rgba::opaque(0x24, 0x28, 0x30),
    preview_bar: Rgba::opaque(0x14, 0x17, 0x1B),
    preview_line: Rgba::opaque(0x3A, 0x3F, 0x45),
    preview_ink: Rgba::opaque(0xF2, 0xF2, 0xF2),
    preview_muted: Rgba::opaque(0x9A, 0xA3, 0xAD),
    text_primary: Rgba::opaque(0xFF, 0xFF, 0xFF),
    text_secondary: Rgba::opaque(0xA8, 0xA8, 0xA8),
    accent: Rgba::opaque(0x60, 0xCD, 0xFF),
    accent_text: Rgba::opaque(0x00, 0x20, 0x30),
    track_off: Rgba::opaque(0x55, 0x55, 0x55),
    disabled_fill: Rgba::opaque(0x36, 0x36, 0x36),
    disabled_text: Rgba::opaque(0x70, 0x70, 0x70),
    warning: Rgba::opaque(0xFC, 0xC7, 0x4B),
    danger: Rgba::opaque(0xFF, 0x99, 0x91),
    ok: Rgba::opaque(0x6C, 0xC4, 0x6C),
};

impl Palette {
    /// 控件当前可用的文本色（置灰时换成 disabled_text）。
    #[must_use]
    pub const fn control_text(self, enabled: bool) -> Rgba {
        if enabled { self.text_primary } else { self.disabled_text }
    }

    /// 控件当前可用的填充色（置灰时换成 disabled_fill）。
    #[must_use]
    #[cfg(test)]
    pub const fn control_fill(self, on: bool, enabled: bool) -> Rgba {
        if !enabled {
            self.disabled_fill
        } else if on {
            self.accent
        } else {
            self.track_off
        }
    }
}

impl ThemeMode {
    /// 取对应调色板（`'static`，无分配）。
    #[must_use]
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::Light => &LIGHT,
            Self::Dark => &DARK,
        }
    }

    /// 探测系统深/浅色。
    ///
    /// 读 `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`：
    /// `1` = 浅色，`0`/缺失 = 深色（与 Windows 自身默认一致）。
    ///
    /// 刻意**不**用 `GetSysColor` / `SystemParametersInfo`——它们返回的是经典 Win32 颜色，
    /// 不反映 Win11 的深色模式。也刻意不用 `DwmGetWindowAttribute`：那是逐窗口的。
    #[must_use]
    pub fn system() -> Self {
        const SUBKEY: &str =
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: 两个字符串都是 NUL 结尾的 HSTRING，缓冲是本地 u32、大小已给出。
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(SUBKEY),
                &HSTRING::from("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut std::ffi::c_void),
                Some(&mut size),
            )
        };
        if status.is_ok() && value != 0 {
            Self::Light
        } else {
            Self::Dark
        }
    }
}

// ---------------------------------------------------------------------------
// 度量（全部 dip，§11.4：UI 链路只用 dip，px 换算只在 render.rs 边界做）
// ---------------------------------------------------------------------------

/// 页面标题区高度（**不是**窗口标题栏：窗口标题栏由 DWM 提供，
/// 因为 B3 要求它承载 `backdrop = 4` 的 Mica 材质，自绘会把它盖掉）。
pub const PAGE_HEADING_HEIGHT_DIP: f32 = 44.0;
/// 分组卡片圆角。
pub const CARD_RADIUS_DIP: f32 = 8.0;
/// 卡片 1 px 描边宽度。
pub const CARD_BORDER_DIP: f32 = 1.0;
/// 内容区外边距。
pub const CONTENT_MARGIN_DIP: f32 = 16.0;
/// 卡片内边距。
pub const CARD_PADDING_DIP: f32 = 16.0;
/// 正文字号。
pub const BODY_FONT_SIZE_DIP: f32 = 14.0;
/// 标题字号。
pub const TITLE_FONT_SIZE_DIP: f32 = 14.0;
/// 最小窗口尺寸（A-08：720×560 下不得出现横向滚动条；留 4 px 余量防 DPI 取整抖动）。
pub const MIN_WIDTH_DIP: f32 = 720.0;
/// 最小窗口高度。
pub const MIN_HEIGHT_DIP: f32 = 560.0;

// ---- 设置窗口（页签式布局）----

/// 头部高度：页签 + 状态 + 着色开关 + 暂停。
pub const HEADER_HEIGHT_DIP: f32 = 48.0;
/// 底栏高度：恢复默认 / 导入 / 导出 / 关于。
pub const FOOTER_HEIGHT_DIP: f32 = 48.0;
/// 内容区外边距。
pub const PAGE_PAD_DIP: f32 = 16.0;
/// 卡片之间、卡片与内容区之间的间距。
pub const PAGE_GAP_DIP: f32 = 12.0;
/// 一行设置的高度（整行可点）。
pub const ROW_HEIGHT_DIP: f32 = 44.0;
/// 开关轨道尺寸（视觉与命中一致）。
pub const TOGGLE_W_DIP: f32 = 36.0;
/// 开关轨道高度。
pub const TOGGLE_H_DIP: f32 = 18.0;
/// 卡片圆角。
pub const TILE_RADIUS_DIP: f32 = 8.0;

/// 正文字体族。
pub const FONT_FAMILY_BODY: &str = "Segoe UI Variable Text";
/// 标题/强调字体族。
pub const FONT_FAMILY_DISPLAY: &str = "Segoe UI Variable Display";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_color_is_fully_opaque() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let p = mode.palette();
            for c in [
                p.window_bg, p.heading_bg, p.card_bg, p.card_alt, p.card_border,
                p.divider, p.preview_wall, p.preview_bar, p.preview_line,
                p.preview_ink, p.preview_muted,
                p.text_primary, p.text_secondary, p.accent, p.accent_text,
                p.track_off, p.disabled_fill, p.disabled_text,
                p.warning, p.danger, p.ok,
            ] {
                assert_eq!(c.a, 255, "B3 定稿：客户区所有颜色 α 恒为 255");
            }
        }
    }

    #[test]
    fn control_helpers_switch_on_enabled() {
        let p = ThemeMode::Light.palette();
        assert_eq!(p.control_text(true), p.text_primary);
        assert_eq!(p.control_text(false), p.disabled_text);
        assert_eq!(p.control_fill(true, true), p.accent);
        assert_eq!(p.control_fill(false, true), p.track_off);
        assert_eq!(p.control_fill(true, false), p.disabled_fill);
    }

    #[test]
    fn palette_lookup_is_static_and_distinct() {
        assert_ne!(ThemeMode::Light.palette().window_bg, ThemeMode::Dark.palette().window_bg);
    }
}
