//! 页面 3：采样区域微调（6 项；实时预览 S-23 尚未落地，v1.1 从 UI 收起）。
//!
//! v1.1 起采样改为"任务栏自身条带 + 三行中位数"，因此 S-20/S-21 只作用于
//! 中部图标区，S-22 表示采样带相对顶边的垂直偏移（像素）。
//! v1.2.x 起本页顶部新增三个**分区自适应开关**（S-48~S-50）：关掉某区，
//! 该区元素写回系统原色，其余分区不受影响。

use crate::config::{Lang, SettingKey};
use crate::ui::i18n;

/// 本页设置项。
pub const KEYS: &[SettingKey] = &[
    SettingKey::S48,
    SettingKey::S49,
    SettingKey::S50,
    SettingKey::S20,
    SettingKey::S21,
    SettingKey::S22,
];

/// 有效像素低于此值时在预览上叠加预警（§13-5，判据仍不跳过采样）。
pub const SMALL_REGION_VALID_PX: u32 = 1500;

/// 预览是否需要显示预警。
#[must_use]
pub const fn needs_small_region_warning(valid_px: u32, has_data: bool) -> bool {
    has_data && valid_px < SMALL_REGION_VALID_PX
}

/// 预警文案（取自 i18n 表，不硬编码在 UI 逻辑里）。
#[must_use]
pub fn warning_text(lang: Lang) -> &'static str {
    i18n::text("banner.region_small", lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Group};

    #[test]
    fn region_page_has_3_switches_and_3_sliders_in_region_group() {
        assert_eq!(KEYS.len(), 6);
        for k in KEYS {
            assert_eq!(k.meta().group, Group::Region);
        }
        // 分区开关排在滑块前面：它们决定"哪个区参与着色"，比微调更常用。
        assert_eq!(KEYS[0], SettingKey::S48);
        assert_eq!(KEYS[1], SettingKey::S49);
        assert_eq!(KEYS[2], SettingKey::S50);
    }

    #[test]
    fn small_region_only_warns_with_real_data() {
        assert!(!needs_small_region_warning(100, false), "无数据时不得虚报预警");
        assert!(needs_small_region_warning(1499, true));
        assert!(!needs_small_region_warning(1500, true));
    }

    #[test]
    fn region_matches_engine_config() {
        let mut c = Config::default();
        assert!(c.set(SettingKey::S20, 20));
        assert!(c.set(SettingKey::S21, 60));
        assert!(c.set(SettingKey::S22, 15));
        let e = c.to_engine_config();
        assert_eq!(e.region.x_offset_pct, 20);
        assert_eq!(e.region.width_pct, 60);
        assert_eq!(e.region.v_inset_pct, 15);
    }
}
