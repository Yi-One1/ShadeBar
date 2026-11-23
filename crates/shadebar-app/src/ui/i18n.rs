//! 内嵌双语资源（T01.7 / NFR-19 / FR-18 / A-25 / A-26）。
//!
//! # 为什么是 TSV + `include_str!`
//!
//! * **文案与代码分离**：所有用户可见文本只在 `resources/lang/*.tsv`，
//!   本文件只有查表逻辑，UI 逻辑（`widgets.rs` / `pages/*` / `tray.rs`）里
//!   一个硬编码文案都没有（NFR-19 / A-25）。
//! * **内嵌单文件**：`include_str!` 在编译期把文本编进 exe，运行期不存在
//!   `lang\*.json` 侧车文件；`dist\` 里只有一个 exe（A-26）。
//! * **A-26 对照**：`no-en` 特性关闭时英文表完全不参与编译，与正式版作
//!   体积差对照；这是「移除 en 资源」的真实实现而不是估算。
//! * **零依赖**：不引入 `rust-i18n` / `fluent`，避免 >40 KB 框架开销。
//!
//! # 覆盖清单（A-25）
//!
//! 49 项标签与说明、7 个分组标题、全部下拉选项、诊断面板标签、托盘菜单 4 项、
//! 首启气泡、降级横幅、注入失败提示、恢复默认确认框、关于页、日志级别名称。
//! 单测 `both_tables_cover_every_required_key` 逐项核对，缺一条即测试失败。

use crate::config::{Group, Lang, SettingKey};

/// 简体中文文案表。
const ZH_TSV: &str = include_str!("../../../../resources/lang/zh-CN.tsv");
/// 英文文案表（`no-en` 对照构建时刻意不编译）。
#[cfg(not(feature = "no-en"))]
const EN_TSV: &str = include_str!("../../../../resources/lang/en-US.tsv");

// ---- 下拉选项的 i18n key 表（被 `config.rs::SettingMeta` 以 const 引用）----

/// S-04 界面语言。
pub const OPT_LANG: &[&str] = &["opt.lang.system", "opt.lang.zh", "opt.lang.en"];
/// S-10 自适应模式。
pub const OPT_ADAPTIVE: &[&str] = &["opt.adaptive.auto", "opt.adaptive.light", "opt.adaptive.dark"];
/// S-13 颜色策略。
pub const OPT_STRATEGY: &[&str] = &["opt.strategy.bw", "opt.strategy.custom"];
/// S-34 / S-36 配色来源。
pub const OPT_SOURCE: &[&str] = &["opt.source.adaptive", "opt.source.fixed"];
/// S-40 左键单击托盘图标。
pub const OPT_LEFT_CLICK: &[&str] = &["opt.left.settings", "opt.left.menu", "opt.left.none"];
/// S-41 关闭设置窗口时。
pub const OPT_CLOSE: &[&str] = &["opt.close.tray", "opt.close.exit"];
/// S-44 注入失败时。
pub const OPT_INJECT_FAIL: &[&str] = &["opt.inject.once", "opt.inject.silent", "opt.inject.always"];
/// S-45 日志级别。
pub const OPT_LOG_LEVEL: &[&str] = &["opt.log.error", "opt.log.warn", "opt.log.info", "opt.log.debug"];
/// S-47 判定粒度。
pub const OPT_JUDGE_MODE: &[&str] = &["opt.judge.region", "opt.judge.global"];
/// 颜色弹层的预设色板：**按用途分三组**，每组 6 色排成一行。
///
/// # 分工（这是"按 key 选组"的依据，见 `ui::swatch_groups_for`）
///
/// | 组 | key | 用在哪 | 是什么 |
/// |---|---|---|---|
/// | A | `picker.on_dark` | S-14 亮端色 | **文字色**：深色背景上用 |
/// | B | `picker.on_light` | S-15 暗端色 | **文字色**：浅色背景上用 |
/// | C | `picker.fill` | S-35 / S-37 | **填充/描边色**：半透明叠在任务栏上 |
///
/// 三组不能混：文字色要保证对比度（A 组每个值在深背景 ≥9.5:1、B 组在浅背景
/// ≥8.0:1），填充色要保证两种背景上都看得见（每个值双背景 ≥3.31:1）。
/// 曾经三组共用一份模板，结果是"调文字色看到 6 个饱和强调色、调指示条看到
/// 6 个近白色"——每一格都不能用。
///
/// # 排序轴（三组一致）
///
/// 位置 1 固定默认色（`#FFFFFF` / `#000000` / 首选项），2–6 按色相
/// 红→琥珀→绿→蓝→紫；组内明度极差 ≤1.3，扫一眼就能定位。
pub const COLOR_SWATCH_GROUPS: &[(&str, [&str; 6])] = &[
    (
        "picker.on_dark",
        ["#FFFFFF", "#FFB4AB", "#FFD08A", "#A6E3A1", "#9CCBFF", "#D9B8FF"],
    ),
    (
        "picker.on_light",
        ["#000000", "#7A1F2B", "#663C00", "#0F5132", "#1B4A82", "#5B2A85"],
    ),
    (
        "picker.fill",
        ["#E5484D", "#C96A00", "#12855A", "#0E8D8D", "#3B82F6", "#8B5CF6"],
    ),
];

