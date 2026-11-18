//! 任务栏几何（T02.1，架构 §8.2 / §4 #41）—— **五条硬判据的落点**。
//!
//! | 判据 | 实现位置 |
//! |---|---|
//! | ①采样矩形 100% 落在 `TrayNotifyWnd` 内，误差 ≤ 4 px（A-02） | [`compute_sample_rect`] + [`rect_clamp`] |
//! | ②自动隐藏时 `is_usable() == false`（EX-06） | [`TaskbarGeom::is_usable`] |
//! | ③竖向任务栏识别 → 暂停采样 + 提示一次（EX-07） | [`TaskbarGeom::is_vertical`] + [`Engine::emit_notice_once`] |
//! | ④多显示器下只取含任务栏的屏（NFR-07 / EX-08） | [`TaskbarGeom::from_probe`] 选屏 |
//! | ⑤DPI 切换后 ≤ 2 s 重算（NFR-08 / EX-22） | [`TaskbarGeom::needs_refresh_from`] + [`Engine::on_dpi_changed`] |
//!
//! # 分层
//!
//! 平台侧只需提供 [`TaskbarSource::read`] 返回一份 [`TaskbarProbe`] 原始读数
//! （`FindWindowW` / `FindWindowExW` / `GetWindowRect` / `MonitorFromWindow` /
//! `GetDpiForWindow` / `SHAppBarMessage` 由 T04 注入实现，且统一经 `win/mod.rs` 转
//! `SbResult<T>`，见 §11.9）。本文件的**选屏、判定、采样矩形计算全部是纯逻辑**，
//! 单测用 `testing::FakeTaskbar` 注入假几何，不依赖真实 explorer / 屏幕。
//!
//! 所有坐标均为**物理像素**（§11.4）；`dpi` 只用于换算与变化检测，绝不把 dip
//! 传进采样链路。

use crate::engine::{
    MAX_SCREENS, SAMPLE_RECT_TOLERANCE_PX, RectPx, RegionCfg, rect_area, rect_clamp, rect_contains,
    rect_height, rect_intersection, rect_is_empty, rect_px, rect_width,
};
use crate::engine::screen::ScreenInfo;
use crate::error::{SbError, SbResult};

/// 平台侧一次任务栏探测的原始读数（全部物理像素）。
///
/// 固定容量 [`MAX_SCREENS`] 数组 + 有效长度，**零堆分配**，可 `Copy`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarProbe {
    /// `Shell_TrayWnd` 窗口矩形（px）。
    pub bar: RectPx,
    /// `TrayNotifyWnd` 客户区矩形（px）——A-02 的包含基准。
    pub tray: RectPx,
    /// 任务栏是否处于自动隐藏状态（`ABM_GETSTATE & ABS_AUTOHIDE`）。
    pub auto_hide: bool,
    /// 任务栏所在显示器的 DPI（96 = 100%）。
    pub dpi: u32,
    /// 枚举到的显示器列表。
    pub monitors: [ScreenInfo; MAX_SCREENS],
    /// `monitors` 的有效长度。
    pub monitors_len: usize,
    /// Win11 托盘折叠状态（折叠时 `TrayNotifyWnd` 宽度显著变小）。
    pub tray_collapsed: bool,
}

impl Default for TaskbarProbe {
    fn default() -> Self {
        Self {
            bar: rect_px(0, 0, 0, 0),
            tray: rect_px(0, 0, 0, 0),
            auto_hide: false,
            dpi: 96,
            monitors: std::array::from_fn(|_| ScreenInfo::default()),
            monitors_len: 0,
            tray_collapsed: false,
        }
    }
}

impl TaskbarProbe {
    /// 有效显示器切片。
    #[must_use]
    pub fn monitors(&self) -> &[ScreenInfo] {
        &self.monitors[..self.monitors_len.min(MAX_SCREENS)]
    }

