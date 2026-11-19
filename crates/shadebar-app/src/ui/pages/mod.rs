//! 六个分组页面（架构 §4 #32~#37 / §8.3 T03.4、T03.5）。
//!
//! 页面本身只是「该分组有哪些 [`SettingKey`]」的静态表；真正的控件树、
//! 布局、命中测试与重绘统一由 `ui::View` 驱动，避免 6 份重复实现。
//! 每个页面单测核对项数与分组归属，防止设置项在绑定时漏项/串组。

pub mod behavior;
pub mod color;
pub mod diagnostics;
pub mod elements;
pub mod general;
pub mod region;

use crate::config::{Group, SettingKey};

/// 按分组取该页的设置项（诊断页无设置项，返回空表）。
#[must_use]
pub fn keys_of(group: Group) -> &'static [SettingKey] {
    match group {
        Group::General => general::KEYS,
        Group::Color => color::KEYS,
        Group::Region => region::KEYS,
        Group::Elements => elements::KEYS,
        Group::Tray => behavior::TRAY_KEYS,
        Group::Behavior => behavior::BEHAVIOR_KEYS,
        Group::Diagnostics => &[],
    }
}

/// 全部设置分组（不含诊断页）按显示顺序。
pub const SETTING_GROUPS: [Group; 6] = [
    Group::General, Group::Color, Group::Region,
    Group::Elements, Group::Tray, Group::Behavior,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_cover_every_visible_setting_exactly_once() {
        let mut seen = [false; crate::config::SETTING_COUNT];
        let mut total = 0usize;
        for g in SETTING_GROUPS {
            for k in keys_of(g) {
                assert_eq!(k.meta().group, g, "{} 串到了别的分组", k.id());
                assert!(!seen[k.index()], "{} 重复绑定", k.id());
                seen[k.index()] = true;
                total += 1;
            }
        }
        // 页面表必须恰好等于「元数据 + 可见性」过滤的结果
        let visible: usize = SettingKey::ALL.iter().filter(|k| k.ui_visible()).count();
        assert_eq!(total, visible, "页面项数与可见项数不一致");
        for k in SettingKey::ALL {
            assert_eq!(seen[k.index()], k.ui_visible(), "{} 可见性绑定错误", k.id());
        }
    }

    #[test]
    fn diagnostics_page_has_no_settings() {
        assert!(keys_of(Group::Diagnostics).is_empty());
    }
}
