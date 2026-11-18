//! UIA 遮罩发现（T02.3，架构 §8.2 / §4 #43）。
//!
//! # 判据对照
//!
//! | 判据 | 落点 |
//! |---|---|
//! | ①量出时钟与日期两个文字矩形，膨胀 2 px | [`ClockRectSource`] + [`MaskSet::union_with`] |
//! | ②UIA 不可用（禁用 / 超时 300 ms）→ 退回固定几何，功能不降级 | [`UiaProbe::refresh_into`] + [`fallback_clock_rects`] |
//! | ③UIA 不阻塞 T0/T1 | [`UiaProbe`] 熔断 + [`UiaConnectionConfig`] 下发 `put_ConnectionTimeout` |
//! | ④遮罩合并行为单测 | `mask.rs` 与本文单测 |
//!
//! # 分层与 §11.10 纪律
//!
//! 真实 `IUIAutomation` 调用由 T04 在**独立 UIA 线程（MTA）**实现并注入：
//! `CoCreateInstance` → `IUIAutomation2::put_ConnectionTimeout(300)` →
//! `ElementFromHandle(Shell_TrayWnd)` → 按 Name 找 `TimeInnerTextBlock` /
//! `DateInnerTextBlock` → `CurrentBoundingRectangle`。
//!
//! 本文件**严禁缓存任何元素引用**：每次 refresh 只把结果落成纯 `MaskRect` 值，
//! 元素指针当次有效当次使用、不跨时点持有（§11.10 UAF 教训）。core 侧完全不碰
//! COM，因而单测可用 `testing::FakeUia` 注入假矩形。

use crate::engine::{
    FALLBACK_CLOCK_H_DIP, FALLBACK_CLOCK_W_DIP, RectPx, UIA_REFRESH_INTERVAL_MS, UIA_TIMEOUT_MS,
    dip_to_px, rect_clamp, rect_is_empty, rect_px,
};
use crate::engine::mask::{MaskKind, MaskRect, MaskSet};
use crate::engine::taskbar::TaskbarGeom;
use crate::engine::sampler::Clock;
use crate::error::{SbError, SbResult};

/// UIA 不可用时的 HRESULT（`REGDB_E_CLASSNOTREG`），仅用于日志与错误携带。
pub const UIA_UNAVAILABLE_HR: i32 = 0x8004_0102u32 as i32;

/// 真实 UIA 实现的连接配置（由 core 下发，T04 注入到 COM 侧）。
///
/// `connection_timeout_ms` 对应 `IUIAutomation2::put_ConnectionTimeout`：
/// 限制单次跨进程 UIA 查询的等待，避免 T0/T1 被 explorer 拖住（判据③）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiaConnectionConfig {
    /// 连接 / 查询超时（毫秒）。
    pub connection_timeout_ms: u32,
    /// 遮罩刷新周期（毫秒，冷路径）。
    pub refresh_interval_ms: u64,
}

impl Default for UiaConnectionConfig {
    fn default() -> Self {
        Self {
            connection_timeout_ms: UIA_TIMEOUT_MS as u32,
            refresh_interval_ms: UIA_REFRESH_INTERVAL_MS,
        }
    }
}

/// UIA 矩形探测能力（薄 trait）。
///
/// 实现者应当在 `out` 里合并**未膨胀**的原始屏幕矩形（px）；膨胀 2 px 由
/// [`MaskSet::union_with`] 统一完成，避免双重膨胀。
pub trait ClockRectSource {
    /// 刷新一次时钟 / 日期矩形。
    ///
    /// # Errors
    ///
    /// UIA 被禁用、`CoCreateInstance` 失败、超时熔断时返回 [`SbError::UiaUnavailable`]。
    fn refresh_into(&mut self, out: &mut MaskSet) -> SbResult<()>;
}

