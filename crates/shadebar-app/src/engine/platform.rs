//! T04 平台底座：系统时钟、可等待定时器、停止令牌、电源状态、配置源、日志出口。
//!
//! # 设计约束
//!
//! * 全部实现 `shadebar_core::engine` 里定义的对象安全 trait（§11.7 依赖注入）。
//! * 所有 Win32 调用按 §11.9 统一处理 `windows` 0.58 的 `Result<()>` 返回类型，
//!   失败时保留 `GetLastError()` 原值到 `SbError::Win32`（§11.2）。
//! * 不持有任何 `HWND`；不停留任何跨时点的 COM 元素引用（§11.10）。
//! * `Sleeper` 用 `CreateWaitableTimerW` 而不是 `Sleep(1)` 忙等（§3.3）。


// 说明：本模块同时服务「正式 release 的 EngineSession」与「verify 取证档」。
// 关闭 verify 时，仅供取证档使用的访问器/字段会变成 dead_code；它们不是废代码，
// 在 --features verify 构建与 cargo test 中都被真实调用。故只在非 verify 构建放宽该 lint。
#![cfg_attr(not(feature = "verify"), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use shadebar_core::engine::{
    Clock, ConfigSource, DecisionSink, DiagSink, DiagSnapshot, EngineConfig, Judgement,
    PowerSource, Sleeper, StopToken, SuspendReason,
};
use shadebar_core::win::last_error_code;
use shadebar_core::{SbError, SbResult};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Threading::{
    CreateWaitableTimerW, GetCurrentProcess, GetProcessTimes, PTIMERAPCROUTINE,
    SetWaitableTimer, WaitForSingleObject,
};

/// `windows` 0.58 的 `Result<()>` 失败统一转成保留原始错误码的 `SbError::Win32`。
///
/// 必须在失败调用之后**立即**调用，中间不得插入其它 Win32 调用（否则 `GetLastError`
/// 会被覆盖）。
#[must_use]
pub fn win_err(op: &'static str) -> SbError {
    SbError::Win32 {
        op,
        code: last_error_code(),
    }
}

/// 把 `windows_core::Error` 转成 `SbError::HResult`（保留 HRESULT 原值）。
#[must_use]
pub fn hr_err(op: &'static str, e: &windows::core::Error) -> SbError {
    SbError::HResult {
        op,
        hr: e.code().0,
    }
}

