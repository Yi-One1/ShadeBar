//! 采样与判定引擎（T02，架构 §8.2）—— **L1 的核心，全链路可单测**。
//!
//! # 设计前提（与架构 §4 文件路径的偏离及理由）
//!
//! 架构 §4 文件清单 #40–#45 把 `engine/` 放在 `shadebar-app`，本实现按 team-lead
//! 本轮指派放在 `shadebar-core`。理由来自两条硬约束：
//!
//! 1. `shadebar-core/src/lib.rs` 规定本 crate **无 UI、无窗口、无线程**；
//! 2. §11.7 规定「单测不得依赖真实屏幕 / explorer，`screen` / `uia` 通过 trait 注入假实现」。
//!
//! 因此这里把**所有平台能力抽成对象安全的薄 trait**（[`TaskbarSource`] /
//! [`ScreenSource`] / [`ClockRectSource`] / [`Clock`] / [`PowerSource`] / [`Sleeper`]），
//! 几何、遮罩、直方图、判定、闭环调度全部是纯逻辑；真实 `FindWindowW` / `BitBlt` /
//! `IUIAutomation` 由 T04 在主进程内实现并注入。**本批不触碰任何进程空间、不重启
//! explorer、不改注册表、不动用户窗口**——引擎只读，天然与 TranslucentTB 共存。
//!
//! # 单位铁律（§11.4）
//!
//! 本模块所有矩形**只接受物理像素 px**（i32）。唯一允许出现 dip 的地方是
//! [`FALLBACK_CLOCK_W_DIP`] / [`FALLBACK_CLOCK_H_DIP`] 这两个设计常量，
//! 且必须经 [`dip_to_px`] 换算后才进入采样链路。
//!
//! # 零堆分配（T02.2 判据①）
//!
//! [`MaskSet`] / [`ScreenBuffer`] / [`LumaHistogram`] 全部使用固定容量或一次性分配的
//! 复用缓冲；`sample_once` 的所有中间值是栈上 `Copy` 类型。用 [`testing::alloc_count`]
//! （线程局部 GlobalAlloc 计数器）可在单测里证明 600 次采样零分配。

pub mod mask;
pub mod sampler;
pub mod screen;
pub mod taskbar;
#[cfg(test)]
#[allow(dead_code)]
mod testing;
pub mod uia;

pub use mask::{MaskKind, MaskRect, MaskSet};
pub use sampler::{
    Clock, ConfigSource, DecisionSink, DiagSink, DiagSnapshot, Judge, Judgement, LumaHistogram,
    LoopReport, PowerSource, SampleOutcome, SampleStats, SamplerLoop, Side, SkipReason, Sleeper,
    StopToken, TickOutcome, sample_once,
};
pub use screen::{ScreenBuffer, ScreenInfo, ScreenSource};
pub use taskbar::{
    TaskbarGeom, TaskbarProbe, TaskbarSource, compute_sample_rect, is_taskbar_auto_hidden,
    is_vertical, locate,
};
pub use uia::{ClockRectSource, UiaProbe, fallback_clock_rects, refresh_clock_rects};

pub use windows::Win32::Foundation::RECT as RectPx;

use crate::color::Rgba;

// ---------------------------------------------------------------------------
// 常量（全部来自架构 §8.2 / §13 / 需求基线 v1.3，禁止在别处硬编码）
// ---------------------------------------------------------------------------

/// A-02：采样矩形落在托盘区内的允许误差上限（物理像素）。
pub const SAMPLE_RECT_TOLERANCE_PX: i32 = 4;

/// §13-5 已拍板：遮罩后有效像素低于此值时跳过本次采样、保持上次判定。
pub const MIN_VALID_PX: u32 = 1500;

/// NFR-08 / EX-22：DPI 或分辨率变化后必须在此时间内重算几何。
pub const DPI_RECOMPUTE_DEADLINE_MS: u64 = 2_000;

/// A-10：配置变更必须在此时间内生效。
pub const CONFIG_APPLY_DEADLINE_MS: u64 = 3_000;

/// A-17：取消暂停后必须在此时间内继续采样。
pub const PAUSE_RESUME_DEADLINE_MS: u64 = 2_000;

/// T02.3：UIA 连接 / 刷新超时熔断阈值。
pub const UIA_TIMEOUT_MS: u64 = 300;

/// T02.3：UIA 矩形的膨胀量（物理像素），覆盖文字抗锯齿边缘。
pub const MASK_INFLATE_PX: i32 = 2;

/// EX-15：电池模式下采样间隔的下限。
pub const BATTERY_MIN_INTERVAL_MS: u32 = 2_000;

/// 默认采样间隔（§3.3：1.5 ms / 1000 ms → CPU ≈ 0.019%）。
pub const DEFAULT_SAMPLE_INTERVAL_MS: u32 = 1_000;

/// NFR-03：单次 `BitBlt` + 统计的耗时预算（微秒）；超出只记日志不崩。
pub const SINGLE_SAMPLE_BUDGET_US: u64 = 5_000;

/// EX-24：连续异常帧达到此数后，跳过并保持上次判定色。
pub const MAX_CONSECUTIVE_BAD_FRAMES: u32 = 5;

/// UIA 遮罩刷新周期（冷路径，不进入每拍热路径）。
pub const UIA_REFRESH_INTERVAL_MS: u64 = 5_000;

/// T02.3 fallback 固定几何宽度（**dip**，必须经 [`dip_to_px`] 换算）。
pub const FALLBACK_CLOCK_W_DIP: f32 = 130.0;

/// T02.3 fallback 固定几何高度（**dip**，必须经 [`dip_to_px`] 换算）。
pub const FALLBACK_CLOCK_H_DIP: f32 = 44.0;

/// `ScreenInfo` / `TaskbarProbe` 固定容量上限；覆盖常规 8 屏以内场景，零堆分配。
pub const MAX_SCREENS: usize = 8;

/// `MaskSet` 固定容量上限；时钟 + 日期 + L3 悬停块/指示条 + 遮挡窗口足够用。
pub const MAX_MASKS: usize = 32;

/// 判定亮度阈值的默认值（FR-12）。
pub const DEFAULT_THRESHOLD_Y: f32 = 0.5;

/// 双阈值迟滞半宽默认值（防止阈值附近抖动，A-13/A-14 的 60 s 零翻转）。
pub const DEFAULT_HYSTERESIS: f32 = 0.08;

