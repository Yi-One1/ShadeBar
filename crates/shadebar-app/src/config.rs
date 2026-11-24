//! 35 项用户配置（需求基线 v1.4 §6 / 架构 §8.3 T03.4）。
//!
//! # 设计
//!
//! * 一个扁平的 `[i64; 35]` 值表 + [`SettingKey`] 枚举索引。控件、布局、判定、
//!   托盘全部经同一张表绑定，避免 35 处各写一份默认值/范围（T03.4 判据①）。
//! * 值的物理含义由 [`SettingMeta`] 描述：布尔 0/1；下拉为选项索引；滑块为
//!   整数（亮度/EWMA 用千分比，避免浮点进 JSON）；颜色为 `0xAARRGGBB`。
//! * 配置与文案、UI 逻辑分离：文案在 `ui/i18n.rs` + `resources/lang/*.tsv`，
//!   本文件只存数值。
//! * 落盘为单个 JSON 文件 `%LOCALAPPDATA%\ShadeBar\config.json`。解析失败
//!   （手工写入非法 JSON / 字段越界）时回退默认值并把损坏文件改名为
//!   `config.json.bak`（NFR-16 / A-11），绝不崩溃。
//! * [`Config::to_engine_config`] 是设置与 core 引擎之间**唯一**的换算点，
//!   每次设置变更递增 `version`，core `Engine::tick` 同拍比对生效（A-10）。

use std::path::{Path, PathBuf};

use shadebar_core::color::Rgba;
use shadebar_core::engine::{EngineConfig, RegionCfg};
use shadebar_core::{SbError, SbResult};

use crate::ui::i18n;

/// 设置项总数（§6 口径，S-35/S-37 的 ARGB alpha 滑块是控件内置能力，不另计数）。
pub const SETTING_COUNT: usize = 49;

/// 设置项编号。顺序与 §6 表格一致，`as usize` 即值表下标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    // 6.1 通用（6）
    S01,
    S02,
    S03,
    S04,
    S05,
    S06,
    // 6.2 配色与采样（11）
    S10,
    S11,
    S12,
    S13,
    S14,
    S15,
    S16,
    S17,
    S18,
    S19,
    // 6.3 采样区域微调（4）
    S20,
    S21,
    S22,
    S23,
    // 6.4 着色元素（8）
    S30,
    S31,
    S32,
    S33,
    S34,
    S35,
    S36,
    S37,
    // 6.5 托盘与行为（7）
    S40,
    S41,
    S42,
    S43,
    S44,
    S45,
    S46,
    // 6.6 判定粒度（1）—— 新增项一律追加在末尾，
    // 保证 `as usize` 的既有下标不漂移，旧 config.json 仍能按原位置读回。
    S47,
    // 6.7 分区自适应开关（3，S-48 ~ S-50）—— 关掉某区即该区元素写回系统原色。
    S48,
    S49,
    S50,
    // 6.8 托盘左键双击（1，S-51）—— 选项集与 S-40 一致，默认"无操作"。
    S51,
    // 6.9 分区独立配色（9，S-52 ~ S-60）—— 仅在"分区独立"模式下生效。
    // 4 项内容 × 2 背景态；S-60 是正常态指示条的不透明度（两种模式都生效）。
    S52,
    S53,
    S54,
    S55,
    S56,
    S57,
    S58,
    S59,
    S60,
}

impl SettingKey {
    /// 全部 49 项，按 §6 顺序。
    pub const ALL: [Self; SETTING_COUNT] = [
        Self::S01, Self::S02, Self::S03, Self::S04, Self::S05, Self::S06,
        Self::S10, Self::S11, Self::S12, Self::S13, Self::S14, Self::S15, Self::S16,
        Self::S17, Self::S18, Self::S19,
        Self::S20, Self::S21, Self::S22, Self::S23,
        Self::S30, Self::S31, Self::S32, Self::S33, Self::S34, Self::S35,
        Self::S36, Self::S37,
        Self::S40, Self::S41, Self::S42, Self::S43, Self::S44, Self::S45, Self::S46,
        Self::S47,
        Self::S48, Self::S49, Self::S50, Self::S51,
        Self::S52, Self::S53, Self::S54, Self::S55, Self::S56, Self::S57,
        Self::S58, Self::S59, Self::S60,
    ];