/// 基于 `std::time::Instant` 的单调毫秒时钟（不受系统时间调整影响）。
///
/// `Copy` 是刻意设计的：[`ClockBackedStop`] 必须能复制同一时间原点，
/// 否则停止令牌与 `EngineDeps.clock` 会各自从零计时而提前/延后退出。
#[derive(Clone, Copy)]
pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClock {
    /// 建立时钟（`0` 点为本对象创建时刻）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        // `Instant` 是单调的，不受系统时间调整影响；毫秒口径与 core 的所有
        // deadline / interval 常量一致。
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// 基于 `CreateWaitableTimerW` 的等待器（§3.3：禁止忙等）。
///
/// 每次 `sleep_until` 设置一个**相对时间**的可等待定时器并阻塞等待；`deadline_ms`
/// 是相对 [`SystemClock`] 起点的绝对毫秒值，由本结构自己换算。
pub struct WaitableTimerSleeper {
    timer: HANDLE,
    clock_start_ms: u64,
    origin: Instant,
    /// 累计等待次数（诊断用，验证没有忙等）。
    waits: std::sync::atomic::AtomicU64,
}

impl WaitableTimerSleeper {
    /// 创建一个手动重置的可等待定时器。
    ///
    /// `clock_start_ms` 传 [`SystemClock`] 的当前值，用于对齐时间轴。
    ///
    /// # Errors
    ///
    /// `CreateWaitableTimerW` 失败时返回 `SbError::Win32`。
    pub fn new(clock_start_ms: u64) -> SbResult<Self> {
        // SAFETY: 无安全属性、手动重置、无名称；失败由 Result 返回。
        let timer = unsafe { CreateWaitableTimerW(None, true, None) }
            .map_err(|_| win_err("CreateWaitableTimerW"))?;
        Ok(Self {
            timer,
            clock_start_ms,
            origin: Instant::now(),
            waits: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// 累计等待次数。
    #[must_use]
    pub fn waits(&self) -> u64 {
        self.waits.load(Ordering::Relaxed)
    }
}

impl Drop for WaitableTimerSleeper {
    fn drop(&mut self) {
        if !self.timer.is_invalid() {
            // SAFETY: 句柄由 CreateWaitableTimerW 创建且只在此处释放一次。
            unsafe { CloseHandle(self.timer) }.ok();
        }
    }
}

impl Sleeper for WaitableTimerSleeper {
    fn sleep_until(&self, deadline_ms: u64) {
        let now = u64::try_from(self.origin.elapsed().as_millis())
            .unwrap_or(u64::MAX)
            .saturating_add(self.clock_start_ms);
        let remaining = deadline_ms.saturating_sub(now);
        if remaining == 0 {
            return;
        }
        // FILETIME 相对时间用负值，单位 100 ns；10_000 = 1 ms。
        let due: i64 = -i64::try_from(remaining.min(u32::MAX as u64))
            .unwrap_or(i64::MAX)
            .saturating_mul(10_000)
            .max(i64::MIN + 1);
        // SAFETY: timer 有效；due 是栈上 i64 的地址，调用期间存活；
        // 无完成例程（PTIMERAPCROUTINE = None）、不唤醒系统。
        let no_routine: PTIMERAPCROUTINE = None;
        let ok = unsafe {
            SetWaitableTimer(
                self.timer,
                &due as *const i64,
                0,
                no_routine,
                None,
                false,
            )
        };
        if ok.is_err() {
            // 定时器设置失败：退化成一次性 Sleep，仍不忙等。
            // SAFETY: Sleep 无前置条件，参数已上限钳制。
            unsafe { windows::Win32::System::Threading::Sleep(remaining.min(u32::MAX as u64) as u32) };
            return;
        }
        // SAFETY: timer 有效；INFINITE 由上面设置的相对到期时间保证会到达。
        let ret = unsafe { WaitForSingleObject(self.timer, u32::MAX) };
        self.waits.fetch_add(1, Ordering::Relaxed);
        debug_assert_eq!(ret, WAIT_OBJECT_0, "可等待定时器应正常到期");
    }
}

/// 停止令牌：截止时刻 + 可外部置位的原子标志（如 Ctrl-C / 脚本提前结束）。
pub struct DeadlineStopToken {
    deadline_ms: u64,
    forced: Arc<AtomicBool>,
}

impl DeadlineStopToken {
    /// `duration_ms` 为从现在起运行的毫秒数。
    #[must_use]
    pub fn new(duration_ms: u64) -> Self {
        Self {
            deadline_ms: duration_ms,
            forced: Arc::new(AtomicBool::new(false)),
        }
    }

}

impl StopToken for DeadlineStopToken {
    fn is_stopped(&self) -> bool {
        // 由 [`SystemClockProbe`] 注入当前时刻；本结构只持有相对 deadline，
        // 实际比较在 `ClockBackedStop` 里做。此处提供 forced 短路。
        self.forced.load(Ordering::Relaxed)
    }
}

/// 把 `DeadlineStopToken` 与真实时钟绑定后的停止判定。
pub struct ClockBackedStop {
    deadline_ms: u64,
    forced: Arc<AtomicBool>,
    clock: SystemClock,
}

impl ClockBackedStop {
    /// 绑定。
    #[must_use]
    pub fn new(token: &DeadlineStopToken, clock: SystemClock) -> Self {
        Self {
            deadline_ms: token.deadline_ms,
            forced: Arc::clone(&token.forced),
            clock,
        }
    }
}

impl StopToken for ClockBackedStop {
    fn is_stopped(&self) -> bool {
        self.forced.load(Ordering::Relaxed) || self.clock.now_ms() >= self.deadline_ms
    }
}

/// 系统电源状态（`GetSystemPowerStatus`）。
pub struct SystemPowerSource;

impl PowerSource for SystemPowerSource {
    fn on_battery(&self) -> bool {
        let mut st = SYSTEM_POWER_STATUS::default();
        // SAFETY: st 是完整初始化的输出结构，调用期间存活。
        if unsafe { GetSystemPowerStatus(&mut st) }.is_err() {
            // 读不到电源状态按交流供电处理（最保守，不触发节能降频）。
            return false;
        }
        // ACLineStatus: 0 = 离线(电池), 1 = 在线, 255 = 未知。
        st.ACLineStatus == 0
    }
}

/// 固定配置源（T01.2 `ConfigStore` 落地前的接线快照）。
///
/// 字段与 [`EngineConfig::default_v1`] 一致；`version` 恒 0 表示本轮不热更新。
pub struct FixedConfigSource {
    cfg: EngineConfig,
}

impl FixedConfigSource {
    /// 以给定配置建立。
    #[must_use]
    pub const fn new(cfg: EngineConfig) -> Self {
        Self { cfg }
    }

    /// 热更新快照（A-10：`EngineSession::set_config` 在每拍前调用）。
    pub fn update(&mut self, cfg: EngineConfig) {
        self.cfg = cfg;
    }

    /// 当前快照（供单测与诊断读取，不经过 trait 也能核对）。
    #[must_use]
    pub const fn get(&self) -> EngineConfig {
        self.cfg
    }
}

impl ConfigSource for FixedConfigSource {
    fn snapshot(&self) -> EngineConfig {
        self.cfg
    }
}

/// 判定出口：累计发布/翻转计数，并把每帧写进证据日志。
pub struct RecordingDecisionSink {
    pub published: u64,
    pub flips: u64,
    pub last: Option<Judgement>,
    pub log: Vec<String>,
    /// 与 `log` 一一对应的逐拍翻转标志。
    ///
    /// v1.4 §9.1 要求 A-13/A-14 按「稳态段」统计翻转（排除切换背景后的
    /// 段首第一拍），故取证端必须能按拍切片，而不能只用累计 `flips`。
    pub flip_flags: Vec<bool>,
}

impl Default for RecordingDecisionSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingDecisionSink {
    /// 空记录器。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            published: 0,
            flips: 0,
            last: None,
            log: Vec::new(),
            flip_flags: Vec::new(),
        }
    }
}

impl DecisionSink for RecordingDecisionSink {
    fn publish(&mut self, decision: &Judgement) {
        self.published = self.published.saturating_add(1);
        if decision.flipped {
            self.flips = self.flips.saturating_add(1);
        }
        self.last = Some(*decision);
        self.flip_flags.push(decision.flipped);
        self.log.push(format!(
            "judge: seq={} side={:?} y={:.4} contrast={:.3} flipped={} color=#{:02X}{:02X}{:02X}",
            decision.seq,
            decision.side,
            decision.y_smoothed,
            decision.contrast,
            decision.flipped,
            decision.color.r,
            decision.color.g,
            decision.color.b
        ));
    }
}

/// 诊断出口：累计耗时样本（用于 NFR-03 的 avg/p95/max 实测）。
pub struct RecordingDiagSink {
    pub cost_us: Vec<u64>,
    pub samples: u64,
    pub skipped: u64,
    pub valid_px: Vec<u32>,
    pub rep_colors: Vec<(u8, u8, u8)>,
    pub y_milli: Vec<u32>,
    pub notices: Vec<SuspendReason>,
}

impl Default for RecordingDiagSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingDiagSink {
    /// 空记录器。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cost_us: Vec::new(),
            samples: 0,
            skipped: 0,
            valid_px: Vec::new(),
            rep_colors: Vec::new(),
            y_milli: Vec::new(),
            notices: Vec::new(),
        }
    }

    /// 计算耗时分位数（微秒）。返回 `(avg, p95, max)`。
    #[must_use]
    pub fn cost_stats(&self) -> (u64, u64, u64) {
        Self::cost_stats_of(&self.cost_us)
    }

    /// 对任意一段耗时样本计算分位数（微秒）。
    ///
    /// 供取证按「本段」切片统计，避免把别段的异常帧混进 p95；与
    /// [`Self::cost_stats`] 共用同一套口径，杜绝重复实现。
    #[must_use]
    pub fn cost_stats_of(samples: &[u64]) -> (u64, u64, u64) {
        if samples.is_empty() {
            return (0, 0, 0);
        }
        let mut v = samples.to_vec();
        v.sort_unstable();
        let sum: u64 = v.iter().sum();
        let avg = sum / v.len() as u64;
        let idx = (v.len() * 95).div_ceil(100).saturating_sub(1).min(v.len() - 1);
        (avg, v[idx], v[v.len() - 1])
    }
}