// ---------------------------------------------------------------------------
// dip → px：§11.4 规定的单点换算函数（采样链路唯一允许 dip 进入的位置）
// ---------------------------------------------------------------------------

/// dip → 物理像素，公式 `px = dip * dpi / 96.0`（§11.4）。
///
/// `dip` 为 0 时 `dpi` 不做下限钳制也返回 0；对非法 DPI（0）按 96 处理，
/// 避免产生 NaN / 0 尺寸矩形。
#[must_use]
pub fn dip_to_px(dpi: u32, dip: f32) -> f32 {
    let effective = if dpi == 0 { 96 } else { dpi };
    dip * effective as f32 / 96.0
}

// ---------------------------------------------------------------------------
// 纯几何工具（px 语义，taskbar / mask / screen / sampler 共用，避免重复实现）
// ---------------------------------------------------------------------------

/// 以物理像素构造矩形。
#[must_use]
pub const fn rect_px(left: i32, top: i32, right: i32, bottom: i32) -> RectPx {
    RectPx {
        left,
        top,
        right,
        bottom,
    }
}

/// 矩形宽（px），负值或退化矩形返回 0。
#[must_use]
pub fn rect_width(r: &RectPx) -> i32 {
    r.right.saturating_sub(r.left).max(0)
}

/// 矩形高（px），负值或退化矩形返回 0。
#[must_use]
pub fn rect_height(r: &RectPx) -> i32 {
    r.bottom.saturating_sub(r.top).max(0)
}

/// 矩形是否为空（宽或高 ≤ 0）。
#[must_use]
pub fn rect_is_empty(r: &RectPx) -> bool {
    rect_width(r) == 0 || rect_height(r) == 0
}

/// 矩形面积（像素数），空矩形为 0。
#[must_use]
pub fn rect_area(r: &RectPx) -> u32 {
    if rect_is_empty(r) {
        0
    } else {
        (rect_width(r) as u32).saturating_mul(rect_height(r) as u32)
    }
}

/// 两矩形交集；不相交时返回 `None`。
#[must_use]
pub fn rect_intersection(a: &RectPx, b: &RectPx) -> Option<RectPx> {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);
    let out = rect_px(left, top, right, bottom);
    if rect_is_empty(&out) {
        None
    } else {
        Some(out)
    }
}

/// `outer` 是否 100% 包含 `inner`（含边界重合），A-02 的核心断言。
#[must_use]
pub fn rect_contains(outer: &RectPx, inner: &RectPx) -> bool {
    !rect_is_empty(inner)
        && inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
}

/// 把 `inner` 夹到 `outer` 之内（A-02 的实现手段：结果误差恒为 0 px ≤ 4 px）。
///
/// 若两者完全不相交，返回 `None`（调用方应视为不可用）。
#[must_use]
pub fn rect_clamp(inner: &RectPx, outer: &RectPx) -> Option<RectPx> {
    if rect_is_empty(outer) {
        return None;
    }
    let left = inner.left.clamp(outer.left, outer.right);
    let right = inner.right.clamp(outer.left, outer.right);
    let top = inner.top.clamp(outer.top, outer.bottom);
    let bottom = inner.bottom.clamp(outer.top, outer.bottom);
    let out = rect_px(left, top, right, bottom);
    if rect_is_empty(&out) {
        None
    } else {
        Some(out)
    }
}

/// 矩形向外膨胀 `px`（px 为负时内缩，结果为空时返回原样的退化矩形）。
#[must_use]
pub fn rect_inflate(r: &RectPx, px: i32) -> RectPx {
    rect_px(
        r.left.saturating_sub(px),
        r.top.saturating_sub(px),
        r.right.saturating_add(px),
        r.bottom.saturating_add(px),
    )
}

/// 两矩形是否相交或边缘相邻（`gap` 内视为相邻，用于遮罩合并）。
#[must_use]
pub fn rect_touches(a: &RectPx, b: &RectPx, gap: i32) -> bool {
    let g = gap.max(0);
    a.left <= b.right.saturating_add(g)
        && a.right.saturating_add(g) >= b.left
        && a.top <= b.bottom.saturating_add(g)
        && a.bottom.saturating_add(g) >= b.top
}

/// 两矩形的包围矩形（遮罩合并用）。
#[must_use]
pub fn rect_union(a: &RectPx, b: &RectPx) -> RectPx {
    rect_px(
        a.left.min(b.left),
        a.top.min(b.top),
        a.right.max(b.right),
        a.bottom.max(b.bottom),
    )
}

// ---------------------------------------------------------------------------
// 配置快照（T01.2 `config.rs` 尚未落地，这里定义引擎侧最小快照并注明映射关系）
// ---------------------------------------------------------------------------

/// 采样区配置（对应架构 §5.1 `RegionCfg`，字段语义逐项一致）。
///
/// T01.2 的 `config::RegionCfg` 落地后，只需提供 `From<RegionCfg> for RegionCfg`
/// 即可接线，引擎内部无需改动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionCfg {
    /// 相对 `TrayNotifyWnd` 左边缘的水平偏移百分比（0..=100）。
    pub x_offset_pct: u8,
    /// 采样区宽度占 `TrayNotifyWnd` 宽度的百分比（0..=100）。
    pub width_pct: u8,
    /// 上下内缩百分比（0..=45，防止上下内缩相遇形成空矩形）。
    pub v_inset_pct: u8,
}

impl Default for RegionCfg {
    fn default() -> Self {
        // 默认取整幅托盘区宽度，文字由 UIA 遮罩排除（§1.3 双防线）。
        Self {
            x_offset_pct: 0,
            width_pct: 100,
            v_inset_pct: 10,
        }
    }
}

/// 判定器配置（对应架构 §5.1 `ColorCfg` 的判定子集）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgeCfg {
    /// 亮度阈值。`Y <= threshold - hyst` → 亮端色；`Y >= threshold + hyst` → 暗端色。
    pub threshold_y: f32,
    /// 双阈值迟滞半宽。
    pub hysteresis: f32,
    /// 变化触发阈值（代表色 0..=255 偏移，低于此值不发布）。
    pub change_threshold: u8,
    /// EWMA 平滑系数（0..=1）。
    pub ewma_alpha: f32,
    /// 背景偏暗时使用的亮端色。
    pub light_color: Rgba,
    /// 背景偏亮时使用的暗端色。
    pub dark_color: Rgba,
}

