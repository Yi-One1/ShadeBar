//! 引擎单测替身（`#[doc(hidden)]`，只在 `cfg(test)` 编译）。
//!
//! 集中提供全套**确定性假实现**，让六个引擎模块都能脱离真实屏幕 / explorer /
//! 消息循环单测（§11.7）。这些不是占位符：它们能完整驱动 600 s 闭环，
//! 包括时间推进、UIA 超时、BitBlt 失败、DPI 切换、托盘折叠等边界。
//!
//! 真实 Win32 / COM 实现由 T04 在主进程注入；本文件不触碰任何平台 API。

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::color::Rgba;
use crate::engine::{
    EngineConfig, MaskKind, MaskRect, MaskSet, RectPx, SuspendReason, rect_px,
};
use crate::engine::mask::MaskKind as MaskKindAlias;
use crate::engine::sampler::{
    Clock, ConfigSource, DecisionSink, DiagSink, DiagSnapshot, Judgement, PowerSource, Sleeper,
    StopToken,
};
use crate::engine::screen::{ScreenBuffer, ScreenInfo, ScreenSource, checked_frame_size};
use crate::engine::taskbar::{TaskbarProbe, TaskbarSource};
use crate::engine::uia::{ClockRectSource, UIA_UNAVAILABLE_HR};
use crate::error::{SbError, SbResult};

// ---------------------------------------------------------------------------
// GlobalAlloc 计数（定义在 lib.rs 的 cfg(test) 模块里，这里只做转发）
// ---------------------------------------------------------------------------

/// 当前线程自上次 reset 以来的堆分配次数（T02.2 判据①的证据来源）。
#[must_use]
pub fn alloc_count() -> u64 {
    crate::test_alloc::count()
}

/// 归零当前线程的分配计数。
pub fn reset_alloc_count() {
    crate::test_alloc::reset();
}

// ---------------------------------------------------------------------------
// 时间
// ---------------------------------------------------------------------------

/// 由共享 `AtomicU64` 驱动的假单调时钟。
pub struct FakeClock {
    shared: Arc<AtomicU64>,
}

impl FakeClock {
    /// 绑定共享时钟。
    #[must_use]
    pub const fn new(shared: Arc<AtomicU64>) -> Self {
        Self { shared }
    }

    /// 手动推进 `ms` 毫秒。
    pub fn advance(&self, ms: u64) {
        self.shared.fetch_add(ms, Ordering::Relaxed);
    }

    /// 直接设定时刻。
    pub fn set(&self, ms: u64) {
        self.shared.store(ms, Ordering::Relaxed);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.shared.load(Ordering::Relaxed)
    }
}

/// 假等待器：不真睡，只把共享时钟推到 deadline。
pub struct TestSleeper {
    shared: Arc<AtomicU64>,
}

impl TestSleeper {
    /// 绑定共享时钟。
    #[must_use]
    pub const fn new(shared: Arc<AtomicU64>) -> Self {
        Self { shared }
    }
}

impl Sleeper for TestSleeper {
    fn sleep_until(&self, deadline_ms: u64) {
        self.shared.fetch_max(deadline_ms, Ordering::Relaxed);
    }
}

/// 在指定时刻停止的假令牌。
pub struct TestStop {
    deadline_ms: u64,
    shared: Arc<AtomicU64>,
}

impl TestStop {
    /// `ms` 毫秒后停止。
    #[must_use]
    pub const fn after_ms(ms: u64, shared: Arc<AtomicU64>) -> Self {
        Self {
            deadline_ms: ms,
            shared,
        }
    }
}

impl StopToken for TestStop {
    fn is_stopped(&self) -> bool {
        self.shared.load(Ordering::Relaxed) >= self.deadline_ms
    }
}

// ---------------------------------------------------------------------------
// 电源 / 配置 / 出口
// ---------------------------------------------------------------------------

/// 假电源状态。
pub struct FakePower {
    battery: Cell<bool>,
}

impl FakePower {
    /// 交流供电。
    #[must_use]
    pub const fn ac() -> Self {
        Self {
            battery: Cell::new(false),
        }
    }

    /// 电池供电。
    #[must_use]
    pub const fn battery() -> Self {
        Self {
            battery: Cell::new(true),
        }
    }

    /// 切换状态。
    pub fn set_battery(&self, on: bool) {
        self.battery.set(on);
    }
}

impl PowerSource for FakePower {
    fn on_battery(&self) -> bool {
        self.battery.get()
    }
}

/// 假配置源（可热改，`version` 自动递增）。
pub struct FakeConfig {
    cfg: Cell<EngineConfig>,
}

impl FakeConfig {
    /// 以配置快照建立。
    #[must_use]
    pub const fn new(cfg: EngineConfig) -> Self {
        Self { cfg: Cell::new(cfg) }
    }

