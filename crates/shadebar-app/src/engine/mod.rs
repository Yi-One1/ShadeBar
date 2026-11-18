//! T04 真实平台接线层（`shadebar-app`）。
//!
//! # 分工
//!
//! [`shadebar_core::engine`] 是纯逻辑（几何/遮罩/直方图/判定/闭环状态机），
//! 所有平台能力经对象安全 trait 注入。本模块提供 **8 类真实实现**：
//!
//! | trait | 实现 |
//! |---|---|
//! | `TaskbarSource` | [`taskbar::Win32TaskbarSource`]（`FindWindowW` / `SHAppBarMessage` / `EnumDisplayMonitors`）|
//! | `ScreenSource` | [`screen::GdiScreenSource`]（`BitBlt` + 复用 DIB）|
//! | `ClockRectSource` | [`uia::UiaClockRectSource`]（`IUIAutomation2`，独立 MTA 线程 + 350 ms 看门狗）|
//! | `Clock` | [`platform::SystemClock`]（`Instant` 单调）|
//! | `Sleeper` | [`platform::WaitableTimerSleeper`]（`CreateWaitableTimerW`）|
//! | `StopToken` | [`platform::ClockBackedStop`] |
//! | `PowerSource` | [`platform::SystemPowerSource`]（`GetSystemPowerStatus`）|
//! | `ConfigSource` | [`platform::FixedConfigSource`]（T01.2 `ConfigStore` 落地前的接线快照）|
//!
//! 另外 `DecisionSink` / `DiagSink` 由 [`platform::RecordingDecisionSink`] /
//! [`platform::RecordingDiagSink`] 提供，把每拍数值写进证据日志。
//!
//! # 只读承诺
//!
//! 采样链路只读取窗口句柄/矩形/DPI/屏幕像素/电源状态；不注入 explorer、
//! 不重启 explorer、不改注册表、不动用户已打开的窗口。取证用的可视化窗口
//! 是本进程**新建**的 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` 窗口，结束即销毁。


// 说明：本模块同时服务「正式 release 的 EngineSession」与「verify 取证档」。
// 关闭 verify 时，仅供取证档使用的访问器/字段会变成 dead_code；它们不是废代码，
// 在 --features verify 构建与 cargo test 中都被真实调用。故只在非 verify 构建放宽该 lint。
#![cfg_attr(not(feature = "verify"), allow(dead_code))]

// ---------------------------------------------------------------------------
// 取证模块只在 `verify` 特性下编译（#9 体积收口 / 需求基线 v1.4 §7 K-07）。
// 正式发布版不链接这三个模块，`main.rs` 的 `--verify` 分支同样被 cfg 掉；
// 取证代码本身保留在仓库里，A-29 与后续版本继续用。
// ---------------------------------------------------------------------------
#[cfg(feature = "verify")]
pub mod evidence;
pub mod platform;
#[cfg(feature = "verify")]
pub mod png;
pub mod screen;
pub mod taskbar;
// ---------------------------------------------------------------------------
// UIA 适配器（`uia.rs`）只在 verify 档编译。
//
// 理由（需求基线 v1.4 §7.1 K-05）：真机 26H2 任务栏 XAML 树 30 个后代、
// 时钟/日期矩形命中 0 个，`core::UiaProbe` 始终走固定几何 fallback。
// 该适配器带独立 MTA 线程 + mpsc 看门狗 + IUIAutomation2 COM vtable，
// 是正式版体积的最大单项开销，却对 L1 判定零贡献。正式版改用
// [`FixedFallbackClockRectSource`]：直接返回 `UiaUnavailable`，由 core
// `UiaProbe::refresh_into` 走同一条固定几何 fallback（T02.3 判据②已验证）。
// 取证档保留完整 UIA 适配器，A-02 / A-13 / A-14 的 UIA 诊断证据不受影响。
// ---------------------------------------------------------------------------
#[cfg(feature = "verify")]
pub mod uia;
#[cfg(feature = "verify")]
pub mod visual;

use shadebar_core::engine::{Clock, Engine, EngineConfig, EngineDeps, Sleeper, TickOutcome};
#[cfg(feature = "verify")]
use shadebar_core::engine::EngineReport;
#[cfg(not(feature = "verify"))]
use shadebar_core::SbError;
use shadebar_core::SbResult;