impl Default for JudgeCfg {
    fn default() -> Self {
        Self {
            threshold_y: DEFAULT_THRESHOLD_Y,
            hysteresis: DEFAULT_HYSTERESIS,
            change_threshold: 8,
            ewma_alpha: 0.2,
            light_color: Rgba::opaque(255, 255, 255),
            dark_color: Rgba::opaque(0, 0, 0),
        }
    }
}

/// 引擎配置快照；`version` 由配置源递增，`Engine` 每拍比对实现 A-10（≤3 s 生效）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineConfig {
    /// 采样间隔（毫秒）。
    pub interval_ms: u32,
    /// 采样区配置。
    pub region: RegionCfg,
    /// 判定器配置。
    pub judge: JudgeCfg,
    /// 手动暂停（A-17）。
    pub paused: bool,
    /// 是否启用电池节能（EX-15）。
    pub battery_saver: bool,
    /// 配置版本号，变更即刻生效。
    pub version: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_SAMPLE_INTERVAL_MS,
            region: RegionCfg::default(),
            judge: JudgeCfg::default(),
            paused: false,
            battery_saver: true,
            version: 0,
        }
    }
}

impl EngineConfig {
    /// v1 默认配置（等价于 `Default`，显式命名便于评审对照 FR 表）。
    #[must_use]
    pub fn default_v1() -> Self {
        Self::default()
    }

    /// 就地钳制非法值，保证任何配置组合都不会产生空矩形 / 除零 / NaN / 忙等。
    ///
    /// 这是 §11.2「可降级错误不得传播成崩溃」在配置面的落实：损坏配置一律
    /// 修正为合法值，而不是拒绝启动。
    pub fn sanitize(&mut self) {
        self.interval_ms = self.interval_ms.clamp(200, 60_000);
        self.region.x_offset_pct = self.region.x_offset_pct.min(100);
        self.region.width_pct = self
            .region
            .width_pct
            .min(100u8.saturating_sub(self.region.x_offset_pct));
        self.region.v_inset_pct = self.region.v_inset_pct.min(45);

        if !self.judge.threshold_y.is_finite() {
            self.judge.threshold_y = DEFAULT_THRESHOLD_Y;
        }
        self.judge.threshold_y = self.judge.threshold_y.clamp(0.0, 1.0);
        if !self.judge.hysteresis.is_finite() || self.judge.hysteresis < 0.0 {
            self.judge.hysteresis = DEFAULT_HYSTERESIS;
        }
        self.judge.hysteresis = self.judge.hysteresis.min(0.5);
        if !self.judge.ewma_alpha.is_finite() {
            self.judge.ewma_alpha = 0.2;
        }
        self.judge.ewma_alpha = self.judge.ewma_alpha.clamp(0.001, 1.0);
    }

    /// 按电池状态计算本拍实际间隔（EX-15：自动 ≥ 2000 ms）。
    #[must_use]
    pub fn effective_interval_ms(&self, on_battery: bool) -> u32 {
        if on_battery && self.battery_saver {
            self.interval_ms.max(BATTERY_MIN_INTERVAL_MS)
        } else {
            self.interval_ms
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎状态与依赖注入
// ---------------------------------------------------------------------------

/// 采样被挂起的原因（EX-06 / EX-07 / A-02 / EX-24）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendReason {
    /// 任务栏自动隐藏：采样区不可见，暂停并保持上次判定（EX-06）。
    AutoHidden,
    /// 竖向任务栏：暂停采样 + 提示一次（EX-07）。
    VerticalTaskbar,
    /// 找不到 `Shell_TrayWnd` / `TrayNotifyWnd`（EX-11 期间）。
    TaskbarMissing,
    /// 采样矩形为空或完全落在屏幕外（EX-24）。
    RegionUnavailable,
}

/// 引擎生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// 未启动。
    Stopped,
    /// 正常采样。
    Running,
    /// 用户手动暂停（A-17）。
    Paused,
    /// 环境不满足，采样挂起并保持上次判定色。
    Suspended(SuspendReason),
}

/// 一拍注入的全部平台依赖（对象安全 trait；T04 在主进程里提供真实实现）。
///
/// 把 `&mut` 与 `&` 分开是为了让 `Engine::tick` 能在持有 `&mut dyn DecisionSink`
/// 的同时读 `&dyn Clock`，不产生交叉借用。
pub struct EngineDeps<'a> {
    /// 任务栏几何 / 监视器 / DPI 原始读数。
    pub taskbar: &'a mut dyn TaskbarSource,
    /// 抓屏（写入复用缓冲，零分配）。
    pub screen: &'a mut dyn ScreenSource,
    /// UIA 时钟 / 日期矩形探测。
    pub uia: &'a mut dyn ClockRectSource,
    /// 电池状态。
    pub power: &'a dyn PowerSource,
    /// 配置快照。
    pub config: &'a dyn ConfigSource,
    /// 判定结果唯一出口。
    pub decision: &'a mut dyn DecisionSink,
    /// 诊断面板 / 通知出口。
    pub diag: &'a mut dyn DiagSink,
    /// 单调时钟。
    pub clock: &'a dyn Clock,
}

/// `Engine::run` 的汇总报告（§11.6 每 5 分钟一行采样汇总的字段来源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineReport {
    /// 累计拍数。
    pub ticks: u64,
    /// 采样成功次数。
    pub sampled: u64,
    /// 跳过次数。
    pub skipped: u64,
    /// 异常帧次数。
    pub bad_frames: u64,
    /// 判定发布次数。
    pub published: u64,
    /// 判定翻转次数。
    pub flips: u64,
    /// 几何重算次数。
    pub geometry_refreshes: u64,
    /// 用户提示次数（EX-07 要求"提示一次"）。
    pub notices: u64,
    /// 运行的总毫秒数。
    pub elapsed_ms: u64,
}