/// 固定几何 fallback：`TrayNotifyWnd` 右侧 130 × 44 dip（T02.3 判据②）。
///
/// 返回时钟与日期**两个**不相邻的原始矩形（中间的空隙保证合并后仍可分辨），
/// 由调用方 `union_with` 膨胀 2 px。所有输出都夹在 `tray` 之内，绝不越界读屏。
#[must_use]
pub fn fallback_clock_rects(g: &TaskbarGeom) -> [MaskRect; 2] {
    let w = dip_to_px(g.dpi, FALLBACK_CLOCK_W_DIP).round() as i32;
    let h = dip_to_px(g.dpi, FALLBACK_CLOCK_H_DIP).round() as i32;
    let right = g.tray.right;
    let left = right.saturating_sub(w.max(1));
    let top = g.tray.top;

    // 上 40% 为时钟、下 40% 为日期、中间 20% 留空，避免 2 px 膨胀后被合并成一个。
    let clock_raw = rect_px(left, top, right, top.saturating_add(h * 2 / 5));
    let date_raw = rect_px(left, top.saturating_add(h * 3 / 5), right, top.saturating_add(h));
    let clock = rect_clamp(&clock_raw, &g.tray).unwrap_or(rect_px(0, 0, 0, 0));
    let date = rect_clamp(&date_raw, &g.tray).unwrap_or(rect_px(0, 0, 0, 0));
    [
        MaskRect::new(clock, MaskKind::Clock),
        MaskRect::new(date, MaskKind::Date),
    ]
}

/// UIA 探测器：超时熔断 + 连续失败熔断 + 自动 fallback。
///
/// 状态机：`Closed`（正常 UIA）→ 超时或连续 3 次失败 → `Open`（直接用固定几何，
/// 不再调用 UIA，彻底避免拖慢 T0/T1）→ T04 在 explorer 变化时调 [`UiaProbe::reset_circuit`]
/// 恢复半开重试。
pub struct UiaProbe {
    in_fallback: bool,
    circuit_open: bool,
    consecutive_failures: u32,
    last_elapsed_ms: u64,
    refreshes: u64,
    fallbacks: u64,
}

impl Default for UiaProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl UiaProbe {
    /// 新建（初始闭合）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            in_fallback: false,
            circuit_open: false,
            consecutive_failures: 0,
            last_elapsed_ms: 0,
            refreshes: 0,
            fallbacks: 0,
        }
    }

    /// 当前是否使用固定几何（T02.3 判据②的可观测证据）。
    #[must_use]
    pub const fn in_fallback(&self) -> bool {
        self.in_fallback
    }

    /// 熔断是否已打开。
    #[must_use]
    pub const fn circuit_open(&self) -> bool {
        self.circuit_open
    }

    /// 上次刷新耗时（毫秒）。
    #[must_use]
    pub const fn last_elapsed_ms(&self) -> u64 {
        self.last_elapsed_ms
    }

    /// 累计刷新次数。
    #[must_use]
    pub const fn refreshes(&self) -> u64 {
        self.refreshes
    }

    /// 累计 fallback 次数。
    #[must_use]
    pub const fn fallbacks(&self) -> u64 {
        self.fallbacks
    }

    /// 恢复半开：允许下一次刷新重新尝试真实 UIA（explorer 变化 / 用户手动刷新时调用）。
    pub fn reset_circuit(&mut self) {
        self.circuit_open = false;
        self.consecutive_failures = 0;
    }

    /// 执行一次遮罩刷新。
    ///
    /// 用注入的 [`Clock`] 测量耗时；`> UIA_TIMEOUT_MS` 立即熔断并丢弃本次结果
    /// （即使 UIA 最终返回了矩形也**不采信**——耗时超标的查询本身就是阻塞风险）。
    /// 失败 / 超时 / 量不出矩形三种情况都退回 [`fallback_clock_rects`]，采样继续。
    pub fn refresh_into(
        &mut self,
        source: &mut dyn ClockRectSource,
        clock: &dyn Clock,
        geom: &TaskbarGeom,
        out: &mut MaskSet,
    ) {
        self.refreshes += 1;
        if self.circuit_open {
            self.apply_fallback(out, geom);
            return;
        }

        out.clear();
        let start = clock.now_ms();
        let result = source.refresh_into(out);
        let elapsed = clock.now_ms().saturating_sub(start);
        self.last_elapsed_ms = elapsed;

        let timed_out = elapsed > UIA_TIMEOUT_MS;
        let empty = result.is_ok() && out.count() == 0;
        if result.is_err() || timed_out || empty {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            // 超时立即熔断；普通失败连续 3 次才熔断（避免一次抖动就永久降级）。
            if timed_out || self.consecutive_failures >= 3 {
                self.circuit_open = true;
            }
            self.apply_fallback(out, geom);
        } else {
            self.consecutive_failures = 0;
            self.in_fallback = false;
        }
    }

    fn apply_fallback(&mut self, out: &mut MaskSet, geom: &TaskbarGeom) {
        out.clear();
        out.union_with(&fallback_clock_rects(geom));
        self.in_fallback = true;
        self.fallbacks += 1;
    }
}

