//! 页面 1：通用（§6.1，S-01 ~ S-06，6 项）。

use crate::config::SettingKey;

/// 本页设置项。
pub const KEYS: &[SettingKey] = &[
    SettingKey::S01,
    SettingKey::S02,
    SettingKey::S03,
    SettingKey::S04,
    SettingKey::S05,
    SettingKey::S06,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Group;

    #[test]
    fn general_page_has_6_items_in_general_group() {
        assert_eq!(KEYS.len(), 6);
        for k in KEYS {
            assert_eq!(k.meta().group, Group::General);
        }
    }

    #[test]
    fn s02_defaults_off_and_s04_follows_system() {
        let c = crate::config::Config::default();
        assert_eq!(c.get(SettingKey::S02), 0);
        assert_eq!(c.get(SettingKey::S04), 0);
    }
}