    /// 值表下标。
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 需求编号字符串（`S-01` 等）。
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::S01 => "S-01", Self::S02 => "S-02", Self::S03 => "S-03",
            Self::S04 => "S-04", Self::S05 => "S-05", Self::S06 => "S-06",
            Self::S10 => "S-10", Self::S11 => "S-11", Self::S12 => "S-12",
            Self::S13 => "S-13", Self::S14 => "S-14", Self::S15 => "S-15",
            Self::S16 => "S-16", Self::S17 => "S-17", Self::S18 => "S-18",
            Self::S19 => "S-19", Self::S20 => "S-20", Self::S21 => "S-21",
            Self::S22 => "S-22", Self::S23 => "S-23", Self::S30 => "S-30",
            Self::S31 => "S-31", Self::S32 => "S-32", Self::S33 => "S-33",
            Self::S34 => "S-34", Self::S35 => "S-35", Self::S36 => "S-36",
            Self::S37 => "S-37", Self::S40 => "S-40", Self::S41 => "S-41",
            Self::S42 => "S-42", Self::S43 => "S-43", Self::S44 => "S-44",
            Self::S45 => "S-45", Self::S46 => "S-46", Self::S47 => "S-47",
            Self::S48 => "S-48", Self::S49 => "S-49", Self::S50 => "S-50",
            Self::S51 => "S-51",
            Self::S52 => "S-52", Self::S53 => "S-53", Self::S54 => "S-54",
            Self::S55 => "S-55", Self::S56 => "S-56", Self::S57 => "S-57",
            Self::S58 => "S-58", Self::S59 => "S-59", Self::S60 => "S-60",
        }
    }

    /// 控件类型与范围元数据。
    #[must_use]
    pub const fn meta(self) -> SettingMeta {
        use Group as G;
        match self {
            Self::S01 => SettingMeta::toggle(G::General, "s01", "s01.help", 1),
            Self::S02 => SettingMeta::toggle(G::General, "s02", "s02.help", 0),
            Self::S03 => SettingMeta::toggle(G::General, "s03", "s03.help", 1),
            Self::S04 => SettingMeta::dropdown(G::General, "s04", "s04.help", 0, 3, i18n::OPT_LANG),
            Self::S05 => SettingMeta::readonly(G::General, "s05", "s05.help"),
            Self::S06 => SettingMeta::button(G::General, "s06", "s06.help"),
            Self::S10 => SettingMeta::dropdown(G::Color, "s10", "s10.help", 0, 3, i18n::OPT_ADAPTIVE),
            Self::S11 => SettingMeta::toggle(G::Color, "s11", "s11.help", 1),
            Self::S12 => SettingMeta::slider(G::Color, "s12", "s12.help", 500, 300, 700, 10),
            Self::S13 => SettingMeta::dropdown(G::Color, "s13", "s13.help", 0, 2, i18n::OPT_STRATEGY),
            Self::S14 => SettingMeta::color(G::Color, "s14", "s14.help", 0xFFFF_FFFF),
            Self::S15 => SettingMeta::color(G::Color, "s15", "s15.help", 0xFF00_0000),
            Self::S16 => SettingMeta::slider(G::Color, "s16", "s16.help", 1000, 500, 5000, 100),
            Self::S17 => SettingMeta::slider(G::Color, "s17", "s17.help", 3, 1, 20, 1),
            Self::S18 => SettingMeta::slider(G::Color, "s18", "s18.help", 300, 50, 1000, 50),
            Self::S19 => SettingMeta::toggle(G::Color, "s19", "s19.help", 0),
            // 判定粒度：0 = 分区独立（默认），1 = 全任务栏统一
            Self::S47 => SettingMeta::dropdown(G::Color, "s47", "s47.help", 0, 2, i18n::OPT_JUDGE_MODE),
            // S-48~S-50：分区自适应开关。关掉某区 = 该区元素写回系统原色，
            // 不是"停止判定"——判定照常跑，只是不下发该区的颜色。
            Self::S48 => SettingMeta::toggle(G::Region, "s48", "s48.help", 1),
            Self::S49 => SettingMeta::toggle(G::Region, "s49", "s49.help", 1),
            Self::S50 => SettingMeta::toggle(G::Region, "s50", "s50.help", 1),
            Self::S20 => SettingMeta::slider(G::Region, "s20", "s20.help", 0, 0, 50, 1),
            // 上限 84 = 托盘区起点（recolor::ICONS_SPAN_MAX）。以前写 100 只是
            // 在采样侧再卡回 84，滑杆却显示 0~100%，与实际生效范围不一致。
            Self::S21 => SettingMeta::slider(G::Region, "s21", "s21.help", 84, 10, 84, 1),
            Self::S22 => SettingMeta::slider(G::Region, "s22", "s22.help", 0, 0, 45, 1),
            Self::S23 => SettingMeta::readonly(G::Region, "s23", "s23.help"),
            Self::S30 => SettingMeta::toggle(G::Elements, "s30", "s30.help", 1),
            Self::S31 => SettingMeta::toggle(G::Elements, "s31", "s31.help", 1),
            Self::S32 => SettingMeta::toggle(G::Elements, "s32", "s32.help", 1),
            Self::S33 => SettingMeta::toggle(G::Elements, "s33", "s33.help", 1),
            Self::S34 => SettingMeta::dropdown(G::Elements, "s34", "s34.help", 0, 2, i18n::OPT_SOURCE),
            // §13-2 决策：S-35/S-37 默认 #38FFFFFF（alpha 0x38 ≈ 22%），
            // 与 §6 表格的 #FFFFFF 不一致时以架构 §13-2 / T03.4 判据为准。
            Self::S35 => SettingMeta::color(G::Elements, "s35", "s35.help", 0x38FF_FFFF),
            Self::S36 => SettingMeta::dropdown(G::Elements, "s36", "s36.help", 0, 2, i18n::OPT_SOURCE),
            Self::S37 => SettingMeta::color(G::Elements, "s37", "s37.help", 0x38FF_FFFF),
            Self::S40 => SettingMeta::dropdown(G::Tray, "s40", "s40.help", 0, 3, i18n::OPT_LEFT_CLICK),
            // 左键双击：选项与 S-40 完全一致，默认"无操作"以免改变既有单击手感。
            Self::S51 => SettingMeta::dropdown(G::Tray, "s51", "s51.help", 2, 3, i18n::OPT_LEFT_CLICK),
            // 分区独立配色：4 项内容 × 2 背景态。偏暗 = 背景偏暗时用（浅色），
            // 偏亮 = 背景偏亮时用（深色）。
            Self::S52 => SettingMeta::color(G::Color, "s52", "s52.help", 0xFFFF_FFFF),
            Self::S53 => SettingMeta::color(G::Color, "s53", "s53.help", 0xFF00_0000),
            Self::S54 => SettingMeta::color(G::Color, "s54", "s54.help", 0xFFFF_FFFF),
            Self::S55 => SettingMeta::color(G::Color, "s55", "s55.help", 0xFF00_0000),
            Self::S56 => SettingMeta::color(G::Color, "s56", "s56.help", 0xFFFF_FFFF),
            Self::S57 => SettingMeta::color(G::Color, "s57", "s57.help", 0xFF00_0000),
            Self::S58 => SettingMeta::color(G::Color, "s58", "s58.help", 0xFFFF_FFFF),
            Self::S59 => SettingMeta::color(G::Color, "s59", "s59.help", 0xFF00_0000),
            Self::S60 => SettingMeta::slider(G::Color, "s60", "s60.help", 45, 0, 100, 1),
            Self::S41 => SettingMeta::dropdown(G::Tray, "s41", "s41.help", 0, 2, i18n::OPT_CLOSE),
            Self::S42 => SettingMeta::toggle(G::Tray, "s42", "s42.help", 0),
            Self::S43 => SettingMeta::toggle(G::Tray, "s43", "s43.help", 1),
            Self::S44 => SettingMeta::dropdown(G::Tray, "s44", "s44.help", 0, 3, i18n::OPT_INJECT_FAIL),
            Self::S45 => SettingMeta::dropdown(G::Behavior, "s45", "s45.help", 2, 4, i18n::OPT_LOG_LEVEL),
            Self::S46 => SettingMeta::button(G::Behavior, "s46", "s46.help"),
        }
    }

    /// 是否在设置页显示。
    ///
    /// 设置窗口只保留"用户真的会调、且当前版本真的生效"的 20 项。
    /// 其余项**只从 UI 收起，配置字段与下标完全不动**，旧 `config.json` 可平滑迁移：
    ///
    /// | 键 | 收起原因 |
    /// |---|---|
    /// | S-05 / S-06 | 版本号与恢复默认改为诊断页/底栏的固定动作 |
    /// | S-11 | 与 S-42 语义重复，顶栏「暂停」统一写 S-42 |
    /// | S-13 | 用户改端色时自动切到自定义，无需再暴露策略 |
    /// | S-17 | 变化阈值属于实现细节，行为由平滑系数决定 |
    /// | S-19 | 主题联动未落地 |
    /// | S-23 | 改为采样页的实时色块，不再是一行只读值 |
    /// | S-30 ~ S-37 | 由三区开关（S-48~S-50）取代，细粒度元素不再单列 |
    /// | S-42 | 由顶栏「暂停」承载 |
    /// | S-43 / S-44 | 无视觉表现 / 未落地 |
    /// | S-46 | 改为诊断页的「导出诊断包」动作 |
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub const fn ui_visible(self) -> bool {
        !matches!(
            self,
            Self::S05 | Self::S06 | Self::S11 | Self::S13 | Self::S14 | Self::S15
                | Self::S17 | Self::S19 | Self::S23 | Self::S30 | Self::S31 | Self::S32
                | Self::S33 | Self::S34 | Self::S35 | Self::S36 | Self::S37 | Self::S42
                | Self::S43 | Self::S44 | Self::S46
        )
    }

    /// 颜色项的值规范化（写入路径 [`Config::set`] 与加载路径 [`Config::sanitize`] 共用）。
    ///
    /// 端色（S-14/S-15）**alpha 恒为 0xFF**：`argb_to_rgba()` 本来就丢弃 alpha
    /// （`Rgba::opaque`），端色的半透明对屏幕零影响，留着只会让 HEX 框显示
    /// `#80FFFFFF` 而屏幕是纯白——那种"显示值与实际效果不一致"最难排查。
    /// S-35/S-37 是叠加在任务栏上的填充/描边色，alpha 有实际语义，必须原样保留。
    #[must_use]
    const fn normalize_color(self, value: i64) -> i64 {
        match self {
            Self::S14 | Self::S15 | Self::S52 | Self::S53 | Self::S54 | Self::S55
            | Self::S56 | Self::S57 | Self::S58 | Self::S59 => value | 0xFF00_0000,
            _ => value,
        }
    }
}