impl DiagSink for RecordingDiagSink {
    fn update(&mut self, snap: &DiagSnapshot) {
        self.samples = self.samples.saturating_add(1);
        self.cost_us.push(snap.cost_us);
        self.valid_px.push(snap.valid_px);
        self.rep_colors
            .push((snap.rep_color.r, snap.rep_color.g, snap.rep_color.b));
        self.y_milli.push(snap.y_milli);
        if snap.valid_px == 0 {
            self.skipped = self.skipped.saturating_add(1);
        }
    }

    fn notice(&mut self, reason: SuspendReason) {
        self.notices.push(reason);
    }
}

/// 读取本进程累计 CPU 时间（kernel + user，单位 100 ns）。
///
/// 用于 A-20「10 分钟平均 CPU」的对照测量：ΔCPU / Δ墙钟 = 单核占用率。
#[must_use]
pub fn process_cpu_100ns() -> u64 {
    let mut creation = FILETIME::default();
    let mut exit_time = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 四个 FILETIME 均为完整初始化的输出缓冲；伪句柄无需关闭。
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit_time,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return 0;
    }
    filetime_to_u64(kernel).saturating_add(filetime_to_u64(user))
}

#[must_use]
const fn filetime_to_u64(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | (t.dwLowDateTime as u64)
}