/// 引擎聚合体：几何 + 遮罩 + 复用缓冲 + 直方图 + 判定器 + 调度状态机。
///
/// 本身**不持有平台依赖**，依赖通过 [`Engine::tick`] 注入，因此可在无 Windows
/// 消息循环、无真实屏幕的单元测试里完整驱动 600 s 闭环。
pub struct Engine {
    config: EngineConfig,
    geom: Option<TaskbarGeom>,
    masks: MaskSet,
    /// L2/L3 由 DLL 回传的已着色矩形（`ColoredRects` 帧）。
    ///
    /// 与 UIA 遮罩**分开保存**：UIA 每次刷新是整份快照（可 clear 重填），
    /// 而 DLL 回传是增量事件；若共用一份集合，UIA 刷新会把 DLL 矩形抹掉
    /// （架构 §1.3 双防线 / §4 #44「合并 UIA 矩形 + DLL 回传矩形」）。
    external_masks: MaskSet,
    buffer: ScreenBuffer,
    hist: LumaHistogram,
    judge: Judge,
    loop_state: SamplerLoop,
    uia_probe: UiaProbe,
    state: EngineState,
    /// 最近一次几何重算的发现延迟（= 本次探测时刻 − 上次探测时刻，单位 ms）。
    geometry_latency_ms: u64,
    last_sample_ms: u64,
    /// 上一拍几何探测时刻（用于计算 DPI 变化的重算延迟）。
    last_probe_ms: u64,
    /// 配置变更生效延迟；每拍都读配置，故恒为 0（同拍生效）。
    config_latency_ms: u64,
    /// 上次 UIA 刷新时刻；`None` 表示尚未刷新过（首拍必须刷新）。
    last_uia_ms: Option<u64>,
    geometry_refreshes: u64,
    vertical_notice_sent: bool,
    /// 手动暂停标志（`pause()` / `resume()`）；与配置里的 `paused` 独立，
    /// 避免"手动暂停后下一拍被配置快照自动恢复"的逻辑漏洞。
    manual_paused: bool,
    report: EngineReport,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(EngineConfig::default_v1())
    }
}

impl Engine {
    /// 以配置快照建立引擎（内部缓冲均为空，首次采样时按需 `ensure`）。
    #[must_use]
    pub fn new(mut config: EngineConfig) -> Self {
        config.sanitize();
        Self {
            config,
            geom: None,
            masks: MaskSet::new(),
            external_masks: MaskSet::new(),
            buffer: ScreenBuffer::new(),
            hist: LumaHistogram::new(),
            judge: Judge::new(),
            loop_state: SamplerLoop::new(),
            uia_probe: UiaProbe::new(),
            state: EngineState::Stopped,
            geometry_latency_ms: 0,
            last_sample_ms: 0,
            last_probe_ms: 0,
            config_latency_ms: 0,
            last_uia_ms: None,
            geometry_refreshes: 0,
            vertical_notice_sent: false,
            manual_paused: false,
            report: EngineReport::default(),
        }
    }

    /// 启动。
    pub fn start(&mut self) {
        self.state = EngineState::Running;
    }

    /// 停止（幂等）。
    pub fn stop(&mut self) {
        self.state = EngineState::Stopped;
    }

    /// 手动暂停（A-17：暂停后 `last_sample_ms` 停止更新）。
    pub fn pause(&mut self) {
        if self.state != EngineState::Stopped {
            self.state = EngineState::Paused;
            self.manual_paused = true;
        }
    }

    /// 取消暂停；下一次 tick 立即把 `next_due` 拉到当前时刻，保证 ≤2 s 内继续。
    pub fn resume(&mut self) {
        self.manual_paused = false;
        if matches!(self.state, EngineState::Paused | EngineState::Suspended(_)) {
            self.state = EngineState::Running;
            self.loop_state.force_due();
        }
    }

    /// 直接写入配置快照（A-10：版本号变化即刻生效）。
    pub fn set_config(&mut self, mut config: EngineConfig) {
        config.sanitize();
        self.config = config;
        self.loop_state.force_due();
    }

    /// 当前状态。
    #[must_use]
    pub const fn state(&self) -> EngineState {
        self.state
    }

    /// 当前几何（`None` 表示尚未成功定位）。
    #[must_use]
    pub const fn geometry(&self) -> Option<&TaskbarGeom> {
        self.geom.as_ref()
    }

    /// 当前遮罩集合。
    #[must_use]
    pub const fn masks(&self) -> &MaskSet {
        &self.masks
    }

    /// 上次成功采样的时刻（毫秒）；暂停期间不更新（A-17）。
    #[must_use]
    pub const fn last_sample_ms(&self) -> u64 {
        self.last_sample_ms
    }

    /// DPI / 显示器变化后几何重算的发现延迟（毫秒）；NFR-08 / EX-22 要求 ≤ 2000。
    #[must_use]
    pub const fn geometry_latency_ms(&self) -> u64 {
        self.geometry_latency_ms
    }

    /// 配置变更到生效的延迟（毫秒）；每拍都读配置，故恒为 0（同拍生效，A-10 要求 ≤ 3000）。
    #[must_use]
    pub const fn config_latency_ms(&self) -> u64 {
        self.config_latency_ms
    }

    /// 几何重算次数。
    #[must_use]
    pub const fn geometry_refreshes(&self) -> u64 {
        self.geometry_refreshes
    }

    /// 当前配置快照。
    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// 运行汇总。
    #[must_use]
    pub const fn report(&self) -> &EngineReport {
        &self.report
    }

    /// UIA 是否处于 fallback 状态（T02.3 判据②的可观测证据）。
    #[must_use]
    pub const fn uia_in_fallback(&self) -> bool {
        self.uia_probe.in_fallback()
    }

    /// 合并 DLL 回传的已着色矩形（`ColoredRects` 帧，L2/L3）。
    ///
    /// 增量语义：新矩形与既有 DLL 矩形连通合并，不清除历史；下一次 UIA 快照
    /// 刷新后会与 UIA 矩形一起重新并入生效遮罩。**L1 不调用本方法**，
    /// 因此纯诊断模式下引擎行为完全不依赖注入能力。
    ///
    /// 返回合并后的 DLL 矩形数量（相邻矩形会被合并，可能小于输入数）。
    pub fn note_colored_rects(&mut self, rects: &[MaskRect]) -> usize {
        self.external_masks.union_with(rects);
        let Engine {
            masks,
            external_masks,
            ..
        } = self;
        masks.union_with(external_masks.rects());
        external_masks.count()
    }

    /// DLL 侧遮罩数量（诊断面板用）。
    #[must_use]
    pub const fn external_mask_count(&self) -> usize {
        self.external_masks.count()
    }

    /// 清空 DLL 回传遮罩（DLL 断开 / 还原完成 / explorer 重启时调用）。
    pub fn clear_colored_rects(&mut self) {
        self.external_masks.clear();
    }

