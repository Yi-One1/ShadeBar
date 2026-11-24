//! 渲染上下文（§4 #27）。
//!
//! # B3 上屏方案（架构 v1.4 冻结规格，见 `docs/02-architecture.md` §2.2 D1 第 7 步）
//!
//! | 项 | 定稿 |
//! |---|---|
//! | 客户区 | **不透明纯色**，浅 `#F3F3F3` / 深 `#202020` |
//! | 标题栏 | DWM `backdrop = 4`（`DWMSBT_TABBEDWINDOW`）提供材质，由 `window.rs` 设置 |
//! | 上屏 | **`ID2D1HwndRenderTarget` 直接画到窗口**，`D2D1_ALPHA_MODE_IGNORE` |
//! | ⛔ 不做 | 自建 DIB 后缓冲、GDI `AlphaBlend` 上屏、`DwmExtendFrameIntoClientArea`、DirectComposition |
//!
//! # 为什么这条路径不会出现 α 累积
//!
//! 两条成立的理由（**均须可验证；历史上有过被撤回的错误论据，不得再引用**）：
//!
//! 1. **DWM backdrop 不进客户区**——干净环境下客户区像素与"同区域无窗口基线"逐字节相同，
//!    所以"半透明内容叠在 Mica 上"在客户区本来就不可达；
//! 2. **`WM_ERASEBKGND→TRUE` + 逐帧 `AlphaBlend` 会让不透明度按 `1 − 0.3ⁿ` 累积**。
//!
//! 本实现不碰 α：HWND 渲染目标天然不透明（`ALPHA_MODE_IGNORE`），
//! 且每帧先铺满整幅不透明背景再画内容 —— 永远是"从干净表面重新合成"，不存在跨帧叠加。
//!
//! # 单位纪律（§11.4）
//!
//! 对外接口的参数**全部是 dip**；像素换算只在 [`RenderCtx::px`] 一处发生。
//!
//! # 设备丢失（`D2DERR_RECREATE_TARGET`）
//!
//! HWND 渲染目标是"有状态"的：GPU 复位 / 显示器切换 / RDP 断连时 `EndDraw` 会返回
//! `D2DERR_RECREATE_TARGET`，此时必须重建目标并重绘，否则窗口会一直空白。
//! [`RenderCtx::end_frame`] 用返回值表达这件事，由 `window.rs` 重新触发重绘。

use shadebar_core::color::Rgba;
use shadebar_core::{SbError, SbResult};
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
    D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_ALIGNMENT,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::ui::theme::{self, ThemeMode};

/// `D2DERR_RECREATE_TARGET`（0x8899000C）：渲染目标需要重建。
const D2DERR_RECREATE_TARGET: i32 = 0x8899_000C_u32 as i32;

/// 以 dip 表述的矩形。渲染上下文对外**只接受**这个类型。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectDip {
    /// 左上角 x。
    pub x: f32,
    /// 左上角 y。
    pub y: f32,
    /// 宽。
    pub w: f32,
    /// 高。
    pub h: f32,
}

impl RectDip {
    /// 构造。
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// 文本角色。字号与字重由角色决定，调用方不直接给字号（避免出现 13.5 px 这类漂移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// 页面/对话框标题（15 px Semibold）。
    Title,
    /// 小节标题与我强调的行标签（13 px Semibold）。
    Subtitle,
    /// 正文行标签（12.5 px）。
    Body,
    /// 辅助说明、单位、数值（11 px）。
    Caption,
    /// 大号数值（22 px Semibold，诊断页的 Y / 对比度）。
    Metric,
}

impl TextRole {
    /// 该角色的字号（dip）。布局宽度必须按这个值量，否则 12.0 量、12.5 画
    /// 会把「恢复默认」截成「恢复默…」。
    #[must_use]
    pub const fn size(self) -> f32 {
        match self {
            Self::Title => 15.0,
            Self::Subtitle => 13.0,
            Self::Body => 12.5,
            Self::Caption => 11.0,
            Self::Metric => 22.0,
        }
    }
}