    /// 改采样间隔（自动递增版本号，模拟 A-10 配置变更）。
    pub fn set_interval(&self, interval_ms: u32) {
        let mut c = self.cfg.get();
        c.interval_ms = interval_ms;
        c.version = c.version.saturating_add(1);
        self.cfg.set(c);
    }

    /// 整体替换配置。
    pub fn set(&self, mut cfg: EngineConfig) {
        cfg.version = self.cfg.get().version.saturating_add(1);
        self.cfg.set(cfg);
    }
}

impl ConfigSource for FakeConfig {
    fn snapshot(&self) -> EngineConfig {
        self.cfg.get()
    }
}

/// 记录判定发布的假出口。
pub struct FakeDecision {
    last: Option<Judgement>,
    count: u64,
}

impl Default for FakeDecision {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeDecision {
    /// 空记录。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last: None,
            count: 0,
        }
    }

    /// 发布次数。
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// 最后一次判定。
    #[must_use]
    pub const fn last(&self) -> Option<Judgement> {
        self.last
    }
}

impl DecisionSink for FakeDecision {
    fn publish(&mut self, decision: &Judgement) {
        self.last = Some(*decision);
        self.count = self.count.saturating_add(1);
    }
}

/// 记录诊断与提示的假出口。
pub struct FakeDiag {
    notices: u64,
    updates: u64,
    last: Option<DiagSnapshot>,
}

impl Default for FakeDiag {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeDiag {
    /// 空记录。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            notices: 0,
            updates: 0,
            last: None,
        }
    }

    /// 用户提示次数（EX-07 要求恰好 1 次）。
    #[must_use]
    pub const fn notices(&self) -> u64 {
        self.notices
    }

    /// 诊断更新次数。
    #[must_use]
    pub const fn updates(&self) -> u64 {
        self.updates
    }

    /// 最后一次诊断快照。
    #[must_use]
    pub const fn last(&self) -> Option<DiagSnapshot> {
        self.last
    }
}

impl DiagSink for FakeDiag {
    fn update(&mut self, snap: &DiagSnapshot) {
        self.updates = self.updates.saturating_add(1);
        self.last = Some(*snap);
    }

    fn notice(&mut self, _reason: SuspendReason) {
        self.notices = self.notices.saturating_add(1);
    }
}

// ---------------------------------------------------------------------------
// 任务栏几何假源
// ---------------------------------------------------------------------------

/// 假任务栏读数源，可模拟自动隐藏 / 竖向 / 多显示器 / DPI / 托盘折叠 / 窗口不存在。
pub struct FakeTaskbar {
    probe: TaskbarProbe,
    missing: bool,
}

impl FakeTaskbar {
    /// 1920×1080、96 DPI、底部 48 px 任务栏、托盘区 300×48。
    #[must_use]
    pub fn desktop_1080p() -> Self {
        let mut monitors = std::array::from_fn(|_| ScreenInfo::default());
        monitors[0] = ScreenInfo {
            bounds: rect_px(0, 0, 1920, 1080),
            work: rect_px(0, 0, 1920, 1032),
            dpi: 96,
            monitor: 1,
            primary: true,
        };
        Self {
            probe: TaskbarProbe {
                bar: rect_px(0, 1032, 1920, 1080),
                tray: rect_px(1620, 1032, 1920, 1080),
                auto_hide: false,
                dpi: 96,
                monitors,
                monitors_len: 1,
                tray_collapsed: false,
            },
            missing: false,
        }
    }

    /// 竖向任务栏（左边缘 60 px 宽）。
    #[must_use]
    pub fn vertical() -> Self {
        let mut s = Self::desktop_1080p();
        s.probe.bar = rect_px(0, 0, 60, 1080);
        s.probe.tray = rect_px(0, 900, 60, 1080);
        s.probe.monitors[0].work = rect_px(60, 0, 1920, 1080);
        s
    }

    /// explorer 重启期间窗口暂时不存在。
    #[must_use]
    pub fn missing() -> Self {
        let mut s = Self::desktop_1080p();
        s.missing = true;
        s
    }

    /// 三屏横排：左副屏 / 中间主屏（含任务栏）/ 右副屏。
    #[must_use]
    pub fn three_monitors_taskbar_on_primary() -> Self {
        let mut monitors = std::array::from_fn(|_| ScreenInfo::default());
        monitors[0] = ScreenInfo {
            bounds: rect_px(-1920, 0, 0, 1080),
            work: rect_px(-1920, 0, 0, 1032),
            dpi: 96,
            monitor: 1,
            primary: false,
        };
        monitors[1] = ScreenInfo {
            bounds: rect_px(1920, 0, 3840, 1080),
            work: rect_px(1920, 0, 3840, 1032),
            dpi: 96,
            monitor: 2,
            primary: true,
        };
        monitors[2] = ScreenInfo {
            bounds: rect_px(3840, 0, 5760, 1080),
            work: rect_px(3840, 0, 5760, 1032),
            dpi: 96,
            monitor: 3,
            primary: false,
        };
        Self {
            probe: TaskbarProbe {
                bar: rect_px(1920, 1032, 3840, 1080),
                tray: rect_px(3540, 1032, 3840, 1080),
                auto_hide: false,
                dpi: 96,
                monitors,
                monitors_len: 3,
                tray_collapsed: false,
            },
            missing: false,
        }
    }