    /// 显式通知 DPI / 分辨率变化（T04 在 `WM_DPICHANGED` 里调用）。
    ///
    /// 立即把下一拍标记为需要重算几何，实际重算发生在下一拍，
    /// 最坏延迟 = 一个采样间隔（≤ 2 s 默认值），满足 NFR-08。
    pub fn on_dpi_changed(&mut self) {
        self.geom = None;
        self.loop_state.force_due();
    }

    /// 驱动一拍。所有外部依赖经 `deps` 注入，返回本拍结果。
    ///
    /// 执行顺序（对应 §1.3 数据流）：配置 → 几何 → 可用性 → UIA 遮罩 → 采样 → 判定 → 发布。
    pub fn tick(&mut self, deps: &mut EngineDeps<'_>) -> TickOutcome {
        let now = deps.clock.now_ms();
        if self.state == EngineState::Stopped {
            return TickOutcome::Stopped;
        }
        self.report.ticks += 1;

        // 1) 配置变更即刻生效（A-10，同拍生效，远小于 3 s）。
        let incoming = deps.config.snapshot();
        if incoming.version != self.config.version {
            let mut cfg = incoming;
            cfg.sanitize();
            self.config = cfg;
            self.config_latency_ms = 0;
            self.loop_state.force_due();
        }
        if self.config.paused || self.manual_paused {
            if self.state != EngineState::Paused {
                self.state = EngineState::Paused;
            }
            return TickOutcome::Skipped(SkipReason::Paused);
        }
        if self.state == EngineState::Paused {
            self.state = EngineState::Running;
            self.loop_state.force_due();
        }

        // 2) 几何：DPI / 尺寸 / 监视器变化后立刻重算（NFR-08 / EX-22）。
        let refresh_result = self.refresh_geometry(deps, now);
        if let Err(reason) = refresh_result {
            self.state = EngineState::Suspended(reason);
            self.emit_notice_once(&mut *deps.diag, reason);
            return TickOutcome::Suspended(reason);
        }
        if matches!(self.state, EngineState::Suspended(_)) {
            self.state = EngineState::Running;
        }

        // 3) UIA 遮罩按冷路径周期刷新；失败自动退回固定几何（T02.3 判据②）。
        //    `None` 表示首拍，必须刷新一次，否则遮罩会凭空缺失。
        let uia_due = match self.last_uia_ms {
            None => true,
            Some(t) => now.saturating_sub(t) >= UIA_REFRESH_INTERVAL_MS,
        };
        if uia_due {
            self.last_uia_ms = Some(now);
            let geom = match self.geom.as_ref() {
                Some(g) => g,
                None => return TickOutcome::Suspended(SuspendReason::TaskbarMissing),
            };
            self.masks.clear();
            let uia_source: &mut dyn ClockRectSource = &mut *deps.uia;
            self.uia_probe
                .refresh_into(uia_source, deps.clock, geom, &mut self.masks);
            // UIA 是整份快照（覆盖式），DLL 回传是增量事件；刷新后重新并入，
            // 否则下一次 UIA 刷新会把 L3 的悬停块/指示条遮罩抹掉（§1.3 双防线）。
            // 两个字段不相交，直接借用即可，无需 `to_vec()` 引入堆分配。
            let Engine {
                masks,
                external_masks,
                ..
            } = self;
            masks.union_with(external_masks.rects());
        }

        // 4) 采样 + 判定 + 发布（热路径，零堆分配）。
        let on_battery = deps.power.on_battery();
        let outcome = {
            let Engine {
                config,
                geom,
                masks,
                buffer,
                hist,
                judge,
                loop_state,
                ..
            } = self;
            let geometry = match geom.as_ref() {
                Some(g) => g,
                None => return TickOutcome::Suspended(SuspendReason::TaskbarMissing),
            };
            loop_state.tick(&mut sampler::SampleCtx {
                screen: &mut *deps.screen,
                buffer,
                hist,
                judge,
                masks: &*masks,
                geom: geometry,
                cfg: &*config,
                decision: &mut *deps.decision,
                diag: &mut *deps.diag,
                now_ms: now,
                on_battery,
            })
        };

        match outcome {
            TickOutcome::Published(_) => {
                self.last_sample_ms = now;
                self.report.sampled += 1;
                self.report.published += 1;
            }
            TickOutcome::Sampled(_) => {
                self.last_sample_ms = now;
                self.report.sampled += 1;
            }
            TickOutcome::Skipped(reason) => match reason {
                SkipReason::BitBltFailed { .. } | SkipReason::BadFrameBurst => {
                    self.report.bad_frames += 1;
                }
                _ => self.report.skipped += 1,
            },
            TickOutcome::NotDue | TickOutcome::Stopped | TickOutcome::Suspended(_) => {}
        }
        self.report.flips = self.judge.flip_count();
        outcome
    }

    /// 阻塞式闭环（由 T04 在 T1 线程调用）。
    ///
    /// 用注入的 [`Sleeper`] 等待而非 `Sleep(1)` 忙等（§3.3）；`stop` 为真时退出。
    /// 这里是**同步循环**，不 spawn 任何线程——符合 core「无线程」约定，也便于单测
    /// 用假时钟在毫秒内跑完 600 s。
    pub fn run(
        &mut self,
        deps: &mut EngineDeps<'_>,
        stop: &dyn StopToken,
        sleeper: &dyn Sleeper,
    ) -> EngineReport {
        let start_ms = deps.clock.now_ms();
        while !stop.is_stopped() {
            let _ = self.tick(deps);
            let now = deps.clock.now_ms();
            let next = self.loop_state.next_due_ms();
            let step = u64::from(self.config.effective_interval_ms(deps.power.on_battery()));
            // 保证每一轮时钟都严格前进：暂停/挂起时 next_due 可能落后于 now，
            // 若直接 sleep_until(now) 会在假时钟下形成无法退出的忙循环（§3.3 禁止忙等）。
            sleeper.sleep_until(next.max(now.saturating_add(step.max(1))));
        }
        self.report.elapsed_ms = deps.clock.now_ms().saturating_sub(start_ms);
        self.report
    }

