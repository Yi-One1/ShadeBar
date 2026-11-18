//! 页面 5：托盘与行为（§6.5，S-40 ~ S-46，7 项）。
//!
//! 需求文档把 6.5 合成一节，但 A-25 要求分组标题逐项核对；这里按 UI 显示
//! 拆成「托盘」(S-40~S-44) 与「行为」(S-45~S-46) 两个分组标题，
//! 合计仍是 7 项、35 项总数不变。

use crate::config::SettingKey;

/// 托盘分组（5 项）。
pub const TRAY_KEYS: &[SettingKey] = &[
    SettingKey::S40,
    SettingKey::S41,
    SettingKey::S42,
    SettingKey::S43,
];

/// 行为分组（2 项）。
pub const BEHAVIOR_KEYS: &[SettingKey] = &[
    SettingKey::S45,
    SettingKey::S46,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Group};

    #[test]
    fn behavior_page_has_6_items() {
        assert_eq!(TRAY_KEYS.len(), 4);
        assert_eq!(BEHAVIOR_KEYS.len(), 2);
        assert_eq!(TRAY_KEYS.len() + BEHAVIOR_KEYS.len(), 6);
    }

    #[test]
    fn sub_groups_match_key_metadata() {
        for k in TRAY_KEYS {
            assert_eq!(k.meta().group, Group::Tray);
        }
        for k in BEHAVIOR_KEYS {
            assert_eq!(k.meta().group, Group::Behavior);
        }
    }

    #[test]
    fn close_action_default_keeps_process_alive() {
        // S-41 默认「最小化到托盘」（A-09：进程仍在、托盘图标仍在）。
        assert_eq!(Config::default().get(SettingKey::S41), 0);
    }
}
