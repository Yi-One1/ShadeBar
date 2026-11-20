//! 输入事件与浮层类型。
//!
//! 设置窗口的控件由 `ui::View` 按页面直接绘制/命中（布局本身承担解释职责，
//! 没有通用的"标签 + 说明 + 控件"行模板），这里只保留窗口层与视图层之间的
//! 消息归一类型。

use crate::config::SettingKey;

// 虚拟键值：直接写 Win32 常量数值，避免为几个常量新开 windows feature。
/// Tab。
pub const VK_TAB: u16 = 0x09;
/// Space。
pub const VK_SPACE: u16 = 0x20;
/// Enter。
pub const VK_RETURN: u16 = 0x0D;
/// Left。
pub const VK_LEFT: u16 = 0x25;
/// Up。
pub const VK_UP: u16 = 0x26;
/// Right。
pub const VK_RIGHT: u16 = 0x27;
/// Down。
pub const VK_DOWN: u16 = 0x28;
/// Escape。
pub const VK_ESCAPE: u16 = 0x1B;

/// 需要 `View` 弹出的浮层 / 立即动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// 下拉选项列表。
    Dropdown(SettingKey),
    /// 颜色选择（预设色块 + 更多颜色）。
    ColorPicker(SettingKey),
    /// 恢复默认二次确认（A-12）。
    ConfirmRestore,
    /// 关于页。
    About,
}