/// 六个分组（§6.1~§6.5 + T03.5 诊断面板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// 通用。
    General,
    /// 配色与采样。
    Color,
    /// 采样区域微调。
    Region,
    /// 着色元素。
    Elements,
    /// 托盘与行为。
    Tray,
    /// 行为（S-45/S-46 独立成组标题）。
    Behavior,
    /// 诊断面板。
    Diagnostics,
}

impl Group {
    /// 全部 7 个分组标题（A-25 要求 5 个设置分组 + 诊断 + 关于）。
    pub const ALL: [Self; 7] = [
        Self::General, Self::Color, Self::Region, Self::Elements,
        Self::Tray, Self::Behavior, Self::Diagnostics,
    ];

    /// 分组标题的 i18n key。
    #[must_use]
    pub const fn title_key(self) -> &'static str {
        match self {
            Self::General => "group.general",
            Self::Color => "group.color",
            Self::Region => "group.region",
            Self::Elements => "group.elements",
            Self::Tray => "group.tray",
            Self::Behavior => "group.behavior",
            Self::Diagnostics => "group.diagnostics",
        }
    }

}

/// 控件类型（§6 控件缩写 + T03.3 七类控件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// 开关。
    Toggle,
    /// 滑块。
    Slider,
    /// 下拉。
    Dropdown,
    /// ARGB 颜色选择器（Hex 输入 + 预设色板 + 内置 alpha）。
    Color,
    /// 动作按钮。
    Button,
    /// 只读展示。
    ReadOnly,
}