/// 除设置项标签/说明/选项与分组标题之外的 UI 文案 key（A-25 逐项核对用）。
///
/// 只在 `cargo test` 的覆盖单测里被枚举；发布版由 TSV 直接查表，
/// 不需要这份清单，故显式放宽非测试构建的 dead_code。
#[cfg_attr(not(test), allow(dead_code))]
pub const UI_KEYS: &[&str] = &[
    // 托盘菜单 4 项（设置 / 开机启动 / 版本号 / 退出）
    "tray.open", "tray.autostart", "tray.version", "tray.exit",
    // 气泡 / 角标
    "tray.balloon.first_run", "tray.restricted",
    // 横幅 / 警告
    "banner.degraded.title", "banner.degraded.body", "banner.inject_fail",
    "banner.s19_warning", "banner.region_small",
    // 诊断面板
    "diag.title", "diag.rep_color", "diag.decision_color", "diag.y", "diag.contrast",
    "diag.valid_px", "diag.cost_us", "diag.samples", "diag.trend", "diag.contrast_low",
    "diag.region_suspect", "diag.no_data", "diag.timestamp", "diag.expand", "diag.collapse",
    // 恢复默认二次确认（A-12）
    "dlg.restore.title", "dlg.restore.body", "dlg.confirm", "dlg.cancel",
    // 关于页
    "about.title", "about.version", "about.license", "about.known_limits", "about.copy_version",
    // 颜色弹层（色板组标题 / HEX / 更多颜色 / 非法输入 / 策略自动切换）
    "picker.hex", "picker.more", "picker.on_dark", "picker.on_light", "picker.fill",
    "picker.invalid", "picker.switched",
    // 页签 / 顶栏 / 底栏 / 状态 chip
    "tab.taskbar", "tab.sample", "tab.tray", "tab.diag",
    "ui.running", "ui.paused", "ui.restricted", "ui.tint", "ui.pause", "ui.resume",
    "action.restore", "action.about", "action.export", "action.copy",
    "action.export_diag", "action.export_config", "action.import_config",
    "config.export_ok", "config.export_fail", "config.import_ok", "config.import_fail",
    "diag.export_ok", "diag.export_fail",
    // 任务栏预览 / 三区 / 采样带 / 诊断摘要
    "preview.auto", "preview.light", "preview.dark",
    "zone.icons", "zone.clock", "zone.tray",
    "card.colors", "color.on_dark", "color.on_light", "thr.title",
    "sample.title", "sample.live", "diag.live", "diag.sample_unit",
    // 通用
    "common.on", "common.off", "common.enabled", "common.disabled",
    "common.copy_ok", "common.export_ok", "common.region_preview",
];

/// 在一份 TSV 里按键查值。行格式 `key\tvalue`，`#` 开头为注释。
fn lookup(tsv: &'static str, key: &str) -> Option<&'static str> {
    for line in tsv.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('\t') else {
            continue;
        };
        if k == key {
            return Some(v);
        }
    }
    None
}

/// 取一份文案；缺 key 时回退到中文表，再缺则原样返回 key（绝不空白/崩溃）。
///
/// 返回值与 `key` 共用生命周期：表内命中是 `'static`，缺 key 时返回调用方
/// 传入的 `key` 本身，两者都可以安全地缩到 `'a`。
#[must_use]
pub fn text(key: &str, lang: Lang) -> &str {
    match lang.resolved() {
        Lang::En => en_lookup(key),
        _ => lookup(ZH_TSV, key).unwrap_or(key),
    }
}