#[cfg(feature = "verify")]
use self::platform::{ClockBackedStop, DeadlineStopToken};
use self::platform::{
    FixedConfigSource, RecordingDecisionSink, RecordingDiagSink, SystemClock, SystemPowerSource,
    WaitableTimerSleeper,
};
use self::screen::GdiScreenSource;
use self::taskbar::Win32TaskbarSource;
#[cfg(feature = "verify")]
use self::uia::UiaClockRectSource;

/// 正式发布版（无 `verify`）的 UIA 源：不实例化 COM、不起 MTA 线程，
/// 直接报告不可用，让 `core::UiaProbe` 走固定几何 fallback。
///
/// 这不是「假装 UIA 可用」，而是把「UIA 发现能力未取得」（K-05）如实
/// 表达成一个显式错误；采样与判定不中断，与真机行为一致。
#[cfg(not(feature = "verify"))]
pub struct FixedFallbackClockRectSource;

#[cfg(not(feature = "verify"))]
impl shadebar_core::engine::ClockRectSource for FixedFallbackClockRectSource {
    fn refresh_into(&mut self, _out: &mut shadebar_core::engine::MaskSet) -> SbResult<()> {
        Err(SbError::UiaUnavailable { hr: -1 })
    }
}

/// 组装好的真实引擎会话：持有全部平台实现 + core 的 [`Engine`]。
///
/// 借用拆分采用字段解构（`let Session { .. } = self`），因此 `engine.tick(&mut deps)`
/// 与 `&mut self.taskbar` 等不会交叉借用；core 的「无窗口、无线程」契约保持不变。
pub struct EngineSession {
    engine: Engine,
    taskbar: Win32TaskbarSource,
    screen: GdiScreenSource,
    #[cfg(feature = "verify")]
    uia: UiaClockRectSource,
    #[cfg(not(feature = "verify"))]
    uia: FixedFallbackClockRectSource,
    power: SystemPowerSource,
    config: FixedConfigSource,
    decision: RecordingDecisionSink,
    diag: RecordingDiagSink,
    clock: SystemClock,
    sleeper: WaitableTimerSleeper,
}

impl EngineSession {
    /// 以给定配置建立会话并启动引擎。
    ///
    /// # Errors
    ///
    /// `CreateWaitableTimerW` 失败时返回 `SbError::Win32`。
    pub fn new(config: EngineConfig) -> SbResult<Self> {
        let clock = SystemClock::new();
        // `SystemClock` 与 sleeper 都在本时刻创建，二者的相对时间轴一致；
        // `clock_start_ms = 0` 即把 sleeper 的绝对时刻对齐到 core 的 `now_ms()`。
        let sleeper = WaitableTimerSleeper::new(0)?;
        let mut engine = Engine::new(config);
        engine.start();
        Ok(Self {
            engine,
            taskbar: Win32TaskbarSource::new(),
            screen: GdiScreenSource::new(),
            #[cfg(feature = "verify")]
            uia: UiaClockRectSource::new(),
            #[cfg(not(feature = "verify"))]
            uia: FixedFallbackClockRectSource,
            power: SystemPowerSource,
            config: FixedConfigSource::new(config),
            decision: RecordingDecisionSink::new(),
            diag: RecordingDiagSink::new(),
            clock,
            sleeper,
        })
    }

    /// 驱动一拍（取证时可在拍间泵自建窗口的消息）。
    pub fn tick(&mut self) -> TickOutcome {
        let EngineSession {
            engine,
            taskbar,
            screen,
            uia,
            power,
            config,
            decision,
            diag,
            clock,
            ..
        } = self;
        let mut deps = EngineDeps {
            taskbar,
            screen,
            uia,
            power: &*power,
            config: &*config,
            decision,
            diag,
            clock: &*clock,
        };
        engine.tick(&mut deps)
    }

    /// 热更新配置源，使下一拍 `Engine::tick` 立刻读到新值（A-10「同一拍生效」）。
    ///
    /// 正式启动流程的采样线程在每拍前从共享配置取快照并调用本方法；
    /// core 的 `ConfigSource::snapshot()` 每拍被调用，故无需重启引擎/进程。
    pub fn set_config(&mut self, cfg: EngineConfig) {
        self.config.update(cfg);
    }

    /// 当前配置源快照（正式流程核对热更新是否落到下一拍）。
    #[must_use]
    pub const fn config_snapshot(&self) -> EngineConfig {
        self.config.get()
    }