/// 单个设置项的元数据（全 `const`，零分配）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingMeta {
    /// 所属分组。
    pub group: Group,
    /// 标签 i18n key。
    pub label: &'static str,
    /// 说明 i18n key。
    pub help: &'static str,
    /// 控件类型。
    pub kind: ControlKind,
    /// 默认值。
    pub default_value: i64,
    /// 滑块/下拉下限（滑块为最小值，下拉恒 0）。
    pub min: i64,
    /// 滑块上限 / 下拉选项数。
    pub max: i64,
    /// 滑块步进。
    pub step: i64,
    /// 下拉选项的 i18n key 表（非下拉为 `&[]`）。
    pub options: &'static [&'static str],
}

impl SettingMeta {
    const fn toggle(g: Group, label: &'static str, help: &'static str, def: i64) -> Self {
        Self { group: g, label, help, kind: ControlKind::Toggle, default_value: def, min: 0, max: 1, step: 1, options: &[] }
    }
    const fn slider(g: Group, label: &'static str, help: &'static str, def: i64, min: i64, max: i64, step: i64) -> Self {
        Self { group: g, label, help, kind: ControlKind::Slider, default_value: def, min, max, step, options: &[] }
    }
    const fn dropdown(g: Group, label: &'static str, help: &'static str, def: i64, count: i64, options: &'static [&'static str]) -> Self {
        Self { group: g, label, help, kind: ControlKind::Dropdown, default_value: def, min: 0, max: count - 1, step: 1, options }
    }
    const fn color(g: Group, label: &'static str, help: &'static str, def: i64) -> Self {
        // 色板不再是"下拉选项"，而是弹层里的分组色块（见 ui::i18n::COLOR_SWATCH_GROUPS），
        // 因此 options 为空；值域仍是 0xAARRGGBB。
        Self { group: g, label, help, kind: ControlKind::Color, default_value: def, min: 0, max: 0xFFFF_FFFF, step: 1, options: &[] }
    }
    const fn button(g: Group, label: &'static str, help: &'static str) -> Self {
        Self { group: g, label, help, kind: ControlKind::Button, default_value: 0, min: 0, max: 0, step: 0, options: &[] }
    }
    const fn readonly(g: Group, label: &'static str, help: &'static str) -> Self {
        Self { group: g, label, help, kind: ControlKind::ReadOnly, default_value: 0, min: 0, max: 0, step: 0, options: &[] }
    }

    /// 把任意输入钳到合法范围（NFR-16：字段越界回退，不崩溃）。
    #[must_use]
    pub const fn clamp(self, value: i64) -> i64 {
        match self.kind {
            ControlKind::Toggle => if value != 0 { 1 } else { 0 },
            ControlKind::Slider | ControlKind::Dropdown => {
                let v = if value < self.min { self.min } else if value > self.max { self.max } else { value };
                if self.step > 1 {
                    self.min + ((v - self.min) / self.step) * self.step
                } else {
                    v
                }
            }
            ControlKind::Color => {
                // 禁用全透明：alpha 最低 1（§6 S-14/S-15）。
                let a = (value >> 24) & 0xFF;
                if a == 0 { value | 0x0100_0000 } else { value & 0xFFFF_FFFF }
            }
            ControlKind::Button | ControlKind::ReadOnly => 0,
        }
    }
}

/// 界面语言（S-04）。`System` 按主语言 ID 2052 判定 zh-CN，其余 English。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// 跟随系统。
    System,
    /// 简体中文。
    Zh,
    /// English。
    En,
}

impl Lang {
    /// 把「跟随系统」解析成具体语言。
    #[must_use]
    pub fn resolved(self) -> Self {
        match self {
            Self::System => {
                // SAFETY: GetUserDefaultUILanguage 无参数、无前置条件。
                let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
                if id & 0xFF == 0x04 && (id & 0xFF00) >> 8 == 0x08 {
                    // 0x0804 = zh-CN；2052 十进制即 0x0804。
                    Self::Zh
                } else {
                    Self::En
                }
            }
            other => other,
        }
    }

    /// 从设置值构造。
    #[must_use]
    pub const fn from_value(v: i64) -> Self {
        match v {
            1 => Self::Zh,
            2 => Self::En,
            _ => Self::System,
        }
    }

}

/// 36 项配置快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    values: [i64; SETTING_COUNT],
    version: u64,
}

/// 兼容读取的下限项数。
///
/// 新增配置项一律追加在 `SettingKey` 末尾，旧 `config.json` 的数组会短于
/// 当前 `SETTING_COUNT`；缺失的尾部按默认值补齐即可，不能因此判定文件损坏
/// （否则升级一次就把用户设置全清）。过短的数组仍按损坏处理，保留 NFR-16 兜底。
const MIN_LEGACY_COUNT: usize = 30;