/// dip → 像素的唯一换算点（§11.4）。
///
/// 之所以独立成函数而不是写在方法里：单测与布局计算都要用同一个公式，
/// 两处各写一遍必然漂移。
#[must_use]
pub fn dip_to_px(dpi: u32, dip: f32) -> f32 {
    dip * (dpi as f32) / 96.0
}

/// 渲染上下文：D2D 工厂 + HWND 渲染目标 + DWrite。
pub struct RenderCtx {
    /// 设备丢失后重建目标时要用，故必须留着。
    factory: ID2D1Factory,
    target: ID2D1HwndRenderTarget,
    hwnd: HWND,
    dwrite: IDWriteFactory,
    formats: TextFormats,
    dpi: u32,
    /// 每 dip 的像素数，`dpi / 96`。
    scale: f32,
    /// 当前目标的后备缓冲尺寸（像素），用于判断是否需要 `Resize`。
    size_px: (u32, u32),
}

impl RenderCtx {
    /// 建立渲染上下文。
    ///
    /// `size_px` 取创建窗口时的客户区尺寸；为 0 时按 1×1 建，首帧 `begin_frame` 会 `Resize`。
    ///
    /// # Errors
    ///
    /// D2D 工厂、HWND 渲染目标、DWrite 工厂或文本格式创建失败时返回 [`SbError::HResult`]。
    pub fn new(hwnd: HWND, dpi: u32, size_px: (u32, u32)) -> SbResult<Self> {
        // SAFETY: 单线程工厂，无额外选项。
        let factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }.map_err(|e| {
                SbError::HResult {
                    op: "D2D1CreateFactory",
                    hr: e.code().0,
                }
            })?;

        // SAFETY: 共享工厂，无额外参数。
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .map_err(|e| SbError::HResult {
                op: "DWriteCreateFactory",
                hr: e.code().0,
            })?;

        let scale = dpi as f32 / 96.0;
        let formats = TextFormats::build(&dwrite, scale)?;

        let size_px = (size_px.0.max(1), size_px.1.max(1));
        let target = create_hwnd_target(&factory, hwnd, size_px)?;