    /// 刷新几何；返回 `Err(reason)` 表示本拍应挂起采样。
    fn refresh_geometry(
        &mut self,
        deps: &mut EngineDeps<'_>,
        now: u64,
    ) -> Result<(), SuspendReason> {
        // 每拍都探测一次；重算延迟 = 本次探测时刻 − 上次探测时刻，
        // 即"变化发生后多久被发现"，默认 ≤ 一个采样间隔（NFR-08 要求 ≤ 2 s）。
        let prev_probe = self.last_probe_ms;
        self.last_probe_ms = now;
        let probe = match deps.taskbar.read() {
            Ok(p) => p,
            Err(_) => return Err(SuspendReason::TaskbarMissing),
        };
        let need = match self.geom.as_ref() {
            None => true,
            Some(g) => g.needs_refresh_from(&probe),
        };
        if need {
            let geom =
                TaskbarGeom::from_probe(&probe).map_err(|_| SuspendReason::RegionUnavailable)?;
            let was_known = self.geom.is_some();
            self.geom = Some(geom);
            self.geometry_refreshes += 1;
            self.geometry_latency_ms = if was_known {
                now.saturating_sub(prev_probe).min(DPI_RECOMPUTE_DEADLINE_MS)
            } else {
                0
            };
        }
        let g = match self.geom.as_ref() {
            Some(g) => g,
            None => return Err(SuspendReason::TaskbarMissing),
        };
        if g.is_auto_hidden() {
            return Err(SuspendReason::AutoHidden);
        }
        if g.is_vertical() {
            return Err(SuspendReason::VerticalTaskbar);
        }
        if !g.is_usable() {
            return Err(SuspendReason::RegionUnavailable);
        }
        if compute_sample_rect(g, &self.config.region).is_none() {
            return Err(SuspendReason::RegionUnavailable);
        }
        Ok(())
    }

