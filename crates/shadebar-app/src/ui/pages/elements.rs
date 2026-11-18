//! 页面 4：着色元素（4 项，全部已在真机验证）。
//!
//! 时间、日期、系统托盘图标三项可稳定改色；运行中指示条只改**正常态**，
//! 选中态由系统强调色接管（XAML 视觉状态会覆盖本地值，见 docs/03 §6）。
//! 悬停高亮块（S-34~S-37）尚未验证，暂时收起。
//! NFR-05 / §13-6：降级模式下这些控件**置灰但保留用户设定值、不清零**。

use crate::config::SettingKey;

/// 本页设置项。
pub const KEYS: &[SettingKey] = &[
    SettingKey::S30,
    SettingKey::S31,
    SettingKey::S32,
    SettingKey::S33,
];

/// 需要 L2/L3 能力、降级时置灰的项（与 `SettingKey::degraded_greyed` 一致）。
pub const GREYED_IN_DEGRADED: &[SettingKey] = &[
    SettingKey::S30,
    SettingKey::S31,
    SettingKey::S32,
    SettingKey::S33,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Group};
    use crate::ui::widgets::SettingWidget;

    #[test]
    fn elements_page_has_4_items_in_elements_group() {
        assert_eq!(KEYS.len(), 4);
        for k in KEYS {
            assert_eq!(k.meta().group, Group::Elements);
        }
    }

    #[test]
    fn greyed_set_matches_key_metadata() {
        for k in KEYS {
            assert_eq!(k.degraded_greyed(), GREYED_IN_DEGRADED.contains(k), "{} 置灰口径不一致", k.id());
        }
    }

    #[test]
    fn degraded_keeps_user_values_not_zeroed() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S30, 0));
        let w = SettingWidget::new(SettingKey::S30, &c, true);
        assert!(!w.enabled, "降级时置灰");
        assert_eq!(c.get(SettingKey::S30), 0, "配置本身保留用户值");
    }

    #[test]
    fn hidden_items_stay_out_of_ui_but_keep_their_fields() {
        // 收起的是 UI，不是配置：旧配置文件仍能加载且不丢值
        assert!(!SettingKey::S34.ui_visible());
        assert!(!SettingKey::S37.ui_visible());
        let mut c = Config::default();
        assert!(c.set(SettingKey::S37, 0xFF_12_34_56));
        assert_eq!(c.get(SettingKey::S37), 0xFF_12_34_56);
    }
}