impl Default for Config {
    fn default() -> Self {
        let mut values = [0i64; SETTING_COUNT];
        for key in SettingKey::ALL {
            values[key.index()] = key.meta().default_value;
        }
        Self { values, version: 1 }
    }
}

impl Config {
    /// 读值。
    #[must_use]
    pub const fn get(&self, key: SettingKey) -> i64 {
        self.values[key.index()]
    }

    /// 写值并钳制；返回是否真的改变（避免无谓的落盘与 `version` 抖动）。
    pub fn set(&mut self, key: SettingKey, raw: i64) -> bool {
        let v = key.normalize_color(key.meta().clamp(raw));
        if self.values[key.index()] == v {
            return false;
        }
        self.values[key.index()] = v;
        // 端色改了但「颜色策略」还是纯黑白时，同步切到自定义。
        // `to_engine_config()` 与 `PolicyCfg::from_config()` 在 S-13 == 0 时强制黑白，
        // 不切的话用户选完颜色任务栏毫无反应——看起来像设置没生效。
        if matches!(key, SettingKey::S14 | SettingKey::S15)
            && self.values[SettingKey::S13.index()] == 0
        {
            self.values[SettingKey::S13.index()] = 1;
            note("colour strategy auto-switched to custom (S-13) after end colour change");
        }
        self.version = self.version.wrapping_add(1).max(1);
        true
    }

    /// 当前配置版本（A-10：core 每拍比对）。
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// 当前语言（S-04）。
    #[must_use]
    pub const fn language(&self) -> Lang {
        Lang::from_value(self.get(SettingKey::S04))
    }

    /// 恢复全部默认值（A-12）。
    pub fn restore_defaults(&mut self) {
        *self = Self::default();
    }

    /// 全量钳制（加载损坏配置后调用）。手改 `config.json` 写入半透明端色时，
    /// 这里负责把 alpha 收回 0xFF，与 [`Config::set`] 完全同一套规范化。
    pub fn sanitize(&mut self) {
        for key in SettingKey::ALL {
            let v = key.normalize_color(key.meta().clamp(self.values[key.index()]));
            self.values[key.index()] = v;
        }
    }

    /// 转成 core 引擎配置快照。这是设置 → 引擎的**唯一**换算点。
    #[must_use]
    pub fn to_engine_config(&self) -> EngineConfig {
        let mut cfg = EngineConfig::default_v1();
        cfg.interval_ms = self.get(SettingKey::S16) as u32;
        cfg.region = RegionCfg {
            x_offset_pct: self.get(SettingKey::S20) as u8,
            width_pct: self.get(SettingKey::S21) as u8,
            v_inset_pct: self.get(SettingKey::S22) as u8,
        };
        cfg.judge.threshold_y = self.get(SettingKey::S12) as f32 / 1000.0;
        cfg.judge.change_threshold = self.get(SettingKey::S17) as u8;
        cfg.judge.ewma_alpha = self.get(SettingKey::S18) as f32 / 1000.0;

        // S-10 自适应模式：强制亮/暗端用极端阈值实现（core JudgeCfg 无 force 字段）。
        match self.get(SettingKey::S10) {
            1 => cfg.judge.threshold_y = 1.0,
            2 => cfg.judge.threshold_y = 0.0,
            _ => {}
        }
        // S-13 颜色策略：非「自定义」时强制纯黑纯白，S-14/S-15 值保留但不生效。
        if self.get(SettingKey::S13) == 0 {
            cfg.judge.light_color = Rgba::opaque(255, 255, 255);
            cfg.judge.dark_color = Rgba::opaque(0, 0, 0);
        } else {
            cfg.judge.light_color = argb_to_rgba(self.get(SettingKey::S14));
            cfg.judge.dark_color = argb_to_rgba(self.get(SettingKey::S15));
        }
        // S-01 总开关关 / S-11 不跟随壁纸 / S-42 暂停 → 停止采样，保持上次判定。
        cfg.paused = self.get(SettingKey::S01) == 0
            || self.get(SettingKey::S11) == 0
            || self.get(SettingKey::S42) != 0;
        cfg.battery_saver = true;
        cfg.version = self.version();
        cfg.sanitize();
        cfg
    }