    /// 由所有显示器计算出的拓扑指纹（用于 EX-22 插拔显示器检测）。
    #[must_use]
    pub fn topology_fingerprint(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for m in self.monitors() {
            for v in [
                m.bounds.left as u64,
                m.bounds.top as u64,
                m.bounds.right as u64,
                m.bounds.bottom as u64,
                u64::from(m.dpi),
                m.monitor as u64,
            ] {
                h ^= v.wrapping_add(0x9e37_79b9_7f4a_7c15);
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }
}

/// 任务栏几何平台能力（薄 trait）。
pub trait TaskbarSource {
    /// 读一次任务栏几何。
    ///
    /// # Errors
    ///
    /// 找不到 `Shell_TrayWnd` / `TrayNotifyWnd` 时返回 [`SbError::TaskbarNotFound`]。
    fn read(&mut self) -> SbResult<TaskbarProbe>;
}

/// 经纯逻辑归一化后的任务栏几何（架构 §5.1 `TaskbarGeom`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarGeom {
    /// `Shell_TrayWnd` 矩形（px）。
    pub bar: RectPx,
    /// `TrayNotifyWnd` 矩形（px）。
    pub tray: RectPx,
    /// 自动隐藏。
    pub auto_hide: bool,
    /// 水平任务栏（false = 竖向）。
    pub horizontal: bool,
    /// DPI。
    pub dpi: u32,
    /// 含任务栏的那块显示器的 `HMONITOR`。
    pub monitor: isize,
    /// **含任务栏那块屏的 bounds**（NFR-07：只对这一屏采样）。
    pub screen: RectPx,
    /// 显示器拓扑指纹。
    pub topology: u64,
    /// 托盘折叠状态。
    pub tray_collapsed: bool,
}

impl Default for TaskbarGeom {
    fn default() -> Self {
        Self {
            bar: rect_px(0, 0, 0, 0),
            tray: rect_px(0, 0, 0, 0),
            auto_hide: false,
            horizontal: true,
            dpi: 96,
            monitor: 0,
            screen: rect_px(0, 0, 0, 0),
            topology: 0,
            tray_collapsed: false,
        }
    }
}

impl TaskbarGeom {
    /// 从平台原始读数归一化几何。
    ///
    /// 选屏顺序（NFR-07 / EX-08）：
    /// 1. 优先选**完整包含** `tray` 的显示器；多个时主屏优先；
    /// 2. 否则选与 `tray` 交集面积最大的显示器；
    /// 3. 若枚举为空，退回主屏（无则第一个），再退回 `bar` 自身所在的虚拟屏。
    ///
    /// # Errors
    ///
    /// `tray` 或 `bar` 为空时返回 [`SbError::TaskbarNotFound`]。
    pub fn from_probe(probe: &TaskbarProbe) -> SbResult<Self> {
        if rect_is_empty(&probe.tray) || rect_is_empty(&probe.bar) {
            return Err(SbError::TaskbarNotFound);
        }
        let chosen = choose_screen(&probe.tray, probe.monitors());
        let (screen, monitor) = match chosen {
            Some(s) => (s.bounds, s.monitor),
            None => (probe.bar, 0),
        };
        let horizontal = rect_width(&probe.bar) >= rect_height(&probe.bar);
        Ok(Self {
            bar: probe.bar,
            tray: probe.tray,
            auto_hide: probe.auto_hide,
            horizontal,
            dpi: probe.dpi,
            monitor,
            screen,
            topology: probe.topology_fingerprint(),
            tray_collapsed: probe.tray_collapsed,
        })
    }

    /// 采样是否可用（EX-06 / EX-07 的核心判据）。
    ///
    /// `false` 的四种原因：自动隐藏、竖向任务栏、`tray` 退化、任务栏落在屏幕外。
    /// 调用点必须**暂停采样并保持上次判定色**，不得用桌面像素顶替。
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.auto_hide
            && self.horizontal
            && !rect_is_empty(&self.tray)
            && !rect_is_empty(&self.bar)
            && rect_intersection(&self.tray, &self.screen).is_some()
    }

    /// 是否自动隐藏（EX-06）。
    #[must_use]
    pub const fn is_auto_hidden(&self) -> bool {
        self.auto_hide
    }

    /// 是否竖向任务栏（EX-07）。
    #[must_use]
    pub const fn is_vertical(&self) -> bool {
        !self.horizontal
    }

    /// 该几何是否需要按新读数重算（NFR-08 / EX-22 / 托盘折叠展开）。
    ///
    /// DPI、任务栏/托盘矩形、自动隐藏、折叠状态、显示器拓扑任一变化都返回 `true`。
    #[must_use]
    pub fn needs_refresh_from(&self, probe: &TaskbarProbe) -> bool {
        self.dpi != probe.dpi
            || self.bar != probe.bar
            || self.tray != probe.tray
            || self.auto_hide != probe.auto_hide
            || self.tray_collapsed != probe.tray_collapsed
            || self.topology != probe.topology_fingerprint()
    }
}