    /// 把任务栏搬到左侧副屏（EX-08 的反向用例）。
    pub fn move_taskbar_to_left_screen(&mut self) {
        self.probe.bar = rect_px(-1920, 1032, 0, 1080);
        self.probe.tray = rect_px(-300, 1032, 0, 1080);
    }

    /// 当前原始读数（供 `needs_refresh_from` 断言）。
    #[must_use]
    pub const fn probe(&self) -> TaskbarProbe {
        self.probe
    }

    /// 切换自动隐藏。
    pub fn set_auto_hide(&mut self, on: bool) {
        self.probe.auto_hide = on;
    }

    /// 切换 DPI（同时按物理像素缩放任务栏、托盘与屏幕）。
    pub fn set_dpi(&mut self, dpi: u32) {
        let s = f64::from(dpi) / 96.0;
        let sw = (1920.0 * s) as i32;
        let sh = (1080.0 * s) as i32;
        let th = (48.0 * s) as i32;
        let tw = (300.0 * s) as i32;
        self.probe.dpi = dpi;
        self.probe.bar = rect_px(0, sh - th, sw, sh);
        self.probe.tray = rect_px(sw - tw, sh - th, sw, sh);
        self.probe.monitors[0].bounds = rect_px(0, 0, sw, sh);
        self.probe.monitors[0].work = rect_px(0, 0, sw, sh - th);
        self.probe.monitors[0].dpi = dpi;
    }

    /// 改托盘区尺寸（右对齐、底对齐），模拟托盘折叠 / 展开。
    pub fn set_tray_size(&mut self, w: i32, h: i32) {
        self.probe.tray = rect_px(
            self.probe.bar.right - w,
            self.probe.bar.bottom - h,
            self.probe.bar.right,
            self.probe.bar.bottom,
        );
    }

    /// 托盘折叠 / 展开（折叠后宽度 120 px）。
    pub fn set_tray_collapsed(&mut self, collapsed: bool) {
        self.probe.tray_collapsed = collapsed;
        if collapsed {
            self.set_tray_size(120, 48);
        } else {
            self.set_tray_size(300, 48);
        }
    }

    /// 把托盘区挪到屏幕外（EX-24 裁剪用例）。
    pub fn set_offscreen_tray(&mut self) {
        self.probe.tray = rect_px(5000, 5000, 5300, 5048);
    }

    /// 热插一块副屏（EX-22）。
    pub fn add_secondary_screen(&mut self) {
        self.probe.monitors[1] = ScreenInfo {
            bounds: rect_px(1920, 0, 3840, 1080),
            work: rect_px(1920, 0, 3840, 1032),
            dpi: 96,
            monitor: 2,
            primary: false,
        };
        self.probe.monitors_len = 2;
    }
}

impl TaskbarSource for FakeTaskbar {
    fn read(&mut self) -> SbResult<TaskbarProbe> {
        if self.missing {
            Err(SbError::TaskbarNotFound)
        } else {
            Ok(self.probe)
        }
    }
}

// ---------------------------------------------------------------------------
// 抓屏假源
// ---------------------------------------------------------------------------

/// 假抓屏模式。
enum FakeScreenMode {
    /// 纯色。
    Uniform(u8, u8, u8),
    /// 黑底 + 指定百分比白噪声。
    WhiteNoise(u32),
    /// `BitBlt` 失败。
    Fail(u32),
}

/// 假抓屏源：写复用缓冲，不分配、不碰真实屏幕。
pub struct FakeScreen {
    mode: FakeScreenMode,
    calls: u64,
}

impl FakeScreen {
    /// 纯色帧。
    #[must_use]
    pub const fn uniform(r: u8, g: u8, b: u8) -> Self {
        Self {
            mode: FakeScreenMode::Uniform(r, g, b),
            calls: 0,
        }
    }

    /// 20% 白像素污染帧（A-28）。
    #[must_use]
    pub const fn white_noise_20() -> Self {
        Self {
            mode: FakeScreenMode::WhiteNoise(20),
            calls: 0,
        }
    }

    /// 指定比例的污染帧。
    #[must_use]
    pub const fn white_noise(pct: u32) -> Self {
        Self {
            mode: FakeScreenMode::WhiteNoise(pct),
            calls: 0,
        }
    }

