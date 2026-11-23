//! 设置窗口的四个页面及其设置项归属。
//!
//! 页面结构按"作用对象"划分，而不是按需求文档的分组：
//!
//! | 页 | 内容 |
//! |---|---|
//! | 任务栏 | 实时预览 + 三区开关 + 颜色与判定 |
//! | 采样 | 采样带可视化 + 间隔/平滑/区域微调 |
//! | 托盘 | 托盘行为 + 启动 + 语言 |
//! | 诊断 | 实时判定 + 日志级别 + 版本 + 诊断包 |
//!
//! 这里只列出"以设置行/控件形式出现"的键；S-05（版本）与 S-46（导出诊断包）
//! 是诊断页上的固定动作，不参与设置行归属。

pub mod diagnostics;
pub mod region;

use crate::config::SettingKey as K;

/// 任务栏页。页面本身由预览/三区/颜色卡承载，这里仅供页面归属核对。
#[cfg(test)]
pub const TASKBAR_KEYS: &[K] = &[
    K::S01, K::S10, K::S12, K::S14, K::S15, K::S47, K::S48, K::S49, K::S50,
];

/// 采样页。
pub const SAMPLE_KEYS: &[K] = &[K::S16, K::S18, K::S20, K::S21, K::S22];

/// 托盘页。
pub const TRAY_KEYS: &[K] = &[K::S40, K::S51, K::S41, K::S02, K::S03, K::S04];

/// 诊断页。
#[cfg(test)]
pub const DIAG_KEYS: &[K] = &[K::S45];

/// 四个页面，顺序与页签一致。
#[cfg(test)]
pub const PAGES: [&[K]; 4] = [TASKBAR_KEYS, SAMPLE_KEYS, TRAY_KEYS, DIAG_KEYS];

/// 设置项所在的页（不可见的项返回 `None`）。
#[cfg(test)]
#[must_use]
pub fn page_of(key: K) -> Option<usize> {
    PAGES.iter().position(|keys| keys.contains(&key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_visible_key_appears_on_exactly_one_page() {
        let mut seen = [0u8; crate::config::SETTING_COUNT];
        for keys in PAGES {
            for &k in keys {
                seen[k.index()] += 1;
                assert!(k.ui_visible(), "{} 已从 UI 收起，却仍绑在页面里", k.id());
            }
        }
        for k in crate::config::SettingKey::ALL {
            assert_eq!(seen[k.index()], u8::from(k.ui_visible()), "{} 页面归属错误", k.id());
        }
    }

    #[test]
    fn retired_keys_have_no_page() {
        for k in [
            crate::config::SettingKey::S05,
            crate::config::SettingKey::S06,
            crate::config::SettingKey::S11,
            crate::config::SettingKey::S13,
            crate::config::SettingKey::S17,
            crate::config::SettingKey::S19,
            crate::config::SettingKey::S23,
            crate::config::SettingKey::S30,
            crate::config::SettingKey::S34,
            crate::config::SettingKey::S35,
            crate::config::SettingKey::S36,
            crate::config::SettingKey::S37,
            crate::config::SettingKey::S42,
            crate::config::SettingKey::S43,
            crate::config::SettingKey::S44,
            crate::config::SettingKey::S46,
        ] {
            assert_eq!(page_of(k), None, "{} 不应再出现在设置页", k.id());
        }
    }

    #[test]
    fn page_order_matches_tab_order() {
        assert_eq!(page_of(crate::config::SettingKey::S48), Some(0));
        assert_eq!(page_of(crate::config::SettingKey::S16), Some(1));
        assert_eq!(page_of(crate::config::SettingKey::S40), Some(2));
        assert_eq!(page_of(crate::config::SettingKey::S45), Some(3));
    }
}