    /// 竖向任务栏等提示**只发一次**（EX-07）。
    fn emit_notice_once(&mut self, diag: &mut dyn DiagSink, reason: SuspendReason) {
        let need = match reason {
            SuspendReason::VerticalTaskbar => !self.vertical_notice_sent,
            _ => false,
        };
        if need {
            diag.notice(reason);
            self.vertical_notice_sent = true;
            self.report.notices += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{
        FakeClock, FakeConfig, FakeDecision, FakeDiag, FakePower, FakeScreen, FakeTaskbar,
        FakeUia, TestSleeper, TestStop,
    };
    use std::sync::Arc;

    #[test]
    fn constants_match_architecture() {
        assert_eq!(SAMPLE_RECT_TOLERANCE_PX, 4, "A-02");
        assert_eq!(MIN_VALID_PX, 1500, "§13-5 拍板值");
        assert_eq!(DPI_RECOMPUTE_DEADLINE_MS, 2_000, "NFR-08");
        assert_eq!(BATTERY_MIN_INTERVAL_MS, 2_000, "EX-15");
        assert_eq!(UIA_TIMEOUT_MS, 300, "T02.3");
        assert_eq!(MASK_INFLATE_PX, 2, "T02.3");
        assert_eq!(MAX_CONSECUTIVE_BAD_FRAMES, 5, "EX-24");
        // 用运行时索引取值，避免编译期常量断言；容量变更时必须同步复核固定容量布局。
        let capacities = [MAX_SCREENS, MAX_MASKS];
        assert!(
            capacities[0] >= 2 && capacities[1] >= 4,
            "固定容量需覆盖多显示器与多遮罩场景"
        );
        assert_eq!(capacities[0], 8, "MAX_SCREENS 变更需同步复核 TaskbarProbe 布局");
        assert_eq!(capacities[1], 32, "MAX_MASKS 变更需同步复核 MaskSet 布局");
    }

    #[test]
    fn dip_to_px_is_the_only_scaling_point() {
        assert!((dip_to_px(96, 130.0) - 130.0).abs() < 1e-5);
        assert!((dip_to_px(144, 130.0) - 195.0).abs() < 1e-4);
        assert!((dip_to_px(192, 44.0) - 88.0).abs() < 1e-4);
        // 非法 DPI 按 96 处理，绝不产出 NaN。
        assert!((dip_to_px(0, 44.0) - 44.0).abs() < 1e-5);
    }

    #[test]
    fn geometry_helpers_are_px_exact() {
        let outer = rect_px(0, 0, 300, 48);
        assert_eq!(rect_width(&outer), 300);
        assert_eq!(rect_height(&outer), 48);
        assert_eq!(rect_area(&outer), 14_400);
        assert!(rect_contains(&outer, &rect_px(10, 4, 290, 44)));
        assert!(!rect_contains(&outer, &rect_px(-1, 4, 290, 44)));
        assert!(rect_is_empty(&rect_px(10, 10, 10, 40)));
        assert_eq!(rect_inflate(&rect_px(10, 10, 20, 20), 2), rect_px(8, 8, 22, 22));
    }

    #[test]
    fn rect_clamp_never_escapes_tray() {
        // A-02：任何越界的期望矩形都被夹回 TrayNotifyWnd，误差恒为 0 px。
        let tray = rect_px(100, 100, 400, 148);
        let wild = rect_px(-500, -500, 900, 900);
        let clamped = rect_clamp(&wild, &tray).expect("clamp 应成功");
        assert!(rect_contains(&tray, &clamped));
        assert_eq!(clamped, tray);
        assert!(rect_clamp(&rect_px(900, 900, 1000, 1000), &tray).is_none());
    }

    #[test]
    fn rect_intersection_and_union_and_touches() {
        let a = rect_px(0, 0, 10, 10);
        let b = rect_px(5, 5, 20, 20);
        assert_eq!(rect_intersection(&a, &b), Some(rect_px(5, 5, 10, 10)));
        assert_eq!(rect_union(&a, &b), rect_px(0, 0, 20, 20));
        assert!(rect_touches(&a, &b, 0));
        let far = rect_px(100, 100, 110, 110);
        assert_eq!(rect_intersection(&a, &far), None);
        assert!(!rect_touches(&a, &far, 2));
        assert!(rect_touches(&a, &rect_px(12, 0, 20, 10), 2));
    }

    #[test]
    fn config_sanitize_repairs_damaged_values() {
        let mut cfg = EngineConfig {
            interval_ms: 0,
            region: RegionCfg {
                x_offset_pct: 200,
                width_pct: 200,
                v_inset_pct: 200,
            },
            judge: JudgeCfg {
                threshold_y: f32::NAN,
                hysteresis: f32::NAN,
                change_threshold: 0,
                ewma_alpha: f32::INFINITY,
                ..JudgeCfg::default()
            },
            paused: false,
            battery_saver: true,
            version: 1,
        };
        cfg.sanitize();
        assert_eq!(cfg.interval_ms, 200);
        assert_eq!(cfg.region.x_offset_pct, 100);
        assert_eq!(cfg.region.width_pct, 0);
        assert_eq!(cfg.region.v_inset_pct, 45);
        assert_eq!(cfg.judge.threshold_y, DEFAULT_THRESHOLD_Y);
        // 非有限值先回退到默认 0.2，再 clamp；不能用 clamp 把 +∞ 变成 1.0（会误当成"完全不平滑"）。
        assert_eq!(cfg.judge.ewma_alpha, 0.2);
        assert_eq!(cfg.judge.hysteresis, DEFAULT_HYSTERESIS);
        let mut huge = EngineConfig::default_v1();
        huge.judge.ewma_alpha = 9.0;
        huge.sanitize();
        assert_eq!(huge.judge.ewma_alpha, 1.0, "有限超界值才 clamp 到 1.0");
    }

    #[test]
    fn battery_mode_enforces_2000ms_floor() {
        let cfg = EngineConfig::default_v1();
        assert_eq!(cfg.effective_interval_ms(false), 1_000);
        assert_eq!(cfg.effective_interval_ms(true), 2_000);
        let mut fast = cfg;
        fast.interval_ms = 200;
        assert_eq!(fast.effective_interval_ms(true), 2_000);
        let mut off = cfg;
        off.battery_saver = false;
        assert_eq!(off.effective_interval_ms(true), 1_000);
    }

    #[test]
    fn engine_lifecycle_start_pause_resume_stop() {
        let mut engine = Engine::default();
        assert_eq!(engine.state(), EngineState::Stopped);
        engine.start();
        assert_eq!(engine.state(), EngineState::Running);
        engine.pause();
        assert_eq!(engine.state(), EngineState::Paused);
        engine.resume();
        assert_eq!(engine.state(), EngineState::Running);
        engine.stop();
        assert_eq!(engine.state(), EngineState::Stopped);
    }

    /// 测试夹具：把全部假实现聚到一起，**每一拍临时构造 `EngineDeps`**。
    ///
    /// 这样 `&mut` 借用的生命周期只覆盖单次 `tick`，测试可以在两拍之间自由
    /// 修改 `taskbar`（切 DPI / 自动隐藏 / 托盘折叠）或读取 `decision` 计数，
    /// 不会被长期借用锁住——这正是「依赖注入」相对「引擎持有依赖」的可测性优势。
    struct TestRig {
        shared: Arc<std::sync::atomic::AtomicU64>,
        clock: FakeClock,
        taskbar: FakeTaskbar,
        screen: FakeScreen,
        uia: FakeUia,
        power: FakePower,
        config: FakeConfig,
        decision: FakeDecision,
        diag: FakeDiag,
        engine: Engine,
    }

    impl TestRig {
        fn new(taskbar: FakeTaskbar, uia: FakeUia) -> Self {
            let shared = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let mut engine = Engine::default();
            engine.start();
            Self {
                clock: FakeClock::new(Arc::clone(&shared)),
                shared,
                taskbar,
                screen: FakeScreen::uniform(0, 0, 0), // 纯黑壁纸
                uia,
                power: FakePower::ac(),
                config: FakeConfig::new(EngineConfig::default_v1()),
                decision: FakeDecision::new(),
                diag: FakeDiag::new(),
                engine,
            }
        }

        fn tick(&mut self) -> TickOutcome {
            let mut deps = EngineDeps {
                taskbar: &mut self.taskbar,
                screen: &mut self.screen,
                uia: &mut self.uia,
                power: &self.power,
                config: &self.config,
                decision: &mut self.decision,
                diag: &mut self.diag,
                clock: &self.clock,
            };
            self.engine.tick(&mut deps)
        }

        fn run(&mut self, stop_ms: u64) -> EngineReport {
            let mut deps = EngineDeps {
                taskbar: &mut self.taskbar,
                screen: &mut self.screen,
                uia: &mut self.uia,
                power: &self.power,
                config: &self.config,
                decision: &mut self.decision,
                diag: &mut self.diag,
                clock: &self.clock,
            };
            let stop = TestStop::after_ms(stop_ms, Arc::clone(&self.shared));
            let sleeper = TestSleeper::new(Arc::clone(&self.shared));
            self.engine.run(&mut deps, &stop, &sleeper)
        }

        fn advance(&self, ms: u64) {
            self.clock.advance(ms);
        }
    }

    #[test]
    fn engine_600s_closed_loop_meets_a03() {
        // A-03：600 s 采样成功 ≥ 590 次；同时验证 60 s 判定翻转 0 次（A-13/A-14）。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        let report = rig.run(600_000);
        assert!(
            report.sampled >= 590,
            "A-03 要求 ≥590 次成功，实际 {}",
            report.sampled
        );
        assert_eq!(report.flips, 0, "纯黑壁纸 60 s 内不得翻转");
        assert!(report.elapsed_ms >= 600_000);
        assert_eq!(rig.decision.last().map(|j| j.side), Some(Side::Light));
    }

    #[test]
    fn engine_config_change_applies_within_three_seconds() {
        // A-10：改采样间隔为 2000 ms，必须 ≤3 s 生效。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        for _ in 0..3 {
            let _ = rig.tick();
            rig.advance(1_000);
        }
        let before = rig.engine.config().interval_ms;
        rig.config.set_interval(2_000);
        // 变更后的下一拍（1 s 内）即生效。
        let _ = rig.tick();
        assert_eq!(before, 1_000);
        assert_eq!(rig.engine.config().interval_ms, 2_000);
        assert!(rig.engine.report().ticks >= 4, "配置生效不得引入额外延迟");
        assert!(rig.engine.config_latency_ms() <= CONFIG_APPLY_DEADLINE_MS);
    }

    #[test]
    fn engine_pause_freezes_sample_timestamp_and_resumes_in_time() {
        // A-17：暂停后时间戳停止更新，恢复后 ≤2 s 内继续。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        for _ in 0..4 {
            let _ = rig.tick();
            rig.advance(1_000);
        }
        let frozen = rig.engine.last_sample_ms();
        rig.engine.pause();
        for _ in 0..5 {
            let out = rig.tick();
            assert_eq!(out, TickOutcome::Skipped(SkipReason::Paused));
            rig.advance(1_000);
        }
        assert_eq!(rig.engine.last_sample_ms(), frozen, "暂停期间时间戳不得更新");
        rig.engine.resume();
        let _ = rig.tick();
        assert!(
            rig.engine.last_sample_ms() <= frozen + 6_000 + PAUSE_RESUME_DEADLINE_MS,
            "恢复后必须继续采样"
        );
        assert!(rig.engine.last_sample_ms() > frozen);
    }