    /// 序列化为单行 JSON。
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(320);
        s.push_str("{\"version\":");
        push_u64(&mut s, self.version);
        s.push_str(",\"values\":[");
        for (i, v) in self.values.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            push_i64(&mut s, *v);
        }
        s.push_str("]}\n");
        s
    }

    /// 从 JSON 文本解析。缺失的尾部配置项按默认值补齐；结构/数值异常返回 `None`。
    #[must_use]
    pub fn from_json(text: &str) -> Option<Self> {
        let marker = "\"values\"";
        let key_at = text.find(marker)?;
        let rest = &text[key_at + marker.len()..];
        let open = rest.find('[')?;
        let body_start = &rest[open + 1..];
        let close = body_start.find(']')?;
        let body = &body_start[..close];
        // 以默认值为底：旧版本的 config.json 缺 S-47 等新增项时，尾部取默认值。
        let mut cfg = Self::default();
        let mut count = 0usize;
        for token in body.split(',') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            let v = t.parse::<i64>().ok()?;
            if count >= SETTING_COUNT {
                return None;
            }
            cfg.values[count] = v;
            count += 1;
        }
        if count < MIN_LEGACY_COUNT || count > SETTING_COUNT {
            return None;
        }
        cfg.version = 1;
        cfg.sanitize();
        Some(cfg)
    }

    /// 产品数据目录：`%LOCALAPPDATA%\ShadeBar\`。
    ///
    /// 取不到 `LOCALAPPDATA` 时回退到当前工作目录下的 `.shadebar\`，
    /// 对应 A-11「exe 放在 Program Files 时回退到可写目录」。
    #[must_use]
    pub fn data_dir() -> PathBuf {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            PathBuf::from(base).join(shadebar_core::DATA_DIR_NAME)
        } else {
            PathBuf::from(".").join(".shadebar")
        }
    }

    /// 配置文件路径。
    #[must_use]
    pub fn config_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// 配置导出文件路径（`%LOCALAPPDATA%\ShadeBar\config-export.json`）。
    #[must_use]
    pub fn export_file_path() -> PathBuf {
        Self::data_dir().join("config-export.json")
    }

    /// 导出当前配置到指定路径（测试传临时路径，不污染真实配置）。
    ///
    /// # Errors
    ///
    /// 目录不可建或文件不可写时返回 `PathNotWritable` / `ConfigSerialize`。
    pub fn export_to(&self, path: &Path) -> SbResult<()> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| SbError::PathNotWritable {
            path: format!("{}: {e}", dir.display()),
        })?;
        std::fs::write(path, self.to_json()).map_err(|e| SbError::ConfigSerialize {
            detail: format!("{}: {e}", path.display()),
        })
    }

    /// 从指定路径读回配置（走同一个 `from_json` + `sanitize`，手改坏文件不崩）。
    /// 返回配置值是否真的变了。
    ///
    /// # Errors
    ///
    /// 文件不可读或 JSON 结构非法时返回 `ConfigParse`。
    pub fn import_from(&mut self, path: &Path) -> SbResult<bool> {
        let text = std::fs::read_to_string(path).map_err(|e| SbError::ConfigParse {
            detail: format!("{}: {e}", path.display()),
        })?;
        let next = Self::from_json(&text).ok_or_else(|| SbError::ConfigParse {
            detail: format!("{} 结构非法", path.display()),
        })?;
        let changed = next.values != self.values;
        *self = next;
        Ok(changed)
    }

    /// 从磁盘加载；损坏/越界/不可读时回退默认并把损坏文件改名为 `.bak`（NFR-16）。
    #[must_use]
    pub fn load() -> Self {
        let path = Self::config_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        if let Some(cfg) = Self::from_json(&text) {
            return cfg;
        }
        // 损坏文件保留现场，不静默覆盖。
        let bak = path.with_extension("json.bak");
        let _ = std::fs::rename(&path, &bak);
        note(&format!("config corrupted, fallback to defaults; backup={}", bak.display()));
        Self::default()
    }

    /// 落盘（原子替换：先写 `.tmp` 再改名，避免写一半留下损坏文件）。
    ///
    /// # Errors
    ///
    /// 目录不可建或文件不可写时返回 `SbError::PathNotWritable`。
    pub fn save(&self) -> SbResult<()> {
        let dir = Self::data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| SbError::PathNotWritable {
            path: format!("{}: {e}", dir.display()),
        })?;
        let path = Self::config_path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json()).map_err(|e| SbError::PathNotWritable {
            path: format!("{}: {e}", tmp.display()),
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| SbError::PathNotWritable {
            path: format!("{}: {e}", path.display()),
        })?;
        Ok(())
    }
}

/// `0xAARRGGBB` → 不透明 sRGB（alpha 只用于 L3 悬停块，判定色恒不透明）。
#[must_use]
pub const fn argb_to_rgba(v: i64) -> Rgba {
    let argb = v as u32;
    Rgba::opaque((argb >> 16) as u8, (argb >> 8) as u8, argb as u8)
}

/// 极简日志：配置加载/损坏/落盘失败都留痕，不含任何用户隐私内容。
pub fn note(line: &str) {
    let dir = Config::data_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("shadebar.log");
    let mut text = String::with_capacity(line.len() + 24);
    text.push_str("config: ");
    text.push_str(line);
    text.push('\n');
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(text.as_bytes())
        });
}

fn push_u64(out: &mut String, mut v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for &b in &buf[i..] {
        out.push(char::from(b));
    }
}

fn push_i64(out: &mut String, v: i64) {
    if v < 0 {
        out.push('-');
        // i64::MIN 的绝对值溢出，先转 u128 再取十进制。
        push_u128(out, (v as i128).unsigned_abs());
    } else {
        push_u64(out, v as u64);
    }
}