        Ok(Self {
            factory,
            target,
            hwnd,
            dwrite,
            formats,
            dpi,
            scale,
            size_px,
        })
    }

    /// dip → 像素（本模块唯一换算点）。
    #[must_use]
    pub fn px(&self, dip: f32) -> f32 {
        dip * self.scale
    }

    /// 更换 DPI：重建文本格式（字号按新 scale 折算）。
    ///
    /// # Errors
    ///
    /// 文本格式重建失败时返回 [`SbError::HResult`]。
    pub fn set_dpi(&mut self, dpi: u32) -> SbResult<()> {
        if dpi == self.dpi {
            return Ok(());
        }
        self.scale = dpi as f32 / 96.0;
        self.dpi = dpi;
        self.formats = TextFormats::build(&self.dwrite, self.scale)?;
        Ok(())
    }

    /// 开始一帧：必要时 `Resize` 后备缓冲，然后 `BeginDraw`。
    ///
    /// # Errors
    ///
    /// 尺寸非正或 `Resize` 失败时返回错误。
    pub fn begin_frame(&mut self, width_px: i32, height_px: i32) -> SbResult<()> {
        if width_px <= 0 || height_px <= 0 {
            return Err(SbError::SampleRegionTooSmall { valid_px: 0 });
        }
        let want = (width_px as u32, height_px as u32);
        if want != self.size_px {
            // SAFETY: 尺寸来自 GetClientRect，已判正。
            unsafe {
                self.target.Resize(&D2D_SIZE_U {
                    width: want.0,
                    height: want.1,
                })
            }
            .map_err(|e| SbError::HResult {
                op: "ID2D1HwndRenderTarget::Resize",
                hr: e.code().0,
            })?;
            self.size_px = want;
        }
        // SAFETY: 目标有效。
        unsafe { self.target.BeginDraw() };
        Ok(())
    }

    /// 不透明填充矩形。
    pub fn fill_rect(&self, rect: RectDip, color: Rgba) {
        let Some(brush) = self.solid(color) else {
            return;
        };
        let r = self.to_d2d_rect(rect);
        // SAFETY: 处于 BeginDraw/EndDraw 之间，brush 与矩形均有效。
        unsafe { self.target.FillRectangle(&r, &brush) };
    }

    /// 画一条抗锯齿线段（dip）。小图形必须走矢量线：用若干像素方块拼出来的
    /// 斜边在 125%/150% 缩放下会变成锯齿块，看起来"糊"且不像箭头。
    pub fn draw_line(&self, x1: f32, y1: f32, x2: f32, y2: f32, width_dip: f32, color: Rgba) {
        let Some(brush) = self.solid(color) else {
            return;
        };
        let p0 = D2D_POINT_2F { x: self.px(x1), y: self.px(y1) };
        let p1 = D2D_POINT_2F { x: self.px(x2), y: self.px(y2) };
        // SAFETY: 处于 BeginDraw/EndDraw 之间；默认线帽为平头，坐标由调用方保证。
        unsafe { self.target.DrawLine(p0, p1, &brush, self.px(width_dip), None) };
    }

    /// 不透明填充圆角矩形。
    pub fn fill_rounded_rect(&self, rect: RectDip, radius_dip: f32, color: Rgba) {
        let Some(brush) = self.solid(color) else {
            return;
        };
        let rr = self.to_rounded(rect, radius_dip);
        // SAFETY: 同上。
        unsafe { self.target.FillRoundedRectangle(&rr, &brush) };
    }

    /// 描边圆角矩形。
    pub fn stroke_rounded_rect(&self, rect: RectDip, radius_dip: f32, color: Rgba, width_dip: f32) {
        let Some(brush) = self.solid(color) else {
            return;
        };
        let rr = self.to_rounded(rect, radius_dip);
        // SAFETY: 同上；stroke style 传 None 使用默认实线。
        unsafe {
            self.target
                .DrawRoundedRectangle(&rr, &brush, self.px(width_dip), None)
        };
    }

    /// 画一行文本（单行、不换行，超出矩形直接裁切）。
    pub fn draw_text(&self, text: &str, rect: RectDip, role: TextRole, color: Rgba) {
        self.draw_in(text, rect, self.formats.of(role), color);
    }

    /// 右对齐画一行文本（数值列）。只有 `Subtitle`/`Body`/`Caption` 需要它。
    pub fn draw_text_right(&self, text: &str, rect: RectDip, role: TextRole, color: Rgba) {
        self.draw_in(text, rect, self.formats.of_right(role), color);
    }

    /// 水平居中画一行文本（胶囊 / 分段 / 按钮）。
    pub fn draw_text_center(&self, text: &str, rect: RectDip, role: TextRole, color: Rgba) {
        self.draw_in(text, rect, self.formats.of_center(role), color);
    }

    fn draw_in(&self, text: &str, rect: RectDip, format: &IDWriteTextFormat, color: Rgba) {
        let Some(brush) = self.solid(color) else {
            return;
        };
        let utf16: Vec<u16> = text.encode_utf16().collect();
        if utf16.is_empty() {
            return;
        }
        let r = self.to_d2d_rect(rect);
        // SAFETY: 目标处于绘制期间；utf16 在本次调用内存活。
        unsafe {
            self.target.DrawText(
                &utf16,
                format,
                &r,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// 结束一帧。
    ///
    /// 返回 `Ok(true)` 表示**渲染目标已重建、需要立刻整体重绘一次**
    /// （发生了 `D2DERR_RECREATE_TARGET`）；调用方应 `InvalidateRect` 后重画。
    ///
    /// # Errors
    ///
    /// `EndDraw` 返回除"需要重建"以外的失败时返回 [`SbError::HResult`]。
    pub fn end_frame(&mut self) -> SbResult<bool> {
        // SAFETY: 与 BeginDraw 配对。
        match unsafe { self.target.EndDraw(None, None) } {
            Ok(()) => Ok(false),
            Err(e) if e.code().0 == D2DERR_RECREATE_TARGET => {
                // 设备丢失：按当前尺寸重建目标，并要求调用方重绘。
                self.target = create_hwnd_target(&self.factory, self.hwnd, self.size_px)?;
                Ok(true)
            }
            Err(e) => Err(SbError::HResult {
                op: "ID2D1RenderTarget::EndDraw",
                hr: e.code().0,
            }),
        }
    }

    /// 建一支不透明画刷。`α < 255` 会被断言为设计违约（B3 阶段不应出现）。
    fn solid(&self, color: Rgba) -> Option<ID2D1SolidColorBrush> {
        debug_assert_eq!(
            color.a, 255,
            "B3 定稿为不透明客户区；出现 α<255 说明设计方案被改动，先改 theme.rs 的约定"
        );
        let (r, g, b, a) = color.premul_f32();
        let d2d = D2D1_COLOR_F { r, g, b, a };
        // CreateSolidColorBrush 定义在 ID2D1RenderTarget 上，HWND 目标需先上转。
        let rt: ID2D1RenderTarget = self.target.cast().ok()?;
        // SAFETY: 目标处于绘制期间；颜色与（默认）画刷属性均有效。
        unsafe { rt.CreateSolidColorBrush(&d2d, None) }.ok()
    }

    fn to_d2d_rect(&self, r: RectDip) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.px(r.x),
            top: self.px(r.y),
            right: self.px(r.x + r.w),
            bottom: self.px(r.y + r.h),
        }
    }

    fn to_rounded(&self, r: RectDip, radius_dip: f32) -> D2D1_ROUNDED_RECT {
        let radius = self.px(radius_dip);
        D2D1_ROUNDED_RECT {
            rect: self.to_d2d_rect(r),
            radiusX: radius,
            radiusY: radius,
        }
    }
}