/// 选择含任务栏的那块显示器（NFR-07：绝不因读到副屏像素而误判）。
fn choose_screen<'a>(tray: &RectPx, monitors: &'a [ScreenInfo]) -> Option<&'a ScreenInfo> {
    let mut containing: Option<&ScreenInfo> = None;
    for m in monitors {
        if rect_contains(&m.bounds, tray) {
            match containing {
                None => containing = Some(m),
                Some(cur) if !cur.primary && m.primary => containing = Some(m),
                _ => {}
            }
        }
    }
    if containing.is_some() {
        return containing;
    }
    let mut best: Option<&ScreenInfo> = None;
    let mut best_area = 0u32;
    for m in monitors {
        if let Some(part) = rect_intersection(tray, &m.bounds) {
            let area = rect_area(&part);
            if area > best_area {
                best_area = area;
                best = Some(m);
            }
        }
    }
    if best.is_some() {
        return best;
    }
    monitors.iter().find(|m| m.primary).or_else(|| monitors.first())
}

/// 定位任务栏几何（架构 §6.2 `locate()` 的注入式形态）。
///
/// 与 §6.2 的偏离：把 `TaskbarSource` 显式注入，使 core 不含任何 Win32 调用，
/// 因而可脱离 Windows 消息循环单测（§11.7）。T04 接线时封装一层无参 `locate()` 即可。
///
/// # Errors
///
/// 平台读数失败或几何退化时返回 [`SbError::TaskbarNotFound`]。
pub fn locate(source: &mut dyn TaskbarSource) -> SbResult<TaskbarGeom> {
    let probe = source.read()?;
    TaskbarGeom::from_probe(&probe)
}

/// 是否自动隐藏（架构 §6.2 `is_taskbar_auto_hidden` 签名）。
#[must_use]
pub const fn is_taskbar_auto_hidden(g: &TaskbarGeom) -> bool {
    g.is_auto_hidden()
}

/// 是否竖向（架构 §6.2 `is_vertical` 签名）。
#[must_use]
pub const fn is_vertical(g: &TaskbarGeom) -> bool {
    g.is_vertical()
}

/// 计算理论期望的采样矩形（**未夹取**），供 A-02 误差核对与诊断面板可视化。
///
/// 与 [`compute_sample_rect`] 用同一套公式，唯一差别是这里不做夹取，
/// 因此两者之差就是"误差"，必须 ≤ [`SAMPLE_RECT_TOLERANCE_PX`]。
#[must_use]
pub fn expected_sample_rect(g: &TaskbarGeom, r: &RegionCfg) -> Option<RectPx> {
    if rect_is_empty(&g.tray) {
        return None;
    }
    let w = rect_width(&g.tray);
    let h = rect_height(&g.tray);
    let x_off = (w as i64 * i64::from(r.x_offset_pct) / 100) as i32;
    let sw = (w as i64 * i64::from(r.width_pct) / 100) as i32;
    let inset = (h as i64 * i64::from(r.v_inset_pct) / 100) as i32;
    let desired = rect_px(
        g.tray.left.saturating_add(x_off),
        g.tray.top.saturating_add(inset),
        g.tray.left.saturating_add(x_off).saturating_add(sw),
        g.tray.bottom.saturating_sub(inset),
    );
    if rect_is_empty(&desired) {
        None
    } else {
        Some(desired)
    }
}

/// 计算采样矩形（**A-02 的核心**）。
///
/// 步骤：
/// 1. 按 [`RegionCfg`] 百分比在 `TrayNotifyWnd` 内算出期望矩形；
/// 2. 用 `rect_clamp` 把矩形**夹回 `tray` 之内** → 结果 100% 落在托盘区，误差 0 px；
/// 3. 再夹到含任务栏那块屏 `geom.screen`（EX-24：不得读到屏外 / 副屏像素）；
/// 4. 最终再验证一次 `rect_contains(tray, result)`，不满足返回 `None`。
///
/// 返回 `None` 的场合：几何不可用、矩形退化、采样区完全在屏幕外。调用点按
/// §13-5 跳过本拍并保持上次判定，**不得报错、不得弹窗**。
#[must_use]
pub fn compute_sample_rect(g: &TaskbarGeom, r: &RegionCfg) -> Option<RectPx> {
    if !g.is_usable() {
        return None;
    }
    let desired = expected_sample_rect(g, r)?;
    let in_tray = rect_clamp(&desired, &g.tray)?;
    let in_screen = rect_clamp(&in_tray, &g.screen)?;
    if !rect_contains(&g.tray, &in_screen) {
        return None;
    }
    Some(in_screen)
}