#[cfg(not(feature = "no-en"))]
fn en_lookup(key: &str) -> &str {
    lookup(EN_TSV, key).unwrap_or_else(|| lookup(ZH_TSV, key).unwrap_or(key))
}

/// A-26 对照构建：英文表被移除，回退中文（该产物只用于体积差测量，不发布）。
#[cfg(feature = "no-en")]
fn en_lookup(key: &str) -> &str {
    lookup(ZH_TSV, key).unwrap_or(key)
}

/// 正式版是否编译了英文资源（A-26 报告用）。
#[must_use]
pub const fn has_english() -> bool {
    !cfg!(feature = "no-en")
}

/// 全部必需 key（供 A-25 覆盖核对单测遍历）。
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn required_keys() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::with_capacity(160);
    for g in Group::ALL {
        out.push(g.title_key());
    }
    for k in SettingKey::ALL {
        let m = k.meta();
        out.push(m.label);
        out.push(m.help);
        for &o in m.options {
            // 颜色预设是 Hex 字面量，不是 i18n key。
            if !o.starts_with('#') {
                out.push(o);
            }
        }
    }
    out.extend_from_slice(UI_KEYS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tables_cover_every_required_key() {
        #[cfg(not(feature = "no-en"))]
        let tables: Vec<(&str, &'static str)> = vec![("zh-CN", ZH_TSV), ("en-US", EN_TSV)];
        #[cfg(feature = "no-en")]
        let tables: Vec<(&str, &'static str)> = vec![("zh-CN", ZH_TSV)];
        for (name, tsv) in tables {
            for key in required_keys() {
                let value = lookup(tsv, key);
                assert!(value.is_some(), "{name} missing key: {key}");
                let Some(v) = value else {
                    return;
                };
                assert!(!v.trim().is_empty(), "{name} empty value for {key}");
            }
        }
    }

    #[test]
    fn both_tables_have_identical_key_sets() {
        #[cfg(not(feature = "no-en"))]
        {
            let keys = |tsv: &'static str| -> Vec<&str> {
                tsv.lines()
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .filter_map(|l| l.split_once('\t').map(|(k, _)| k))
                    .collect()
            };
            let zh = keys(ZH_TSV);
            let en = keys(EN_TSV);
            assert_eq!(zh.len(), en.len(), "key count differs between languages");
            for k in &zh {
                assert!(en.contains(k), "en-US missing key: {k}");
            }
        }
    }

    #[test]
    fn known_texts_resolve_in_both_languages() {
        assert_eq!(text("group.general", Lang::Zh), "通用");
        #[cfg(not(feature = "no-en"))]
        assert_eq!(text("group.general", Lang::En), "General");
        assert_eq!(text("tray.exit", Lang::Zh), "退出");
        assert_eq!(text("no.such.key", Lang::Zh), "no.such.key");
    }

    #[test]
    fn option_tables_are_not_empty_and_in_range() {
        assert_eq!(OPT_LANG.len(), 3);
        assert_eq!(OPT_ADAPTIVE.len(), 3);
        assert_eq!(OPT_STRATEGY.len(), 2);
        assert_eq!(OPT_SOURCE.len(), 2);
        assert_eq!(OPT_LEFT_CLICK.len(), 3);
        assert_eq!(OPT_CLOSE.len(), 2);
        assert_eq!(OPT_INJECT_FAIL.len(), 3);
        assert_eq!(OPT_LOG_LEVEL.len(), 4);
        assert_eq!(COLOR_SWATCH_GROUPS.len(), 3);
        for (key, colors) in COLOR_SWATCH_GROUPS {
            assert!(!key.is_empty());
            assert_eq!(colors.len(), 6, "每组必须 6 色，渲染按 6 列排");
        }
    }

    #[test]
    fn swatch_colours_are_parseable_and_globally_unique() {
        let mut seen: Vec<shadebar_core::color::Rgba> = Vec::new();
        for (key, colors) in COLOR_SWATCH_GROUPS {
            for hex in colors {
                let c = shadebar_core::color::Rgba::parse_hex(hex)
                    .unwrap_or_else(|| panic!("{key} 里有无法解析的色值: {hex}"));
                assert_eq!(c.a, 255, "{hex} 必须是 6 位不透明色");
                assert!(!seen.contains(&c), "{hex} 在色板里重复了");
                seen.push(c);
            }
        }
        assert_eq!(seen.len(), 18);
    }

    #[test]
    fn english_flag_matches_feature() {
        assert_eq!(has_english(), !cfg!(feature = "no-en"));
    }
}