/// 创建一个与窗口绑定的不透明渲染目标。
fn create_hwnd_target(
    factory: &ID2D1Factory,
    hwnd: HWND,
    size_px: (u32, u32),
) -> SbResult<ID2D1HwndRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        // 软件渲染：设置页是一帧静态界面，硬件加速没有收益，
        // 而 `D2D1_RENDER_TARGET_TYPE_DEFAULT` 会拉起整条 D3D11/DXGI 设备栈，
        // 实测常驻多占 36 MB 且释放后不归还（NFR-02 要求 ≤20 MB）。
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            // HWND 目标本身就是不透明表面；显式写 IGNORE 让"绝不碰窗口表面 alpha"
            // 这条设计约束在代码里可见。
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U {
            width: size_px.0.max(1),
            height: size_px.1.max(1),
        },
        // NONE：由 D2D 决定呈现时机（默认在 EndDraw 时呈现）。
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    // SAFETY: 两个属性结构都已完整初始化，hwnd 有效。
    let target = unsafe { factory.CreateHwndRenderTarget(&props, &hwnd_props) }.map_err(|e| {
        SbError::HResult {
            op: "ID2D1Factory::CreateHwndRenderTarget",
            hr: e.code().0,
        }
    })?;
    // 不透明表面上没有彩边问题，但仍固定灰度抗锯齿：与 SP-1 结论保持一致，
    // 且将来若某块区域改成带 α 的合成也不会突然出彩边。
    // SAFETY: target 有效。
    unsafe {
        let rt: ID2D1RenderTarget = target.cast().map_err(|e| SbError::HResult {
            op: "QI ID2D1RenderTarget",
            hr: e.code().0,
        })?;
        rt.SetTextAntialiasMode(
            windows::Win32::Graphics::Direct2D::D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
        );
    }
    Ok(target)
}

/// 全部文本格式。字号/字重集中在这里，`View` 只说角色。
struct TextFormats {
    title: IDWriteTextFormat,
    subtitle: IDWriteTextFormat,
    body: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    metric: IDWriteTextFormat,
    subtitle_r: IDWriteTextFormat,
    body_r: IDWriteTextFormat,
    caption_r: IDWriteTextFormat,
    subtitle_c: IDWriteTextFormat,
    body_c: IDWriteTextFormat,
    caption_c: IDWriteTextFormat,
}

