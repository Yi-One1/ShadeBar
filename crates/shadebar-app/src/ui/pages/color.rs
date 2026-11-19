//! 页面 2：配色与采样（9 项；主题联动 S-19 尚未落地，v1.1 从 UI 收起）。

use crate::config::SettingKey;

/// 本页设置项。
pub const KEYS: &[SettingKey] = &[
    SettingKey::S10,
    SettingKey::S11,
    SettingKey::S12,
    SettingKey::S13,
    SettingKey::S14,
    SettingKey::S15,
    SettingKey::S16,
    SettingKey::S17,
    SettingKey::S18,
    SettingKey::S47,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Group};

    #[test]
    fn color_page_has_9_items_in_color_group() {
        assert_eq!(KEYS.len(), 10);
        for k in KEYS {
            assert_eq!(k.meta().group, Group::Color);
        }
    }

    #[test]
    fn s19_is_hidden_from_ui() {
        // 主题联动会连动开始菜单与控制中心，侵入感强；未落地前不出现在设置页
        assert!(!SettingKey::S19.ui_visible());
        assert!(!KEYS.contains(&SettingKey::S19));
    }

    #[test]
    fn interval_default_is_1000ms() {
        assert_eq!(Config::default().get(SettingKey::S16), 1000);
    }
}