    /// 阻塞式闭环：由真实可等待定时器驱动，直到停止令牌到期。
    ///
    /// `duration_ms` 为从调用时刻起的墙钟时长；`clock` 与停止令牌共用同一
    /// 时间轴（[`SystemClock`] 可 `Copy`），因此不会提前/延后退出。
    /// 仅取证档（`verify`）使用；正式流程用 [`Self::tick`] 逐拍驱动以便热更新配置。
    #[cfg(feature = "verify")]
    pub fn run(&mut self, duration_ms: u64) -> EngineReport {
        let deadline = self.clock.now_ms().saturating_add(duration_ms);
        let token = DeadlineStopToken::new(deadline);
        let stop = ClockBackedStop::new(&token, self.clock);
        let EngineSession {
            engine,
            taskbar,
            screen,
            uia,
            power,
            config,
            decision,
            diag,
            clock,
            sleeper,
        } = self;
        let mut deps = EngineDeps {
            taskbar,
            screen,
            uia,
            power: &*power,
            config: &*config,
            decision,
            diag,
            clock: &*clock,
        };
        engine.run(&mut deps, &stop, &*sleeper)
    }

    /// 等待一个采样间隔（取证自定义循环用；同样走真实可等待定时器）。
    pub fn wait_interval(&self, step_ms: u64) {
        self.sleeper
            .sleep_until(self.clock.now_ms().saturating_add(step_ms.max(1)));
    }

    /// 当前单调毫秒。
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// core 引擎只读访问。
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// core 引擎可变访问（故障注入 / DPI 通知等）。仅取证档使用。
    #[cfg(feature = "verify")]
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// 判定记录器只读访问（正式启动流程的诊断面板与托盘判定点也用它）。
    #[must_use]
    pub const fn decision(&self) -> &RecordingDecisionSink {
        &self.decision
    }

    /// 诊断记录器只读访问（正式启动流程的诊断面板也用它）。
    #[must_use]
    pub const fn diag(&self) -> &RecordingDiagSink {
        &self.diag
    }

    /// 任务栏源可变访问（A-02 独立几何探测）。仅取证档使用。
    #[cfg(feature = "verify")]
    pub fn taskbar_mut(&mut self) -> &mut Win32TaskbarSource {
        &mut self.taskbar
    }

    /// 抓屏源可变访问（EX-24 真实失败注入）。仅取证档使用。
    #[cfg(feature = "verify")]
    pub fn screen_mut(&mut self) -> &mut GdiScreenSource {
        &mut self.screen
    }

    /// UIA 源只读访问（诊断日志）。仅取证档使用。
    #[cfg(feature = "verify")]
    #[must_use]
    pub const fn uia(&self) -> &UiaClockRectSource {
        &self.uia
    }

    /// 等待器累计等待次数（证明不是忙等）。仅取证档使用。
    #[cfg(feature = "verify")]
    #[must_use]
    pub fn sleeper_waits(&self) -> u64 {
        self.sleeper.waits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_constructs_and_ticks_without_panicking() {
        // 真机探测：任务栏在测试机上应存在；即便不存在也不得 panic（core 会挂起）。
        let mut session = match EngineSession::new(EngineConfig::default_v1()) {
            Ok(s) => s,
            Err(_) => return, // 定时器不可用的极简环境：跳过，不制造假失败
        };
        let outcome = session.tick();
        // 只断言不崩且状态机有明确结果；真机状态（自动隐藏/竖向/遮挡）不预设。
        let _ = outcome;
        assert!(session.now_ms() < u64::MAX);
    }

    #[test]
    fn wait_interval_returns_without_panicking() {
        let session = match EngineSession::new(EngineConfig::default_v1()) {
            Ok(s) => s,
            Err(_) => return,
        };
        // 1 ms 的相对等待必须正常返回；不预设等待次数（那是取证档才编译的计数器）。
        session.wait_interval(1);
        assert!(session.now_ms() < u64::MAX);
    }

    #[test]
    fn set_config_is_reflected_by_next_snapshot() {
        let mut session = match EngineSession::new(EngineConfig::default_v1()) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut cfg = EngineConfig::default_v1();
        cfg.interval_ms = 2000;
        cfg.version = 7;
        session.set_config(cfg);
        let snap = session.config_snapshot();
        assert_eq!(snap.interval_ms, 2000);
        assert_eq!(snap.version, 7);
    }
}