fn push_u128(out: &mut String, mut v: u128) {
    let mut buf = [0u8; 40];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for &b in &buf[i..] {
        out.push(char::from(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_35_settings_with_unique_indices() {
        assert_eq!(SettingKey::ALL.len(), SETTING_COUNT);
        let mut seen = [false; SETTING_COUNT];
        for k in SettingKey::ALL {
            assert!(!seen[k.index()], "duplicate index for {}", k.id());
            seen[k.index()] = true;
        }
        assert!(seen.iter().all(|&b| b));
    }

    #[test]
    fn defaults_match_requirements_table() {
        let c = Config::default();
        assert_eq!(c.get(SettingKey::S01), 1);
        assert_eq!(c.get(SettingKey::S02), 0);
        assert_eq!(c.get(SettingKey::S03), 1);
        assert_eq!(c.get(SettingKey::S04), 0);
        assert_eq!(c.get(SettingKey::S12), 500);
        assert_eq!(c.get(SettingKey::S16), 1000);
        assert_eq!(c.get(SettingKey::S17), 3);
        assert_eq!(c.get(SettingKey::S18), 300);
        assert_eq!(c.get(SettingKey::S19), 0);
        assert_eq!(c.get(SettingKey::S21), 84);
        assert_eq!(c.get(SettingKey::S41), 0);
        assert_eq!(c.get(SettingKey::S45), 2);
        assert_eq!(c.get(SettingKey::S35), 0x38FF_FFFF);
        assert_eq!(c.get(SettingKey::S37), 0x38FF_FFFF);
    }

    #[test]
    fn json_roundtrip_preserves_all_values() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S16, 2000));
        assert!(c.set(SettingKey::S04, 2));
        let text = c.to_json();
        let parsed = Config::from_json(&text);
        assert!(parsed.is_some(), "own JSON must parse");
        let Some(back) = parsed else {
            return;
        };
        assert_eq!(back.get(SettingKey::S16), 2000);
        assert_eq!(back.get(SettingKey::S04), 2);
        assert_eq!(back.get(SettingKey::S35), c.get(SettingKey::S35));
    }

    #[test]
    fn corrupt_json_is_rejected_not_panicking() {
        assert!(Config::from_json("{ this is not json").is_none());
        assert!(Config::from_json("{\"values\":[1,2,3]}").is_none());
        assert!(Config::from_json("{\"version\":1,\"values\":[]}").is_none());
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let text = format!("{{\"version\":1,\"values\":[{}]}}", vec!["999999"; SETTING_COUNT].join(","));
        let parsed = Config::from_json(&text);
        assert!(parsed.is_some(), "structure valid");
        let Some(c) = parsed else {
            return;
        };
        for k in SettingKey::ALL {
            let m = k.meta();
            let v = c.get(k);
            match m.kind {
                ControlKind::Slider | ControlKind::Dropdown => {
                    assert!(v >= m.min && v <= m.max, "{} out of range: {v}", k.id());
                }
                ControlKind::Toggle => assert!(v == 0 || v == 1),
                _ => {}
            }
        }
    }

    #[test]
    fn engine_config_maps_interval_region_and_pause() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S16, 2000));
        assert!(c.set(SettingKey::S20, 20));
        assert!(c.set(SettingKey::S21, 60));
        assert!(c.set(SettingKey::S22, 15));
        let e = c.to_engine_config();
        assert_eq!(e.interval_ms, 2000);
        assert_eq!(e.region.x_offset_pct, 20);
        assert_eq!(e.region.width_pct, 60);
        assert_eq!(e.region.v_inset_pct, 15);
        assert_eq!(e.version, c.version());
        assert!(!e.paused);
        assert!(c.set(SettingKey::S42, 1));
        assert!(c.to_engine_config().paused);
    }

    #[test]
    fn set_bumps_version_only_on_real_change() {
        let mut c = Config::default();
        let v = c.version();
        assert!(!c.set(SettingKey::S01, 1));
        assert_eq!(c.version(), v);
        assert!(c.set(SettingKey::S01, 0));
        assert_eq!(c.version(), v + 1);
    }

    #[test]
    fn restore_defaults_resets_every_value() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S16, 5000));
        assert!(c.set(SettingKey::S04, 2));
        c.restore_defaults();
        assert_eq!(c.get(SettingKey::S16), 1000);
        assert_eq!(c.get(SettingKey::S04), 0);
    }

    #[test]
    fn language_resolution_values() {
        assert_eq!(Lang::from_value(0), Lang::System);
        assert_eq!(Lang::from_value(1), Lang::Zh);
        assert_eq!(Lang::from_value(2), Lang::En);
        assert_eq!(Lang::from_value(9), Lang::System, "越界值回退跟随系统");
        assert_eq!(Lang::En.resolved(), Lang::En);
    }

    #[test]
    fn color_field_never_allows_fully_transparent() {
        let m = SettingKey::S14.meta();
        assert_eq!(m.clamp(0x00FF_FFFF), 0x01FF_FFFF);
        assert_eq!(m.clamp(0xFFFF_FFFF), 0xFFFF_FFFF);
    }

    #[test]
    fn end_colours_are_forced_opaque_but_fill_colours_keep_alpha() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S14, 0x8012_3456));
        assert_eq!(c.get(SettingKey::S14), 0xFF12_3456, "端色 alpha 必须收回 0xFF");
        assert!(c.set(SettingKey::S15, 0x8011_2233));
        assert_eq!(c.get(SettingKey::S15), 0xFF11_2233);
        // alpha=0 的输入先被 clamp 抬到 1，再被规范化为 0xFF
        assert!(c.set(SettingKey::S15, 0x0011_2234));
        assert_eq!(c.get(SettingKey::S15), 0xFF11_2234);
        // S-35/S-37 的 alpha 有实际语义（默认 0x38），不能被抹平
        assert!(!c.set(SettingKey::S35, 0x38FF_FFFF), "同值不应抖动");
        assert_eq!(c.get(SettingKey::S35), 0x38FF_FFFF);
        assert!(c.set(SettingKey::S35, 0x38E5_484D));
        assert_eq!(c.get(SettingKey::S35), 0x38E5_484D);
        assert!(c.set(SettingKey::S37, 0x20E5_484D));
        assert_eq!(c.get(SettingKey::S37), 0x20E5_484D);
        // 加载路径同样规范化：手改 config.json 写了半透明端色也收得回来
        let mut d = Config::default();
        d.values[SettingKey::S14.index()] = 0x8012_3456;
        d.values[SettingKey::S35.index()] = 0x38FF_FFFF;
        d.sanitize();
        assert_eq!(d.get(SettingKey::S14), 0xFF12_3456);
        assert_eq!(d.get(SettingKey::S35), 0x38FF_FFFF);
    }

    #[test]
    fn changing_end_colour_switches_strategy_to_custom() {
        let mut c = Config::default();
        assert_eq!(c.get(SettingKey::S13), 0);
        assert!(c.set(SettingKey::S14, 0x8012_3456));
        assert_eq!(c.get(SettingKey::S13), 1, "端色改动必须把颜色策略切到自定义");
        // 值没变就不该动 S-13（避免用户什么都没改却被切策略）
        let mut d = Config::default();
        assert!(!d.set(SettingKey::S14, 0xFFFF_FFFF));
        assert_eq!(d.get(SettingKey::S13), 0);
        // 已经是自定义时不再重复切换，也不报错
        let mut e = Config::default();
        assert!(e.set(SettingKey::S13, 1));
        assert!(e.set(SettingKey::S15, 0xFF11_2233));
        assert_eq!(e.get(SettingKey::S13), 1);
        // 切到自定义后引擎侧真的用上用户色
        let e = e.to_engine_config();
        assert_eq!(e.judge.light_color, Rgba::opaque(255, 255, 255));
        assert_eq!(e.judge.dark_color, Rgba::opaque(0x11, 0x22, 0x33));
    }

    #[test]
    fn argb_to_rgba_always_drops_alpha() {
        assert_eq!(argb_to_rgba(0x8012_3456).a, 255);
        assert_eq!(argb_to_rgba(0x8012_3456), Rgba::opaque(0x12, 0x34, 0x56));
    }

    /// 问题 2 之后 S-14/S-15 端色不再参与取色，从设置页收起。
    #[test]
    fn visible_keys_match_expectation() {
        let visible: Vec<SettingKey> = SettingKey::ALL
            .iter()
            .copied()
            .filter(|k| k.ui_visible())
            .collect();
        let want = [
            SettingKey::S01, SettingKey::S02, SettingKey::S03, SettingKey::S04,
            SettingKey::S10, SettingKey::S12,
            SettingKey::S16, SettingKey::S18, SettingKey::S20, SettingKey::S21,
            SettingKey::S22, SettingKey::S40, SettingKey::S41, SettingKey::S45,
            SettingKey::S47, SettingKey::S48, SettingKey::S49, SettingKey::S50,
            SettingKey::S51,
            SettingKey::S52, SettingKey::S53, SettingKey::S54, SettingKey::S55,
            SettingKey::S56, SettingKey::S57, SettingKey::S58, SettingKey::S59,
            SettingKey::S60,
        ];
        assert_eq!(visible, want);
        for key in [
            SettingKey::S05, SettingKey::S06, SettingKey::S11, SettingKey::S13,
            SettingKey::S14, SettingKey::S15, SettingKey::S17, SettingKey::S19,
            SettingKey::S23, SettingKey::S30, SettingKey::S31, SettingKey::S32,
            SettingKey::S33, SettingKey::S34, SettingKey::S35, SettingKey::S36,
            SettingKey::S37, SettingKey::S42, SettingKey::S43, SettingKey::S44,
            SettingKey::S46,
        ] {
            assert!(!key.ui_visible(), "{} 应已从设置页收起", key.id());
        }
    }

    #[test]
    fn config_export_import_is_json_roundtrip_without_touching_real_path() {
        let mut src = Config::default();
        assert!(src.set(SettingKey::S16, 2400));
        assert!(src.set(SettingKey::S14, 0x8011_2233));
        let path = std::env::temp_dir().join(format!(
            "shadebar-g5-export-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        src.export_to(&path).expect("导出到临时路径必须成功");
        let text = std::fs::read_to_string(&path).expect("导出文件可读");
        assert!(Config::from_json(&text).is_some(), "导出内容必须是合法 JSON");

        let mut dst = Config::default();
        assert!(dst.import_from(&path).expect("导入不崩"));
        for k in SettingKey::ALL {
            assert_eq!(dst.get(k), src.get(k), "{} 导入后不一致", k.id());
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn importing_corrupt_json_keeps_the_existing_config() {
        let mut cfg = Config::default();
        assert!(cfg.set(SettingKey::S16, 4321));
        let path = std::env::temp_dir().join(format!(
            "shadebar-g5-corrupt-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::write(&path, "{\"values\":[1,2,3]}").expect("临时文件可写");
        let before = cfg.clone();
        assert!(cfg.import_from(&path).is_err(), "短数组必须被拒");
        assert_eq!(cfg, before, "导入失败不得改变现有配置");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn neg_push_helpers_handle_min() {
        let mut s = String::new();
        push_i64(&mut s, i64::MIN);
        assert_eq!(s, i64::MIN.to_string());
    }
}