/// 独立刷新入口（架构 §6.2 `refresh_clock_rects` 的注入式形态）。
///
/// 与 §6.2 的偏离：显式注入 `ClockRectSource` / `Clock`，返回 [`MaskSet`] 而非
/// `Vec<MaskRect>`（固定容量、零堆分配）；UIA 不可用时**仍返回 `Ok(fallback)`**，
/// 因为按判据②功能不得降级——只有连固定几何都算不出时才返回 `Err`。
///
/// # Errors
///
/// `geom.tray` 退化导致 fallback 矩形为空时返回 [`SbError::UiaUnavailable`]。
pub fn refresh_clock_rects(
    source: &mut dyn ClockRectSource,
    clock: &dyn Clock,
    geom: &TaskbarGeom,
) -> SbResult<MaskSet> {
    let mut probe = UiaProbe::new();
    let mut out = MaskSet::new();
    probe.refresh_into(source, clock, geom, &mut out);
    if out.count() == 0 || rect_is_empty(&geom.tray) {
        Err(SbError::UiaUnavailable {
            hr: UIA_UNAVAILABLE_HR,
        })
    } else {
        Ok(out)
    }
}

/// 检查一组 UIA 原始矩形是否合法（全部非空、且落在任务栏矩形内）。
///
/// 真实 UIA 返回的 `BoundingRectangle` 可能因为窗口动画/最小化而跑到屏外，
/// 本函数在合并前过滤，避免把屏外坐标当成遮罩（EX-24）。
#[must_use]
pub fn rects_within(rects: &[MaskRect], bar: &RectPx) -> bool {
    rects
        .iter()
        .all(|m| !rect_is_empty(&m.rect) && rect_clamp(&m.rect, bar).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{FakeClock, FakeTaskbar, FakeUia};
    use crate::engine::taskbar::locate;
    use std::sync::Arc;

    fn shared() -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(0))
    }

    #[test]
    fn connection_config_limits_timeout_to_300ms() {
        let c = UiaConnectionConfig::default();
        assert_eq!(c.connection_timeout_ms, 300, "T02.3 判据③");
        assert_eq!(c.refresh_interval_ms, UIA_REFRESH_INTERVAL_MS);
    }

    #[test]
    fn working_uia_yields_clock_and_date_rects() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::working();
        let mut probe = UiaProbe::new();
        let mut out = MaskSet::new();
        probe.refresh_into(&mut uia, &clock, &geom, &mut out);

        assert_eq!(out.count(), 2, "时钟 + 日期两个独立矩形（判据①）");
        assert!(!probe.in_fallback());
        assert_eq!(uia.calls(), 1);
        for m in out.rects() {
            assert!(
                rect_clamp(&m.rect, &geom.tray).is_some(),
                "UIA 矩形不得越出托盘区"
            );
        }
    }

    #[test]
    fn uia_unavailable_falls_back_without_degradation() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::unavailable();
        let mut probe = UiaProbe::new();
        let mut out = MaskSet::new();
        probe.refresh_into(&mut uia, &clock, &geom, &mut out);

        assert!(probe.in_fallback(), "判据②：应退回固定几何");
        assert_eq!(out.count(), 2, "fallback 仍提供两个矩形，功能不降级");
        assert!(out.rects().iter().any(|m| m.kind == MaskKind::Clock));
        assert!(out.rects().iter().any(|m| m.kind == MaskKind::Date));
    }

    #[test]
    fn uia_timeout_over_300ms_trips_circuit_breaker() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::slow(350, Arc::clone(&s));
        let mut probe = UiaProbe::new();
        let mut out = MaskSet::new();
        probe.refresh_into(&mut uia, &clock, &geom, &mut out);

        assert_eq!(probe.last_elapsed_ms(), 350);
        assert!(probe.circuit_open(), "超时必须立即熔断（判据③）");
        assert!(probe.in_fallback());

        // 熔断后不再调用 UIA → T0/T1 绝不被阻塞。
        let before = uia.calls();
        for _ in 0..10 {
            probe.refresh_into(&mut uia, &clock, &geom, &mut out);
        }
        assert_eq!(uia.calls(), before);
        assert!(out.count() >= 2);
    }

    #[test]
    fn three_consecutive_failures_trip_circuit_and_reset_retries() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::unavailable();
        let mut probe = UiaProbe::new();
        let mut out = MaskSet::new();
        for _ in 0..2 {
            probe.refresh_into(&mut uia, &clock, &geom, &mut out);
        }
        assert!(!probe.circuit_open(), "两次失败不应熔断");
        probe.refresh_into(&mut uia, &clock, &geom, &mut out);
        assert!(probe.circuit_open());
        probe.reset_circuit();
        assert!(!probe.circuit_open());
        probe.refresh_into(&mut uia, &clock, &geom, &mut out);
        assert_eq!(uia.calls(), 4, "reset 后应重新尝试真实 UIA");
    }

    #[test]
    fn empty_uia_result_also_falls_back() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::empty();
        let out = refresh_clock_rects(&mut uia, &clock, &geom).expect("应给出 fallback");
        assert_eq!(out.count(), 2);
    }

    #[test]
    fn fallback_scales_with_dpi_and_stays_inside_tray() {
        // §11.4 单位铁律：130×44 dip 必须经 dip_to_px 换算，且结果只含 px。
        for dpi in [96, 120, 144, 192] {
            let mut src = FakeTaskbar::desktop_1080p();
            src.set_dpi(dpi);
            let geom = locate(&mut src).expect("几何");
            let rects = fallback_clock_rects(&geom);
            let w = (rects[0].rect.right - rects[0].rect.left) as f32;
            let expect = dip_to_px(dpi, FALLBACK_CLOCK_W_DIP);
            assert!((w - expect).abs() <= 4.0, "dpi={dpi} w={w} expect={expect}");
            for m in rects {
                assert!(rect_clamp(&m.rect, &geom.tray).is_some());
            }
        }
    }

    #[test]
    fn fallback_inflates_by_two_pixels_after_union() {
        let s = shared();
        let clock = FakeClock::new(Arc::clone(&s));
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let mut uia = FakeUia::unavailable();
        let mut out = MaskSet::new();
        UiaProbe::new().refresh_into(&mut uia, &clock, &geom, &mut out);
        let raw = fallback_clock_rects(&geom);
        for (got, want) in out.rects().iter().zip(raw.iter()) {
            assert_eq!(
                got.rect.left,
                want.rect.left - 2,
                "膨胀 2 px（判据①）"
            );
        }
    }

    #[test]
    fn rects_within_filters_offscreen_rects() {
        let bar = rect_px(0, 1032, 1920, 1080);
        let good = MaskRect::new(rect_px(1700, 1040, 1800, 1060), MaskKind::Clock);
        assert!(rects_within(&[good], &bar));
        let off = MaskRect::new(rect_px(9000, 9000, 9100, 9100), MaskKind::Clock);
        assert!(!rects_within(&[off], &bar));
        let empty = MaskRect::new(rect_px(10, 10, 10, 20), MaskKind::Date);
        assert!(!rects_within(&[empty], &bar));
    }
}