    /// 每次 `grab` 都失败（EX-24）。
    #[must_use]
    pub const fn failing(code: u32) -> Self {
        Self {
            mode: FakeScreenMode::Fail(code),
            calls: 0,
        }
    }

    /// 累计 grab 调用次数。
    #[must_use]
    pub const fn calls(&self) -> u64 {
        self.calls
    }
}

impl ScreenSource for FakeScreen {
    fn grab(&mut self, rect: &RectPx, buffer: &mut ScreenBuffer) -> SbResult<()> {
        self.calls = self.calls.saturating_add(1);
        if let FakeScreenMode::Fail(code) = self.mode {
            return Err(SbError::BitBltFailed { code });
        }
        let (w, h) = match checked_frame_size(rect) {
            Some(s) => s,
            None => return Err(SbError::BitBltFailed { code: 87 }),
        };
        buffer.ensure(w, h)?;
        // `Fail` 已在函数开头提前返回；这里用 `if let` 只处理成功模式，
        // 无需穷尽匹配的兜底分支，也就不需要任何"不可能到达"的占位。
        match self.mode {
            FakeScreenMode::Uniform(r, g, b) => buffer.fill_uniform(r, g, b),
            FakeScreenMode::WhiteNoise(pct) => buffer.fill_black_with_white_noise(pct),
            FakeScreenMode::Fail(_) => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// UIA 假源
// ---------------------------------------------------------------------------

/// 假 UIA 模式。
enum FakeUiaMode {
    /// 正常量出时钟 + 日期两个矩形。
    Working,
    /// UIA 不可用。
    Unavailable,
    /// 量不出任何矩形。
    Empty,
    /// 超时：推进共享时钟 `ms` 毫秒后再返回结果。
    Slow {
        /// 模拟耗时（毫秒）。
        ms: u64,
        /// 与 `FakeClock` 共享的时钟。
        shared: Arc<AtomicU64>,
    },
}

/// 假 UIA 矩形源。
pub struct FakeUia {
    mode: FakeUiaMode,
    calls: u64,
}

impl FakeUia {
    /// 正常 UIA（量出两个互不相邻的矩形）。
    #[must_use]
    pub const fn working() -> Self {
        Self {
            mode: FakeUiaMode::Working,
            calls: 0,
        }
    }

    /// UIA 被禁用 / `CoCreateInstance` 失败。
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            mode: FakeUiaMode::Unavailable,
            calls: 0,
        }
    }

    /// UIA 可用但量不出任何矩形。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            mode: FakeUiaMode::Empty,
            calls: 0,
        }
    }

    /// 模拟耗时 `ms` 的 UIA 查询（推进共享时钟，用于超时熔断测试）。
    #[must_use]
    pub const fn slow(ms: u64, shared: Arc<AtomicU64>) -> Self {
        Self {
            mode: FakeUiaMode::Slow { ms, shared },
            calls: 0,
        }
    }

    /// 累计调用次数（熔断后应停止增长）。
    #[must_use]
    pub const fn calls(&self) -> u64 {
        self.calls
    }
}

impl ClockRectSource for FakeUia {
    fn refresh_into(&mut self, out: &mut MaskSet) -> SbResult<()> {
        self.calls = self.calls.saturating_add(1);
        match &self.mode {
            FakeUiaMode::Unavailable => Err(SbError::UiaUnavailable {
                hr: UIA_UNAVAILABLE_HR,
            }),
            FakeUiaMode::Empty => Ok(()),
            FakeUiaMode::Slow { ms, shared } => {
                shared.fetch_add(*ms, Ordering::Relaxed);
                write_clock_date_rects(out);
                Ok(())
            }
            FakeUiaMode::Working => {
                write_clock_date_rects(out);
                Ok(())
            }
        }
    }
}

/// 写两个不相邻的文字矩形（中间的空隙避免 2 px 膨胀后被合并）。
fn write_clock_date_rects(out: &mut MaskSet) {
    // 对应 desktop_1080p 的托盘区右侧。
    let clock = MaskRect::new(rect_px(1790, 1034, 1910, 1052), MaskKindAlias::Clock);
    let date = MaskRect::new(rect_px(1790, 1062, 1910, 1078), MaskKindAlias::Date);
    out.union_with(&[clock, date]);
}

/// 供其它模块单测直接构造一个不透明色（避免重复硬编码 Rgba）。
#[must_use]
pub const fn opaque(r: u8, g: u8, b: u8) -> Rgba {
    Rgba::opaque(r, g, b)
}

/// 供其它模块单测构造遮罩矩形（统一来源，避免重复导入）。
#[must_use]
pub const fn mask_rect(r: RectPx, kind: MaskKind) -> MaskRect {
    MaskRect::new(r, kind)
}
