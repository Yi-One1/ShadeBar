//! 颜色类型（§4 #15 / §11.5 颜色空间的**唯一定义处**）。
//!
//! # 当前范围（T03.1 所需子集）
//!
//! 本文件此刻只提供 [`Rgba`] 与"sRGB straight α ↔ 预乘 f32"的转换。
//! `stats`（直方图灰阶）、`perceptual_y`、`contrast_ratio`（WCAG）等属 **T01.3** 的其余部分，
//! 会在该任务里补进本文件——**不另建文件**，因为 §11.5 规定颜色空间只在这里定义一次。
//!
//! # 约定（§11.5，实现时必须遵守）
//!
//! | 边界 | 格式 |
//! |---|---|
//! | 内部统一 | [`Rgba`]，**straight alpha + sRGB gamma 编码** |
//! | 配置文件 | `"#RRGGBB"`；需要 alpha 时 `"#AARRGGBB"` |
//! | XAML `Windows.UI.Color` | straight alpha ARGB，直接映射不需预乘 |
//! | Direct2D 画刷 | **预乘 alpha**，由 `render.rs` 统一从 [`Rgba`] 转换，其它模块不得自行转换 |
//!
//! 特别提醒：`perceptual_y`（感知亮度）与 `contrast_ratio`（WCAG 相对亮度，需先线性化）
//! **不可混用**，是两个函数（T01.3）。

/// 内部统一颜色：straight alpha（非预乘）+ sRGB gamma 编码。
///
/// 之所以坚持 straight alpha：配置、XAML、日志都直接可读；
/// 预乘只在"真正提交给 D2D 画刷"这一瞬间出现，且由 `render.rs` 单点转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// 红 0..=255（sRGB gamma 编码）。
    pub r: u8,
    /// 绿 0..=255。
    pub g: u8,
    /// 蓝 0..=255。
    pub b: u8,
    /// 不透明度 0..=255（0 = 全透明，255 = 不透明）。
    pub a: u8,
}

impl Rgba {
    /// 四通道构造。
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 不透明构造（α = 255）。
    #[must_use]
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// 从 `0xRRGGBB` 整数构造不透明色。
    #[must_use]
    pub const fn from_rgb(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xFF) as u8,
            g: ((hex >> 8) & 0xFF) as u8,
            b: (hex & 0xFF) as u8,
            a: 255,
        }
    }

    /// 解析 `"#RRGGBB"` 或 `"#AARRGGBB"`（大小写不敏感，`#` 可省略）。
    ///
    /// 解析失败返回 `None`——按 §11.5，调用点应"保留旧值 + 记日志"，不得回退成黑或白。
    #[must_use]
    pub fn parse_hex(text: &str) -> Option<Self> {
        let body = text.strip_prefix('#').unwrap_or(text);
        // 只接受纯 ASCII 十六进制，避免 `char_indices` 在 UTF-8 上错位。
        if !body.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(body, 16).ok()?;
        match body.len() {
            6 => Some(Self::from_rgb(value)),
            8 => Some(Self::new(
                ((value >> 16) & 0xFF) as u8,
                ((value >> 8) & 0xFF) as u8,
                (value & 0xFF) as u8,
                ((value >> 24) & 0xFF) as u8,
            )),
            _ => None,
        }
    }

    /// 转成 straight-alpha 的 `0.0..=1.0` 浮点四元组 `(r, g, b, a)`。
    ///
    /// Direct2D 的 `D2D1_COLOR_F` 需要的是**预乘**，故本函数只用于需要
    /// straight 语义的场合（如写 XAML `Windows.UI.Color`）。
    #[must_use]
    pub fn straight_f32(self) -> (f32, f32, f32, f32) {
        (
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
            f32::from(self.a) / 255.0,
        )
    }

    /// 转成**预乘 alpha** 的 `0.0..=1.0` 浮点四元组，供 Direct2D 画刷使用。
    ///
    /// 预乘公式：`c' = c × α`。α = 0 时 RGB 必须全部归零，否则 D2D 会画出
    /// 不该有的加色（这是"透明区域发亮"的经典成因）。
    #[must_use]
    pub fn premul_f32(self) -> (f32, f32, f32, f32) {
        let a = f32::from(self.a) / 255.0;
        (
            f32::from(self.r) / 255.0 * a,
            f32::from(self.g) / 255.0 * a,
            f32::from(self.b) / 255.0 * a,
            a,
        )
    }

    /// `#RRGGBB` 形式（丢弃 alpha），用于日志与诊断面板。
    #[must_use]
    pub fn to_hex_rgb(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rgb_splits_channels() {
        let c = Rgba::from_rgb(0x00_1A_2B_3C);
        assert_eq!((c.r, c.g, c.b, c.a), (0x1A, 0x2B, 0x3C, 255));
    }

    #[test]
    fn parse_hex_accepts_six_and_eight_digits() {
        assert_eq!(Rgba::parse_hex("#FFFFFF"), Some(Rgba::opaque(255, 255, 255)));
        assert_eq!(Rgba::parse_hex("000000"), Some(Rgba::opaque(0, 0, 0)));
        // #AARRGGBB：38 = α 56/255，即默认悬停块的 #38FFFFFF
        assert_eq!(
            Rgba::parse_hex("#38FFFFFF"),
            Some(Rgba::new(255, 255, 255, 0x38))
        );
    }

    #[test]
    fn parse_hex_rejects_bad_input() {
        assert_eq!(Rgba::parse_hex("#FFF"), None);
        assert_eq!(Rgba::parse_hex("#GGGGGG"), None);
        assert_eq!(Rgba::parse_hex("#FFFFFFF"), None);
        assert_eq!(Rgba::parse_hex(""), None);
        // 非 ASCII 不应 panic（曾因 char_indices 在 UTF-8 上错位踩过）
        assert_eq!(Rgba::parse_hex("#颜色颜色"), None);
    }

    #[test]
    fn premul_zeroes_rgb_when_transparent() {
        let (r, g, b, a) = Rgba::new(255, 255, 255, 0).premul_f32();
        assert_eq!((r, g, b, a), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn premul_matches_formula() {
        let (r, _g, _b, a) = Rgba::new(255, 0, 0, 128).premul_f32();
        assert!((a - 128.0 / 255.0).abs() < 1e-6);
        assert!((r - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn straight_keeps_rgb_regardless_of_alpha() {
        let (r, _, _, a) = Rgba::new(255, 0, 0, 0).straight_f32();
        assert!((r - 1.0).abs() < 1e-6);
        assert!(a.abs() < 1e-6);
    }

    #[test]
    fn hex_roundtrip_is_uppercase() {
        assert_eq!(Rgba::opaque(0xF3, 0xF3, 0xF3).to_hex_rgb(), "#F3F3F3");
    }
}