impl TextFormats {
    fn build(dwrite: &IDWriteFactory, scale: f32) -> SbResult<Self> {
        use theme::{FONT_FAMILY_BODY as BODY, FONT_FAMILY_DISPLAY as DISPLAY};
        Ok(Self {
            title: create_text_format(dwrite, DISPLAY, 15.0 * scale, true, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            subtitle: create_text_format(dwrite, BODY, 13.0 * scale, true, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            body: create_text_format(dwrite, BODY, 12.5 * scale, false, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            caption: create_text_format(dwrite, BODY, 11.0 * scale, false, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            metric: create_text_format(dwrite, DISPLAY, 22.0 * scale, true, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            subtitle_r: create_text_format(dwrite, BODY, 13.0 * scale, true, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            body_r: create_text_format(dwrite, BODY, 12.5 * scale, false, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            caption_r: create_text_format(dwrite, BODY, 11.0 * scale, false, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            subtitle_c: create_text_format(dwrite, BODY, 13.0 * scale, true, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            body_c: create_text_format(dwrite, BODY, 12.5 * scale, false, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            caption_c: create_text_format(dwrite, BODY, 11.0 * scale, false, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        })
    }

    fn of(&self, role: TextRole) -> &IDWriteTextFormat {
        match role {
            TextRole::Title => &self.title,
            TextRole::Subtitle => &self.subtitle,
            TextRole::Body => &self.body,
            TextRole::Caption => &self.caption,
            TextRole::Metric => &self.metric,
        }
    }

    fn of_right(&self, role: TextRole) -> &IDWriteTextFormat {
        match role {
            TextRole::Subtitle => &self.subtitle_r,
            TextRole::Body => &self.body_r,
            TextRole::Caption => &self.caption_r,
            other => self.of(other),
        }
    }

    fn of_center(&self, role: TextRole) -> &IDWriteTextFormat {
        match role {
            TextRole::Subtitle => &self.subtitle_c,
            TextRole::Body => &self.body_c,
            TextRole::Caption => &self.caption_c,
            other => self.of(other),
        }
    }
}

fn create_text_format(
    dwrite: &IDWriteFactory,
    family: &str,
    size_px: f32,
    bold: bool,
    align: DWRITE_TEXT_ALIGNMENT,
) -> SbResult<IDWriteTextFormat> {
    let weight = if bold {
        DWRITE_FONT_WEIGHT_SEMI_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    // SAFETY: 字体族/区域名字符串在调用期间存活。
    let format = unsafe {
        dwrite.CreateTextFormat(
            &HSTRING::from(family),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size_px,
            &HSTRING::from("zh-CN"),
        )
    }
    .map_err(|e| SbError::HResult {
        op: "IDWriteFactory::CreateTextFormat",
        hr: e.code().0,
    })?;
    // SAFETY: format 刚创建，未被共享。
    unsafe {
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP).ok();
        format.SetTextAlignment(align).ok();
    }
    Ok(format)
}

/// T03.1 的**渲染骨架帧**：证明"B3 上屏链路"完整可用。
///
/// 画三样东西，每一件都对应一条验收判据：
/// * 整幅不透明背景 → 验证"客户区纯色可见"、且不透明绘制不会被 backdrop 的底遮住；
/// * 页面标题文字 → 验证 14 px Semibold 文本在纯色底上无彩边；
/// * 一张圆角卡片 + 两行正文 → 验证 8 dip 圆角、1 px 描边与次要文本色。
///
/// **注意这里没有"自绘窗口标题栏"**：B3 要求窗口标题栏承载 DWM 的 `backdrop = 4` 材质，
/// 一旦把非客户区归零、自绘标题栏，就会把那块材质盖掉。
/// 因此本实现保留系统标题栏，客户区从标题栏下方开始（与资源管理器/设置应用一致）。
///
/// T03.2/T03.4 已用 `ui::View` 的正式布局替换本函数作为重绘入口。
/// 本函数**保留**为 T03.1「B3 上屏链路」的可复现最小参照（改主题/改 D2D 时
/// 可用它做一帧对照），故显式允许 dead_code；它不是占位符，代码完整可用。
#[allow(dead_code)]
pub fn paint_skeleton_frame(ctx: &RenderCtx, client_dip: (f32, f32), mode: ThemeMode) {
    let palette = mode.palette();
    let (w, h) = client_dip;

    // 1) 整幅不透明背景（每帧覆盖整幅 → 天然无跨帧累积）
    ctx.fill_rect(RectDip::new(0.0, 0.0, w, h), palette.window_bg);

    // 2) 页面标题区（窗口标题栏由 DWM 画在客户区之外，这里不重复）
    let margin = theme::CONTENT_MARGIN_DIP;
    ctx.fill_rect(
        RectDip::new(0.0, 0.0, w, theme::PAGE_HEADING_HEIGHT_DIP),
        palette.heading_bg,
    );
    ctx.draw_text(
        "随影 ShadeBar — 设置",
        RectDip::new(
            margin,
            (theme::PAGE_HEADING_HEIGHT_DIP - theme::TITLE_FONT_SIZE_DIP) / 2.0,
            (w - margin * 2.0).max(0.0),
            theme::TITLE_FONT_SIZE_DIP + 4.0,
        ),
        TextRole::Title,
        palette.text_primary,
    );

    // 3) 一张分组卡片
    let card_top = theme::PAGE_HEADING_HEIGHT_DIP + margin;
    let card = RectDip::new(
        margin,
        card_top,
        (w - margin * 2.0).max(0.0),
        (h - card_top - margin).max(0.0),
    );
    ctx.fill_rounded_rect(card, theme::CARD_RADIUS_DIP, palette.card_bg);
    ctx.stroke_rounded_rect(
        card,
        theme::CARD_RADIUS_DIP,
        palette.card_border,
        theme::CARD_BORDER_DIP,
    );

    let pad = theme::CARD_PADDING_DIP;
    ctx.draw_text(
        "B3 上屏链路：客户区不透明纯色 + 标题栏 Mica(backdrop=4)",
        RectDip::new(
            card.x + pad,
            card.y + pad,
            (card.w - pad * 2.0).max(0.0),
            theme::BODY_FONT_SIZE_DIP + 6.0,
        ),
        TextRole::Body,
        palette.text_primary,
    );
    ctx.draw_text(
        "上屏：ID2D1HwndRenderTarget（α=IGNORE），无自建 DIB、无 AlphaBlend、无 DComp",
        RectDip::new(
            card.x + pad,
            card.y + pad + 26.0,
            (card.w - pad * 2.0).max(0.0),
            theme::BODY_FONT_SIZE_DIP + 6.0,
        ),
        TextRole::Body,
        palette.text_secondary,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dip_to_px_at_96_is_identity() {
        assert!((dip_to_px(96, 11.0) - 11.0).abs() < 1e-6);
    }

    #[test]
    fn dip_to_px_scales_linearly() {
        assert!((dip_to_px(144, 10.0) - 15.0).abs() < 1e-6);
        assert!((dip_to_px(192, 10.0) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn min_window_size_meets_acceptance() {
        // A-08：720×560 是验收下限，theme 的常量不得低于它。
        // 用运行时索引取值，避免编译期常量断言（clippy::assertions_on_constants）。
        let limits = [theme::MIN_WIDTH_DIP, theme::MIN_HEIGHT_DIP];
        let required = [720.0_f32, 560.0_f32];
        assert!(
            limits[0] >= required[0] && limits[1] >= required[1],
            "A-08 最小窗口 720×560 dip：实际 {:?}",
            limits
        );
    }

    #[test]
    fn palette_is_fully_opaque() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let p = mode.palette();
            for c in [
                p.window_bg,
                p.heading_bg,
                p.card_bg,
                p.card_border,
                p.text_primary,
                p.text_secondary,
            ] {
                assert_eq!(
                    c.a, 255,
                    "B3 定稿为不透明客户区，调色板不得含 α<255 的颜色"
                );
            }
        }
    }
}