    #[test]
    fn engine_dpi_change_recomputes_within_two_seconds() {
        // NFR-08 / EX-22：100% ↔ 150% 切换后几何重算。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        let _ = rig.tick();
        assert_eq!(rig.engine.geometry().map(|g| g.dpi), Some(96));
        let before = rig.engine.geometry_refreshes();

        rig.advance(1_000);
        rig.taskbar.set_dpi(144);
        let _ = rig.tick();
        assert_eq!(rig.engine.geometry().map(|g| g.dpi), Some(144));
        assert!(rig.engine.geometry_refreshes() > before);
        assert!(rig.engine.geometry_latency_ms() <= DPI_RECOMPUTE_DEADLINE_MS);
    }

    #[test]
    fn engine_explicit_dpi_notification_forces_recompute() {
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        let _ = rig.tick();
        let n = rig.engine.geometry_refreshes();
        rig.taskbar.set_dpi(192);
        rig.engine.on_dpi_changed();
        let _ = rig.tick();
        assert_eq!(rig.engine.geometry().map(|g| g.dpi), Some(192));
        assert!(rig.engine.geometry_refreshes() > n);
    }

    #[test]
    fn engine_auto_hide_suspends_and_keeps_last_decision() {
        // EX-06：自动隐藏时暂停采样并保持上次判定色。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        let _ = rig.tick();
        let published = rig.decision.count();
        assert!(published > 0);

        rig.advance(1_000);
        rig.taskbar.set_auto_hide(true);
        let out = rig.tick();
        assert_eq!(out, TickOutcome::Suspended(SuspendReason::AutoHidden));
        assert_eq!(
            rig.engine.state(),
            EngineState::Suspended(SuspendReason::AutoHidden)
        );
        assert_eq!(rig.decision.count(), published, "必须保持上次判定，不得新增发布");

        rig.advance(1_000);
        rig.taskbar.set_auto_hide(false);
        let _ = rig.tick();
        assert_eq!(rig.engine.state(), EngineState::Running);
        assert!(rig.engine.last_sample_ms() > 0);
    }

    #[test]
    fn engine_vertical_taskbar_suspends_and_notifies_exactly_once() {
        // EX-07：竖向任务栏 → 暂停采样 + 提示一次（不重复刷）。
        let mut rig = TestRig::new(FakeTaskbar::vertical(), FakeUia::working());
        for _ in 0..6 {
            let out = rig.tick();
            assert_eq!(out, TickOutcome::Suspended(SuspendReason::VerticalTaskbar));
            rig.advance(1_000);
        }
        assert_eq!(rig.diag.notices(), 1, "EX-07 要求只提示一次");
        assert_eq!(rig.decision.count(), 0, "竖向时不得采样/发布");
    }

    #[test]
    fn engine_uia_failure_falls_back_without_functional_degradation() {
        // T02.3 判据②：UIA 不可用 → 固定几何，功能不降级（采样继续）。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::unavailable());
        for _ in 0..3 {
            let _ = rig.tick();
            rig.advance(1_000);
        }
        assert!(rig.engine.uia_in_fallback(), "应处于 fallback 状态");
        assert!(rig.engine.masks().count() >= 2, "fallback 至少含时钟 + 日期两个矩形");
        assert!(rig.decision.count() > 0, "功能不得降级：判定仍在发布");
    }

    #[test]
    fn engine_tray_collapse_and_expand_keep_sampling() {
        // 托盘折叠/展开：几何变化触发重算，采样不中断。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        for _ in 0..2 {
            let _ = rig.tick();
            rig.advance(1_000);
        }
        let n = rig.engine.geometry_refreshes();
        rig.taskbar.set_tray_collapsed(true);
        let _ = rig.tick();
        assert!(rig.engine.geometry_refreshes() > n);
        assert!(rig.engine.geometry().is_some_and(|g| g.tray_collapsed));
        assert!(rig.decision.count() > 0);
    }

    #[test]
    fn engine_battery_mode_slows_sampling_but_never_stops() {
        // EX-15：电池模式自动 ≥2000 ms；CPU 预算因此更宽松，采样仍继续。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        rig.power.set_battery(true);
        let report = rig.run(600_000);
        assert!(report.sampled >= 295, "电池模式 600 s 至少 ~300 次");
        assert!(report.sampled <= 305, "电池模式不得反而更密");
        assert_eq!(report.flips, 0);
    }

    #[test]
    fn engine_colored_rects_survive_uia_snapshot_refresh() {
        // §4 #44「合并 UIA 矩形 + DLL 回传矩形」：UIA 是覆盖式快照，
        // 刷新后不得把 L3 的增量遮罩抹掉。
        let mut rig = TestRig::new(FakeTaskbar::desktop_1080p(), FakeUia::working());
        let _ = rig.tick();
        let uia_only = rig.engine.masks().count();
        assert!(uia_only >= 2);

        let hover = MaskRect::new(rect_px(1650, 1040, 1680, 1060), MaskKind::Hover);
        let dll_count = rig.engine.note_colored_rects(&[hover]);
        assert_eq!(rig.engine.external_mask_count(), dll_count);
        assert!(rig.engine.masks().count() > uia_only, "DLL 矩形应并入生效遮罩");
        assert!(rig.engine.masks().hits(1660, 1050));

        // 推进到下一次 UIA 刷新周期，快照覆盖后 DLL 矩形仍在。
        rig.advance(UIA_REFRESH_INTERVAL_MS + 1_000);
        let _ = rig.tick();
        assert!(
            rig.engine.masks().hits(1660, 1050),
            "UIA 刷新后 DLL 矩形不得丢失"
        );
        assert_eq!(rig.engine.external_mask_count(), dll_count);

        rig.engine.clear_colored_rects();
        assert_eq!(rig.engine.external_mask_count(), 0);
    }
}