/// 采样矩形与理论期望的四边误差（px），用于 A-02「误差 ≤ 4 px」的可观测证据。
#[must_use]
pub fn sample_rect_error_px(expected: &RectPx, actual: &RectPx) -> [i32; 4] {
    [
        actual.left.saturating_sub(expected.left).abs(),
        actual.top.saturating_sub(expected.top).abs(),
        actual.right.saturating_sub(expected.right).abs(),
        actual.bottom.saturating_sub(expected.bottom).abs(),
    ]
}

/// 四边误差是否全部在 A-02 容差内。
#[must_use]
pub fn error_within_tolerance(errors: &[i32; 4]) -> bool {
    errors.iter().all(|&e| e <= SAMPLE_RECT_TOLERANCE_PX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{FakeTaskbar, alloc_count, reset_alloc_count};

    #[test]
    fn a02_sample_rect_is_fully_inside_tray_within_4px() {
        // A-02 / 判据①：覆盖多种托盘宽度、DPI、偏移/宽度/内缩组合。
        let cases = [
            (300, 48, 0, 100, 10),
            (300, 48, 10, 50, 20),
            (180, 40, 25, 40, 5),
            (450, 72, 3, 90, 45),
            (130, 32, 0, 33, 0),
        ];
        for (tw, th, xo, wp, vi) in cases {
            let mut src = FakeTaskbar::desktop_1080p();
            src.set_tray_size(tw, th);
            let g = locate(&mut src).expect("几何应有效");
            let region = RegionCfg {
                x_offset_pct: xo,
                width_pct: wp,
                v_inset_pct: vi,
            };
            let got = compute_sample_rect(&g, &region).expect("采样矩形应存在");
            assert!(
                rect_contains(&g.tray, &got),
                "100% 落在 TrayNotifyWnd 内：tray={:?} got={:?}",
                g.tray,
                got
            );
            let want = expected_sample_rect(&g, &region).expect("期望矩形");
            let err = sample_rect_error_px(&want, &got);
            assert!(
                error_within_tolerance(&err),
                "误差 ≤4px：case=({tw},{th},{xo},{wp},{vi}) err={err:?}"
            );
        }
    }

    #[test]
    fn a02_wild_region_config_is_clamped_back_into_tray() {
        let mut src = FakeTaskbar::desktop_1080p();
        let g = locate(&mut src).expect("几何应有效");
        let region = RegionCfg {
            x_offset_pct: 90,
            width_pct: 100,
            v_inset_pct: 45,
        };
        let got = compute_sample_rect(&g, &region).expect("夹取后仍应有效");
        assert!(rect_contains(&g.tray, &got));
        assert_eq!(got.right, g.tray.right);
    }

    #[test]
    fn ex06_auto_hide_makes_geometry_unusable() {
        // EX-06：自动隐藏时 is_usable() == false，且不得计算出采样矩形。
        let mut src = FakeTaskbar::desktop_1080p();
        src.set_auto_hide(true);
        let g = locate(&mut src).expect("几何仍可读出");
        assert!(g.is_auto_hidden());
        assert!(!g.is_usable(), "EX-06 判据②");
        assert!(is_taskbar_auto_hidden(&g));
        assert_eq!(compute_sample_rect(&g, &RegionCfg::default()), None);
    }

    #[test]
    fn ex07_vertical_taskbar_is_detected() {
        // EX-07 / 判据③：竖向任务栏识别（暂停 + 提示在 Engine 层验证）。
        let mut src = FakeTaskbar::vertical();
        let g = locate(&mut src).expect("几何仍可读出");
        assert!(g.is_vertical());
        assert!(is_vertical(&g));
        assert!(!g.horizontal);
        assert!(!g.is_usable());
        assert_eq!(compute_sample_rect(&g, &RegionCfg::default()), None);
    }

    #[test]
    fn nfr07_multi_monitor_uses_only_the_screen_with_taskbar() {
        // NFR-07 / EX-08 / 判据④：副屏在左、主屏在中、副屏在右。
        let mut src = FakeTaskbar::three_monitors_taskbar_on_primary();
        let g = locate(&mut src).expect("几何应有效");
        assert_eq!(g.screen, rect_px(1920, 0, 3840, 1080));
        assert_eq!(g.monitor, 2);
        let rect = compute_sample_rect(&g, &RegionCfg::default()).expect("采样矩形");
        assert!(
            rect_contains(&rect_px(1920, 0, 3840, 1080), &rect),
            "不得采样到副屏像素：{rect:?}"
        );
        assert!(rect_contains(&g.tray, &rect));
    }

    #[test]
    fn nfr07_taskbar_on_secondary_screen_is_supported() {
        let mut src = FakeTaskbar::three_monitors_taskbar_on_primary();
        src.move_taskbar_to_left_screen();
        let g = locate(&mut src).expect("几何应有效");
        assert_eq!(g.screen, rect_px(-1920, 0, 0, 1080));
        let rect = compute_sample_rect(&g, &RegionCfg::default()).expect("采样矩形");
        assert!(rect_contains(&g.screen, &rect));
        assert!(rect_contains(&g.tray, &rect));
    }

    #[test]
    fn nfr08_dpi_change_is_detected_and_recomputed() {
        // NFR-08 / EX-22 / 判据⑤：100% ↔ 150% 切换。
        let mut src = FakeTaskbar::desktop_1080p();
        let g96 = locate(&mut src).expect("96 dpi");
        assert_eq!(g96.dpi, 96);
        assert!(!g96.needs_refresh_from(&src.probe()), "未变化不得反复重算");

        src.set_dpi(144);
        assert!(g96.needs_refresh_from(&src.probe()), "DPI 变化必须触发重算");
        let g144 = locate(&mut src).expect("144 dpi");
        assert_eq!(g144.dpi, 144);
        assert!(!g144.needs_refresh_from(&src.probe()));

        // 150% 下托盘区按物理像素变大，采样矩形随之变大且仍在 tray 内。
        let rect = compute_sample_rect(&g144, &RegionCfg::default()).expect("采样矩形");
        assert!(rect_contains(&g144.tray, &rect));
        assert!(rect_width(&rect) >= rect_width(&g96.tray) * 6 / 10);
    }

    #[test]
    fn ex22_monitor_hotplug_changes_topology_fingerprint() {
        let mut src = FakeTaskbar::desktop_1080p();
        let g = locate(&mut src).expect("几何应有效");
        src.add_secondary_screen();
        assert!(
            g.needs_refresh_from(&src.probe()),
            "插拔显示器必须触发几何重算（EX-22）"
        );
        let g2 = locate(&mut src).expect("重算后几何");
        assert_ne!(g.topology, g2.topology);
    }

    #[test]
    fn tray_collapse_and_expand_keep_rect_inside_tray() {
        // 托盘折叠/展开：宽度骤变，采样矩形必须重算且仍在 tray 内。
        let mut src = FakeTaskbar::desktop_1080p();
        let expanded = locate(&mut src).expect("展开态");
        let rect_expanded = compute_sample_rect(&expanded, &RegionCfg::default()).expect("矩形");
        assert!(rect_contains(&expanded.tray, &rect_expanded));

        src.set_tray_collapsed(true);
        assert!(expanded.needs_refresh_from(&src.probe()));
        let collapsed = locate(&mut src).expect("折叠态");
        let rect_collapsed = compute_sample_rect(&collapsed, &RegionCfg::default()).expect("矩形");
        assert!(rect_contains(&collapsed.tray, &rect_collapsed));
        assert!(
            rect_width(&rect_collapsed) < rect_width(&rect_expanded),
            "折叠后采样区应随之变窄"
        );
    }

    #[test]
    fn taskbar_missing_returns_error_not_panic() {
        // EX-11：explorer 重启期间窗口暂时不存在。
        let mut src = FakeTaskbar::missing();
        assert!(matches!(locate(&mut src), Err(SbError::TaskbarNotFound)));
        let probe = TaskbarProbe::default();
        assert!(TaskbarGeom::from_probe(&probe).is_err());
    }

    #[test]
    fn offscreen_tray_is_unusable() {
        let mut src = FakeTaskbar::desktop_1080p();
        src.set_offscreen_tray();
        let g = locate(&mut src).expect("几何仍可构造");
        assert!(!g.is_usable());
        assert_eq!(compute_sample_rect(&g, &RegionCfg::default()), None);
    }

    #[test]
    fn locate_and_compute_are_allocation_free() {
        let mut src = FakeTaskbar::desktop_1080p();
        let g = locate(&mut src).expect("几何");
        let region = RegionCfg::default();
        reset_alloc_count();
        for _ in 0..600 {
            let _ = compute_sample_rect(&g, &region);
            let _ = g.needs_refresh_from(&src.probe());
        }
        assert_eq!(alloc_count(), 0, "几何热路径不得分配");
    }

    #[test]
    fn expected_and_actual_agree_on_default_config() {
        let mut src = FakeTaskbar::desktop_1080p();
        let g = locate(&mut src).expect("几何");
        let region = RegionCfg::default();
        let want = expected_sample_rect(&g, &region).expect("期望");
        let got = compute_sample_rect(&g, &region).expect("实际");
        assert_eq!(sample_rect_error_px(&want, &got), [0, 0, 0, 0]);
    }
}