/// 逻辑处理器数量（用于把「单核占用」换算成任务管理器口径）。
#[must_use]
pub fn logical_cpu_count() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic() {
        let c = SystemClock::new();
        let a = c.now_ms();
        std::thread::sleep(std::time::Duration::from_millis(12));
        let b = c.now_ms();
        assert!(b >= a, "时钟不得倒退：{a} -> {b}");
    }

    #[test]
    fn filetime_conversion_matches_high_low() {
        let t = FILETIME {
            dwLowDateTime: 0x1234_5678,
            dwHighDateTime: 0x0000_00AB,
        };
        assert_eq!(filetime_to_u64(t), 0x0000_00AB_1234_5678);
    }

    #[test]
    fn cost_stats_empty_is_zero() {
        let d = RecordingDiagSink::new();
        assert_eq!(d.cost_stats(), (0, 0, 0));
    }

    #[test]
    fn cost_stats_avg_p95_max() {
        let mut d = RecordingDiagSink::new();
        for i in 1..=100u64 {
            d.cost_us.push(i * 10);
        }
        let (avg, p95, max) = d.cost_stats();
        assert_eq!(avg, 505);
        assert_eq!(p95, 950);
        assert_eq!(max, 1000);
    }

    #[test]
    fn decision_sink_counts_flips() {
        use shadebar_core::engine::Side;
        use shadebar_core::Rgba;
        let mut s = RecordingDecisionSink::new();
        let base = Judgement {
            side: Side::Light,
            color: Rgba::opaque(255, 255, 255),
            y_smoothed: 0.02,
            contrast: 21.0,
            flipped: false,
            seq: 1,
        };
        s.publish(&base);
        assert_eq!(s.published, 1);
        assert_eq!(s.flips, 0);
        let mut flip = base;
        flip.flipped = true;
        flip.seq = 2;
        s.publish(&flip);
        assert_eq!(s.published, 2);
        assert_eq!(s.flips, 1);
        assert_eq!(s.log.len(), 2);
        assert_eq!(s.flip_flags, vec![false, true]);
    }

    #[test]
    fn steady_flips_excludes_segment_head() {
        // v1.4 §9.1：段首第一拍（切背景那一拍）不计入稳态翻转。
        let mut s = RecordingDecisionSink::new();
        use shadebar_core::engine::Side;
        use shadebar_core::Rgba;
        let base = Judgement {
            side: Side::Light,
            color: Rgba::opaque(255, 255, 255),
            y_smoothed: 0.02,
            contrast: 21.0,
            flipped: false,
            seq: 1,
        };
        let mut head = base;
        head.flipped = true;
        head.seq = 2;
        s.publish(&head);
        s.publish(&base);
        let steady: usize = s.flip_flags.iter().skip(1).filter(|&&f| f).count();
        assert_eq!(s.flips, 1, "全段原始翻转如实保留");
        assert_eq!(steady, 0, "稳态段排除段首后翻转应为 0");
    }

    #[test]
    fn fixed_config_source_returns_same_snapshot() {
        let cfg = EngineConfig::default_v1();
        let src = FixedConfigSource::new(cfg);
        assert_eq!(src.snapshot(), cfg);
    }

    #[test]
    fn process_cpu_is_readable() {
        // 只验证 API 不崩且能读到值（可能为 0，取决于权限，不做数值断言）。
        let _ = process_cpu_100ns();
        assert!(logical_cpu_count() >= 1);
    }
}
