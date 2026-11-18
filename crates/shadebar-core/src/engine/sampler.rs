//! 采样算法与 T1 闭环状态机（T02.2 / T02.4，架构 §8.2 / §4 #45）。
//!
//! # 为什么是「可驱动状态机」而不是 `std::thread::spawn`
//!
//! `lib.rs` 规定 core **无窗口、无线程**；§3.1 规定 T1 由主进程创建、用
//! `CreateWaitableTimerW` + `WaitForMultipleObjects` 驱动。因此这里实现
//! [`SamplerLoop::tick`] / [`Engine::run`] 的同步循环，把「时钟推进」「等待」
//! 「停止」全部抽成 trait（[`Clock`] / [`Sleeper`] / [`StopToken`]），
//! 由 T04 在 T1 线程里调用。单测用假时钟可在毫秒内跑完 600 s 闭环（A-03）。
//!
//! # 判定口径（§13-1 已拍板，禁止改成 trimmed mean）
//!
//! 代表色取直方图 **P50 桶的 RGB 均值**；P15/P85 只用于 `outlier_ratio` 异常帧检测。
//! 中位数实际崩溃点 50%，对 ≤20% 字符污染免疫（A-28）。
//!
//! # 零堆分配
//!
//! [`LumaHistogram`] 是栈上 4 × `[u32;256]`（4 KB），所有统计值 `Copy`；
//! 采样不产生 `Vec` / `String` / `Box`。耗时用 `std::time::Instant` 记录进
//! [`DiagSnapshot`]，不谎报「实测 ≤5 ms」——真实耗时由 T04 的 Debug 日志给出。

use std::time::Instant;

use crate::color::Rgba;
use crate::engine::{
    EngineConfig, MAX_CONSECUTIVE_BAD_FRAMES, MIN_VALID_PX, SINGLE_SAMPLE_BUDGET_US,
    SuspendReason, TaskbarGeom, rect_intersection,
};
use crate::engine::mask::MaskSet;
use crate::engine::screen::{ScreenBuffer, ScreenSource, checked_frame_size};
use crate::engine::taskbar::compute_sample_rect;
use crate::error::{SbError, SbResult};

// ---------------------------------------------------------------------------
// 注入 trait（对象安全）
// ---------------------------------------------------------------------------

/// 单调时钟（毫秒）。
pub trait Clock {
    /// 当前单调时间（毫秒）。
    fn now_ms(&self) -> u64;
}

/// 电源状态（EX-15）。
pub trait PowerSource {
    /// 是否正在使用电池。
    fn on_battery(&self) -> bool;
}

/// 配置快照源（A-10）。
pub trait ConfigSource {
    /// 读当前配置；`version` 递增表示有变更。
    fn snapshot(&self) -> EngineConfig;
}

/// 判定结果的唯一出口（§1.3）。
pub trait DecisionSink {
    /// 发布一次判定（T04 写入 `DecisionSlot` + `SetEvent`）。
    fn publish(&mut self, decision: &Judgement);
}

/// 诊断面板与用户提示出口。
pub trait DiagSink {
    /// 每拍更新诊断快照（T04 写 `SharedDiag` 原子量，只在面板可见时重绘）。
    fn update(&mut self, snap: &DiagSnapshot);

    /// 发送一次用户可见提示（EX-07 竖向任务栏）。
    fn notice(&mut self, reason: SuspendReason);
}

/// 等待到指定时刻（§3.3：禁止 `Sleep(1)` 忙等）。
pub trait Sleeper {
    /// 阻塞到 `deadline_ms`；假实现只推进假时钟，不真睡。
    fn sleep_until(&self, deadline_ms: u64);
}

/// 停止令牌。
pub trait StopToken {
    /// 是否应当退出循环。
    fn is_stopped(&self) -> bool;
}

// ---------------------------------------------------------------------------
// 统计与判定数据类型
// ---------------------------------------------------------------------------

/// 单次采样的统计量（架构 §5.1 `SampleStats`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleStats {
    /// 代表色（P50 桶的 RGB 均值，α = 255）。
    pub rep_color: Rgba,
    /// 感知亮度 `Y = 0.299R + 0.587G + 0.114B`，0..=1（FR-12）。
    pub y: f32,
    /// 15 分位灰度。
    pub p15: u8,
    /// 50 分位灰度（**代表桶，不是截尾均值**）。
    pub p50: u8,
    /// 85 分位灰度。
    pub p85: u8,
    /// 遮罩后参与统计的像素数。
    pub used_px: u32,
    /// `(<P15 + >P85) / used_px`，EX-24 异常帧检测用。
    pub outlier_ratio: f32,
}

/// 判定端（架构 §5.1：`Light` = 用亮端色，背景偏暗；`Dark` = 用暗端色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// 背景偏暗 → 使用亮端色（A-13 纯黑壁纸 `Y ≤ 0.05`）。
    Light,
    /// 背景偏亮 → 使用暗端色（A-14 纯白壁纸 `Y ≥ 0.95`）。
    Dark,
}

/// 一次判定（唯一出口）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Judgement {
    /// 判定端。
    pub side: Side,
    /// 实际下发的颜色。
    pub color: Rgba,
    /// EWMA 平滑后的亮度。
    pub y_smoothed: f32,
    /// 代表色与判定色的 WCAG 对比度。
    pub contrast: f32,
    /// 本次是否发生翻转。
    pub flipped: bool,
    /// 单调递增序号。
    pub seq: u64,
}

/// 诊断快照（T03.5 诊断面板 / §11.6 汇总行的字段来源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagSnapshot {
    /// 判定序号。
    pub seq: u64,
    /// 代表色。
    pub rep_color: Rgba,
    /// 判定色。
    pub decision_color: Rgba,
    /// `y * 1000`。
    pub y_milli: u32,
    /// `contrast * 1000`。
    pub contrast_milli: u32,
    /// 生效遮罩数。
    pub masks: u32,
    /// 有效像素数。
    pub valid_px: u32,
    /// 本拍耗时（微秒）。
    pub cost_us: u64,
    /// 耗时是否超出 [`SINGLE_SAMPLE_BUDGET_US`]。
    pub over_budget: bool,
}

/// 跳过本次采样的原因（全部是"保持上次判定"，不报错、不弹窗，§13-5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 用户手动暂停（A-17）。
    Paused,
    /// 尚未到采样时刻。
    NotDue,
    /// 采样矩形退化或完全在屏幕外（EX-24）。
    RegionUnavailable,
    /// 遮罩后有效像素 < [`MIN_VALID_PX`]。
    RegionTooSmall {
        /// 实际有效像素数。
        valid_px: u32,
    },
    /// 采样区被遮罩 100% 覆盖。
    FullyMasked,
    /// 直方图无有效像素。
    NoValidPixels,
    /// `BitBlt` 失败（原始错误码保留）。
    BitBltFailed {
        /// `GetLastError()` 原值。
        code: u32,
    },
    /// 连续异常帧达到上限（EX-24）。
    BadFrameBurst,
    /// 变化阈值过滤：本拍判定未发布，但采样成功。
    BelowChangeThreshold,
}

/// 单次采样结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleOutcome {
    /// 采样成功。
    Sampled(SampleStats),
    /// 跳过（保持上次判定）。
    Skipped(SkipReason),
}

/// 一拍的结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickOutcome {
    /// 采样成功且发布了新判定。
    Published(Judgement),
    /// 采样成功，但变化阈值过滤，未发布。
    Sampled(SampleStats),
    /// 跳过本拍。
    Skipped(SkipReason),
    /// 环境不满足，采样挂起。
    Suspended(SuspendReason),
    /// 未到时刻。
    NotDue,
    /// 引擎已停止。
    Stopped,
}

// ---------------------------------------------------------------------------
// 颜色数学（§11.5：perceptual_y 与 WCAG 相对亮度严格分开，不可混用）
// ---------------------------------------------------------------------------

/// 感知亮度 `Y = 0.299R + 0.587G + 0.114B`（gamma 编码 sRGB 输入，输出 0..=1）。
#[must_use]
pub fn perceptual_y(c: Rgba) -> f32 {
    (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0
}

/// sRGB 分量线性化（WCAG），输入 0..=1。
#[must_use]
pub fn linearize(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 相对亮度（先线性化）。
#[must_use]
pub fn rel_luminance(c: Rgba) -> f32 {
    0.2126 * linearize(f32::from(c.r) / 255.0)
        + 0.7152 * linearize(f32::from(c.g) / 255.0)
        + 0.0722 * linearize(f32::from(c.b) / 255.0)
}

/// WCAG 对比度 `(L1+0.05)/(L2+0.05)`，`L1` 为较亮者。
#[must_use]
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (rel_luminance(a), rel_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

// ---------------------------------------------------------------------------
// 灰度直方图（栈上 4 KB，零堆分配）
// ---------------------------------------------------------------------------

/// 256 桶灰度直方图 + RGB 累加（§11.5：`(77R + 150G + 29B) >> 8`）。
pub struct LumaHistogram {
    bins: [u32; 256],
    sum_r: [u32; 256],
    sum_g: [u32; 256],
    sum_b: [u32; 256],
    used_px: u32,
    skipped_px: u32,
}

impl Default for LumaHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LumaHistogram {
    /// 空直方图（常量构造，编译期零初始化）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bins: [0; 256],
            sum_r: [0; 256],
            sum_g: [0; 256],
            sum_b: [0; 256],
            used_px: 0,
            skipped_px: 0,
        }
    }

    /// 复位（不释放任何内存，热路径零分配）。
    pub fn reset(&mut self) {
        self.bins.fill(0);
        self.sum_r.fill(0);
        self.sum_g.fill(0);
        self.sum_b.fill(0);
        self.used_px = 0;
        self.skipped_px = 0;
    }

    /// 累加一行 BGRA8 像素。
    ///
    /// `x0` / `y` 是该行在**屏幕绝对坐标系**中的位置，`masks.hits(x, y)` 命中即跳过。
    /// 单遍完成：不分两次扫描、不构造中间数组。
    pub fn add_bgra_row(&mut self, row: &[u8], x0: i32, y: i32, masks: &MaskSet) {
        // `as_chunks::<4>()` 把整行切成 BGRA 像素；尾部不足 4 字节的残片直接丢弃
        // （抓屏缓冲按 `width*4` 分配，正常情况下残片恒为空）。
        let (pixels, _tail) = row.as_chunks::<4>();
        for (i, px) in pixels.iter().enumerate() {
            let x = x0.saturating_add(i as i32);
            if masks.hits(x, y) {
                self.skipped_px = self.skipped_px.saturating_add(1);
                continue;
            }
            let (b, g, r) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
            // §11.5：BT.601 integer，与 perceptual_y 同一系数体系。
            let luma = ((77 * r + 150 * g + 29 * b) >> 8) as usize;
            if luma >= 256 {
                continue;
            }
            self.bins[luma] = self.bins[luma].saturating_add(1);
            self.sum_r[luma] = self.sum_r[luma].saturating_add(r);
            self.sum_g[luma] = self.sum_g[luma].saturating_add(g);
            self.sum_b[luma] = self.sum_b[luma].saturating_add(b);
            self.used_px = self.used_px.saturating_add(1);
        }
    }

    /// 参与统计的像素数。
    #[must_use]
    pub const fn used_px(&self) -> u32 {
        self.used_px
    }

    /// 被遮罩跳过的像素数。
    #[must_use]
    pub const fn skipped_px(&self) -> u32 {
        self.skipped_px
    }

    /// 分位数（0..=100），累积到 `ceil(used_px * p / 100)` 的第一个桶。
    #[must_use]
    pub fn percentile(&self, p: u32) -> u8 {
        if self.used_px == 0 {
            return 0;
        }
        let p = p.min(100);
        let target = (u64::from(self.used_px) * u64::from(p)).div_ceil(100) as u32;
        let mut acc = 0u32;
        for (i, &n) in self.bins.iter().enumerate() {
            acc = acc.saturating_add(n);
            if acc >= target.max(1) {
                return i as u8;
            }
        }
        255
    }

    /// 汇总为 [`SampleStats`]；无有效像素返回 `None`（EX-17/EX-24 走"保持上次判定"）。
    #[must_use]
    pub fn finish(&self) -> Option<SampleStats> {
        if self.used_px == 0 {
            return None;
        }
        let p15 = self.percentile(15);
        let p50 = self.percentile(50);
        let p85 = self.percentile(85);

        // P50 桶一般非空；极端离散分布下退到最近的非空桶，绝不返回 None 让判定断链。
        let bucket = self.nearest_nonempty(p50 as usize);
        let n = self.bins[bucket].max(1);
        let half = n / 2;
        let r = ((self.sum_r[bucket] + half) / n).min(255) as u8;
        let g = ((self.sum_g[bucket] + half) / n).min(255) as u8;
        let b = ((self.sum_b[bucket] + half) / n).min(255) as u8;
        let rep_color = Rgba::opaque(r, g, b);

        let mut low = 0u32;
        for &n in &self.bins[..p15 as usize] {
            low = low.saturating_add(n);
        }
        let mut high = 0u32;
        let start = (p85 as usize + 1).min(256);
        for &n in &self.bins[start..] {
            high = high.saturating_add(n);
        }
        let outlier_ratio = (low.saturating_add(high)) as f32 / self.used_px as f32;

        Some(SampleStats {
            rep_color,
            y: perceptual_y(rep_color),
            p15,
            p50,
            p85,
            used_px: self.used_px,
            outlier_ratio,
        })
    }

    fn nearest_nonempty(&self, start: usize) -> usize {
        let s = start.min(255);
        if self.bins[s] > 0 {
            return s;
        }
        let mut d = 1usize;
        while d < 256 {
            if s >= d && self.bins[s - d] > 0 {
                return s - d;
            }
            if s + d < 256 && self.bins[s + d] > 0 {
                return s + d;
            }
            d += 1;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// 判定器（EWMA + 双阈值迟滞 + 变化触发）
// ---------------------------------------------------------------------------

/// 判定状态机（架构 §5.1 `Judge`）。
///
/// 恒定输入下 `y_smoothed` 收敛后不再触发变化阈值，`flip_count` 恒为 0
/// （A-13 / A-14 / A-28 的 60 s 零翻转判据）。
pub struct Judge {
    ewma_y: f32,
    last_emit_y: f32,
    current: Option<Side>,
    flip_count: u64,
    stable_frames: u32,
    seq: u64,
    initialized: bool,
}

impl Default for Judge {
    fn default() -> Self {
        Self::new()
    }
}

impl Judge {
    /// 新建。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ewma_y: 0.0,
            last_emit_y: 0.0,
            current: None,
            flip_count: 0,
            stable_frames: 0,
            seq: 0,
            initialized: false,
        }
    }

    /// 累计翻转次数。
    #[must_use]
    pub const fn flip_count(&self) -> u64 {
        self.flip_count
    }

    /// 连续稳定帧数。
    #[must_use]
    pub const fn stable_frames(&self) -> u32 {
        self.stable_frames
    }

    /// 复位（配置大幅变更 / 引擎重启时用）。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 喂入一次统计量，返回是否应发布新判定。
    pub fn update(&mut self, stats: &SampleStats, cfg: &EngineConfig) -> Option<Judgement> {
        let j = &cfg.judge;
        let ewma = if self.initialized {
            j.ewma_alpha * stats.y + (1.0 - j.ewma_alpha) * self.ewma_y
        } else {
            stats.y
        };

        // 双阈值迟滞：中间带保持上一次的端，杜绝阈值附近来回翻转。
        let hold = self.current.unwrap_or(if ewma < j.threshold_y {
            Side::Light
        } else {
            Side::Dark
        });
        let target = if ewma <= j.threshold_y - j.hysteresis {
            Side::Light
        } else if ewma >= j.threshold_y + j.hysteresis {
            Side::Dark
        } else {
            hold
        };

        let flipped = self.current.is_some_and(|c| c != target);
        if flipped {
            self.flip_count = self.flip_count.saturating_add(1);
        }
        let delta = (ewma - self.last_emit_y).abs() * 255.0;
        let emit = !self.initialized
            || flipped
            || self.current != Some(target)
            || delta >= f32::from(j.change_threshold);

        self.ewma_y = ewma;
        self.last_emit_y = if emit { ewma } else { self.last_emit_y };
        self.current = Some(target);
        self.initialized = true;
        self.stable_frames = self.stable_frames.saturating_add(1);

        if !emit {
            return None;
        }
        self.seq = self.seq.saturating_add(1);
        let color = match target {
            Side::Light => j.light_color,
            Side::Dark => j.dark_color,
        };
        Some(Judgement {
            side: target,
            color,
            y_smoothed: ewma,
            contrast: contrast_ratio(stats.rep_color, color),
            flipped,
            seq: self.seq,
        })
    }
}

// ---------------------------------------------------------------------------
// 单次采样（架构 §6.2 `sample_once` 的注入式形态）
// ---------------------------------------------------------------------------

/// 单次采样：矩形 → 裁剪 → 有效像素检查 → 抓屏 → 单遍直方图 → 统计量。
///
/// 与 §6.2 的偏离：把 `ScreenSampler` 拆成 `ScreenSource + ScreenBuffer` 两个注入项，
/// 使几何不变时的 600 次采样可被 GlobalAlloc 计数器证明零堆分配。
///
/// # Errors
///
/// 只有抓屏失败会返回 `Err(SbError::BitBltFailed)`（EX-24 明确错误、不崩）；
/// 「采样区过小 / 被全遮罩覆盖」按 §13-5 返回 `Ok(Skipped(..))`，**不报错**。
pub fn sample_once(
    screen: &mut dyn ScreenSource,
    buffer: &mut ScreenBuffer,
    hist: &mut LumaHistogram,
    masks: &MaskSet,
    geom: &TaskbarGeom,
    cfg: &EngineConfig,
) -> SbResult<SampleOutcome> {
    let Some(rect) = compute_sample_rect(geom, &cfg.region) else {
        return Ok(SampleOutcome::Skipped(SkipReason::RegionUnavailable));
    };
    // NFR-07：只对含任务栏那块屏采样；完全在屏外则跳过。
    let Some(clipped) = rect_intersection(&rect, &geom.screen) else {
        return Ok(SampleOutcome::Skipped(SkipReason::RegionUnavailable));
    };
    let valid_px = masks.valid_px(&clipped);
    if valid_px == 0 {
        return Ok(SampleOutcome::Skipped(SkipReason::FullyMasked));
    }
    if valid_px < MIN_VALID_PX {
        return Ok(SampleOutcome::Skipped(SkipReason::RegionTooSmall { valid_px }));
    }
    let (_, h) = match checked_frame_size(&clipped) {
        Some(s) => s,
        None => return Ok(SampleOutcome::Skipped(SkipReason::RegionUnavailable)),
    };

    screen.grab(&clipped, buffer)?;

    hist.reset();
    for y in 0..h {
        let Some(row) = buffer.row(y) else {
            break;
        };
        let screen_y = clipped.top.saturating_add(y as i32);
        hist.add_bgra_row(row, clipped.left, screen_y, masks);
    }
    match hist.finish() {
        Some(stats) => Ok(SampleOutcome::Sampled(stats)),
        None => Ok(SampleOutcome::Skipped(SkipReason::NoValidPixels)),
    }
}

// ---------------------------------------------------------------------------
// T1 闭环状态机
// ---------------------------------------------------------------------------

/// 一拍所需的全部上下文（由 `Engine::tick` 构造，全部是借用）。
pub struct SampleCtx<'a> {
    /// 抓屏源。
    pub screen: &'a mut dyn ScreenSource,
    /// 复用像素缓冲。
    pub buffer: &'a mut ScreenBuffer,
    /// 复用直方图。
    pub hist: &'a mut LumaHistogram,
    /// 判定器。
    pub judge: &'a mut Judge,
    /// 当前遮罩。
    pub masks: &'a MaskSet,
    /// 当前几何。
    pub geom: &'a TaskbarGeom,
    /// 当前配置。
    pub cfg: &'a EngineConfig,
    /// 判定出口。
    pub decision: &'a mut dyn DecisionSink,
    /// 诊断出口。
    pub diag: &'a mut dyn DiagSink,
    /// 当前时刻（毫秒）。
    pub now_ms: u64,
    /// 是否电池供电。
    pub on_battery: bool,
}

/// 5 分钟汇总行的结构化字段（§11.6 / §13-B：字段可增不可删）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryFields {
    /// 窗口秒数。
    pub win_s: u64,
    /// 成功次数。
    pub ok: u64,
    /// 跳过次数。
    pub skip: u64,
    /// 平均耗时（微秒）。
    pub cost_avg_us: u64,
    /// P95 耗时（微秒）。
    pub cost_p95_us: u64,
    /// 当前 `y * 1000`。
    pub y_milli: u32,
    /// 窗口内翻转次数。
    pub flips: u64,
    /// 当前遮罩数。
    pub mask: u32,
}

/// 循环累计报告（架构 §4 #45 的"异常帧计数" + §11.6 汇总行）。
#[derive(Default)]
pub struct LoopReport {
    ok: u64,
    skip: u64,
    bad: u64,
    flips_at_window_start: u64,
    flips: u64,
    mask: u32,
    y_milli: u32,
    cost_sum_us: u64,
    cost_count: u64,
    /// 16 个耗时桶，每桶 500 µs，最后一桶为溢出。
    cost_hist: [u32; 16],
    published: u64,
}

impl LoopReport {
    /// 成功次数。
    #[must_use]
    pub const fn ok(&self) -> u64 {
        self.ok
    }

    /// 跳过次数。
    #[must_use]
    pub const fn skip(&self) -> u64 {
        self.skip
    }

    /// 异常帧次数。
    #[must_use]
    pub const fn bad_frames(&self) -> u64 {
        self.bad
    }

    /// 判定发布次数。
    #[must_use]
    pub const fn published(&self) -> u64 {
        self.published
    }

    /// 汇总字段（字段名与 §11.6 示例一致）。
    #[must_use]
    pub fn summary_fields(&self, win_s: u64) -> SummaryFields {
        let avg = self
            .cost_sum_us
            .checked_div(self.cost_count)
            .unwrap_or_default();
        SummaryFields {
            win_s,
            ok: self.ok,
            skip: self.skip,
            cost_avg_us: avg,
            cost_p95_us: self.cost_p95_us(),
            y_milli: self.y_milli,
            flips: self.flips.saturating_sub(self.flips_at_window_start),
            mask: self.mask,
        }
    }

    /// 与 §11.6 示例格式一致的汇总行（冷路径，每 5 分钟一次，允许分配）。
    #[must_use]
    pub fn summary_line(&self, win_s: u64) -> String {
        let f = self.summary_fields(win_s);
        format!(
            "sample: summary win={}s ok={} skip={} cost_avg_us={} cost_p95_us={} y={:.3} flips={} mask={}",
            f.win_s,
            f.ok,
            f.skip,
            f.cost_avg_us,
            f.cost_p95_us,
            (f.y_milli as f32) / 1000.0,
            f.flips,
            f.mask
        )
    }

    /// 由 16 桶直方图估算 P95（微秒）。
    fn cost_p95_us(&self) -> u64 {
        if self.cost_count == 0 {
            return 0;
        }
        let target = (self.cost_count * 95).div_ceil(100).max(1) as u32;
        let mut acc = 0u32;
        for (i, &n) in self.cost_hist.iter().enumerate() {
            acc = acc.saturating_add(n);
            if acc >= target {
                return ((i as u64) + 1) * 500;
            }
        }
        8_000
    }

    fn note_cost(&mut self, cost_us: u64) {
        self.cost_sum_us = self.cost_sum_us.saturating_add(cost_us);
        self.cost_count = self.cost_count.saturating_add(1);
        let bucket = (cost_us / 500).min(15) as usize;
        self.cost_hist[bucket] = self.cost_hist[bucket].saturating_add(1);
    }

    fn note_success(&mut self) {
        self.ok = self.ok.saturating_add(1);
    }

    fn note_skip(&mut self) {
        self.skip = self.skip.saturating_add(1);
    }

    fn note_bad(&mut self) {
        self.bad = self.bad.saturating_add(1);
    }

    fn note_publish(&mut self, j: &Judgement) {
        self.published = self.published.saturating_add(1);
        self.y_milli = (j.y_smoothed * 1000.0).clamp(0.0, 1000.0) as u32;
        if j.flipped {
            self.flips = self.flips.saturating_add(1);
        }
    }

    fn note_masks(&mut self, n: usize) {
        self.mask = n.min(u32::MAX as usize) as u32;
    }
}

/// T1 采样闭环状态机（不持有线程，由 T04 驱动）。
pub struct SamplerLoop {
    next_due: u64,
    bad_frames: u32,
    started: bool,
    last_judgement: Option<Judgement>,
    report: LoopReport,
}

impl Default for SamplerLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl SamplerLoop {
    /// 新建。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_due: 0,
            bad_frames: 0,
            started: false,
            last_judgement: None,
            report: LoopReport {
                ok: 0,
                skip: 0,
                bad: 0,
                flips_at_window_start: 0,
                flips: 0,
                mask: 0,
                y_milli: 0,
                cost_sum_us: 0,
                cost_count: 0,
                cost_hist: [0; 16],
                published: 0,
            },
        }
    }

    /// 下一拍时刻。
    #[must_use]
    pub const fn next_due_ms(&self) -> u64 {
        self.next_due
    }

    /// 强制下一拍立即到期（配置变更 / 恢复暂停 / DPI 变化 / 几何刷新）。
    pub fn force_due(&mut self) {
        self.next_due = 0;
    }

    /// 最近一次发布的判定（异常帧期间用于"保持上次判定色"）。
    #[must_use]
    pub const fn last_judgement(&self) -> Option<&Judgement> {
        self.last_judgement.as_ref()
    }

    /// 循环累计报告。
    #[must_use]
    pub const fn report(&self) -> &LoopReport {
        &self.report
    }

    /// 驱动一拍。
    ///
    /// 顺序：到期检查 → `sample_once` → `Judge::update` → `DecisionSink::publish`
    /// → `DiagSink::update`。异常帧（`BitBlt` 失败 / 超时）连续
    /// [`MAX_CONSECUTIVE_BAD_FRAMES`] 次后进入 `BadFrameBurst`，期间**保持上次判定**
    /// 不发新 Decision（EX-24）。
    pub fn tick(&mut self, ctx: &mut SampleCtx<'_>) -> TickOutcome {
        let interval = u64::from(ctx.cfg.effective_interval_ms(ctx.on_battery));
        if !self.started {
            self.started = true;
            self.next_due = ctx.now_ms;
        }
        if ctx.now_ms < self.next_due {
            return TickOutcome::NotDue;
        }
        self.next_due = ctx.now_ms.saturating_add(interval);

        let start = Instant::now();
        let result = sample_once(
            ctx.screen,
            ctx.buffer,
            ctx.hist,
            ctx.masks,
            ctx.geom,
            ctx.cfg,
        );
        let cost_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.report.note_cost(cost_us);
        self.report.note_masks(ctx.masks.count());

        let outcome = match result {
            Err(SbError::BitBltFailed { code }) => {
                self.bad_frames = self.bad_frames.saturating_add(1);
                self.report.note_bad();
                if self.bad_frames >= MAX_CONSECUTIVE_BAD_FRAMES {
                    TickOutcome::Skipped(SkipReason::BadFrameBurst)
                } else {
                    TickOutcome::Skipped(SkipReason::BitBltFailed { code })
                }
            }
            Err(_) => {
                self.bad_frames = self.bad_frames.saturating_add(1);
                self.report.note_bad();
                TickOutcome::Skipped(SkipReason::BitBltFailed { code: 0 })
            }
            Ok(SampleOutcome::Skipped(reason)) => {
                self.report.note_skip();
                TickOutcome::Skipped(reason)
            }
            Ok(SampleOutcome::Sampled(stats)) => {
                self.bad_frames = 0;
                self.report.note_success();
                match ctx.judge.update(&stats, ctx.cfg) {
                    Some(j) => {
                        ctx.decision.publish(&j);
                        self.report.note_publish(&j);
                        self.last_judgement = Some(j);
                        TickOutcome::Published(j)
                    }
                    None => TickOutcome::Sampled(stats),
                }
            }
        };

        let snap = DiagSnapshot {
            seq: self.last_judgement.map_or(0, |j| j.seq),
            rep_color: match outcome {
                TickOutcome::Sampled(s) => s.rep_color,
                _ => self.last_judgement.map_or(Rgba::opaque(0, 0, 0), |j| j.color),
            },
            decision_color: self.last_judgement.map_or(Rgba::opaque(0, 0, 0), |j| j.color),
            y_milli: self.report.summary_fields(0).y_milli,
            contrast_milli: self
                .last_judgement
                .map_or(0, |j| (j.contrast * 1000.0).min(u32::MAX as f32) as u32),
            masks: ctx.masks.count().min(u32::MAX as usize) as u32,
            valid_px: ctx.hist.used_px(),
            cost_us,
            over_budget: cost_us > SINGLE_SAMPLE_BUDGET_US,
        };
        ctx.diag.update(&snap);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{
        FakeClock, FakeConfig, FakeDecision, FakeDiag, FakePower, FakeScreen, FakeTaskbar,
        FakeUia, TestSleeper, TestStop, alloc_count, reset_alloc_count,
    };
    use crate::engine::mask::MaskRect;
    use crate::engine::taskbar::locate;
    use crate::engine::{Engine, EngineDeps};
    use std::sync::Arc;

    /// 单模块级最小上下文（不经过 `Engine`）的注入依赖集合。
    ///
    /// 用结构体字面量代替 10 参数函数：既避免 `clippy::too_many_arguments`，
    /// 也让每个依赖的对应关系在调用点一目了然（与 `SampleCtx` 字段一一对应）。
    struct BareCtxParts<'a> {
        screen: &'a mut FakeScreen,
        buffer: &'a mut ScreenBuffer,
        hist: &'a mut LumaHistogram,
        judge: &'a mut Judge,
        masks: &'a MaskSet,
        geom: &'a TaskbarGeom,
        cfg: &'a EngineConfig,
        decision: &'a mut FakeDecision,
        diag: &'a mut FakeDiag,
        now_ms: u64,
    }

    impl<'a> BareCtxParts<'a> {
        fn build(self) -> SampleCtx<'a> {
            SampleCtx {
                screen: self.screen,
                buffer: self.buffer,
                hist: self.hist,
                judge: self.judge,
                masks: self.masks,
                geom: self.geom,
                cfg: self.cfg,
                decision: self.decision,
                diag: self.diag,
                now_ms: self.now_ms,
                on_battery: false,
            }
        }
    }

    #[test]
    fn perceptual_y_endpoints_match_a13_a14() {
        assert!(perceptual_y(Rgba::opaque(0, 0, 0)) <= 0.05);
        assert!(perceptual_y(Rgba::opaque(255, 255, 255)) >= 0.95);
        assert!((perceptual_y(Rgba::opaque(128, 128, 128)) - 128.0 / 255.0).abs() < 1e-5);
    }

    #[test]
    fn wcag_contrast_black_white_is_21() {
        let c = contrast_ratio(Rgba::opaque(0, 0, 0), Rgba::opaque(255, 255, 255));
        assert!((c - 21.0).abs() < 1e-3, "WCAG 黑白对比度应为 21:1，实际 {c}");
        assert!((contrast_ratio(Rgba::opaque(0, 0, 0), Rgba::opaque(0, 0, 0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn histogram_p50_is_robust_to_20_percent_white_noise() {
        // A-28：20% 白像素污染下代表色偏移 ≤ 2/255。
        let mut buf = ScreenBuffer::new();
        buf.ensure(300, 48).expect("ensure");
        buf.fill_black_with_white_noise(20);
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        for y in 0..48 {
            let row = buf.row(y).expect("row");
            hist.add_bgra_row(row, 0, y as i32, &masks);
        }
        let stats = hist.finish().expect("统计量");
        assert_eq!(stats.p50, 0, "P50 必须仍落在黑桶");
        assert!(stats.rep_color.r <= 2, "代表色偏移 ≤2/255：{:?}", stats.rep_color);
        assert!(stats.y <= 0.05, "A-28：Y 仍 ≤0.05");
        assert!(stats.outlier_ratio <= 0.25, "异常比例应约 20%");
    }

    #[test]
    fn histogram_skips_masked_pixels() {
        let mut buf = ScreenBuffer::new();
        buf.ensure(10, 4).expect("ensure");
        buf.fill_uniform(255, 255, 255);
        let mut masks = MaskSet::new();
        masks.union_with(&[MaskRect::new(
            crate::engine::rect_px(0, 0, 10, 4),
            crate::engine::MaskKind::Clock,
        )]);
        let mut hist = LumaHistogram::new();
        for y in 0..4 {
            hist.add_bgra_row(buf.row(y).expect("row"), 0, y as i32, &masks);
        }
        assert_eq!(hist.used_px(), 0);
        assert!(hist.skipped_px() > 0);
        assert!(hist.finish().is_none(), "全遮罩覆盖时不得产出统计量");
    }

    #[test]
    fn judge_pure_black_and_pure_white_endpoints_do_not_flip() {
        let cfg = EngineConfig::default_v1();
        let mut judge = Judge::new();
        let black = SampleStats {
            rep_color: Rgba::opaque(0, 0, 0),
            y: 0.0,
            p15: 0,
            p50: 0,
            p85: 0,
            used_px: 14_400,
            outlier_ratio: 0.0,
        };
        let mut last: Option<Judgement> = None;
        for _ in 0..600 {
            if let Some(j) = judge.update(&black, &cfg) {
                last = Some(j);
            }
        }
        assert_eq!(last.map(|j| j.side), Some(Side::Light), "A-13 亮端色");
        assert_eq!(judge.flip_count(), 0, "60 s 翻转 0 次");
        assert_eq!(judge.stable_frames(), 600);

        let mut judge2 = Judge::new();
        let white = SampleStats {
            rep_color: Rgba::opaque(255, 255, 255),
            y: 1.0,
            p15: 255,
            p50: 255,
            p85: 255,
            used_px: 14_400,
            outlier_ratio: 0.0,
        };
        let mut last2: Option<Judgement> = None;
        for _ in 0..600 {
            if let Some(j) = judge2.update(&white, &cfg) {
                last2 = Some(j);
            }
        }
        assert_eq!(last2.map(|j| j.side), Some(Side::Dark), "A-14 暗端色");
        assert_eq!(judge2.flip_count(), 0);
    }

    #[test]
    fn hysteresis_prevents_threshold_chatter() {
        let cfg = EngineConfig::default_v1();
        let mut judge = Judge::new();
        // 在迟滞带内来回抖动 ±0.03，不应翻转。
        let mk = |y: f32| SampleStats {
            rep_color: Rgba::opaque((y * 255.0) as u8, (y * 255.0) as u8, (y * 255.0) as u8),
            y,
            p15: 0,
            p50: (y * 255.0) as u8,
            p85: 0,
            used_px: 14_400,
            outlier_ratio: 0.0,
        };
        for i in 0..600 {
            let y = if i % 2 == 0 { 0.48 } else { 0.52 };
            let _ = judge.update(&mk(y), &cfg);
        }
        assert_eq!(judge.flip_count(), 0, "迟滞带内不得翻转");
    }

    #[test]
    fn a28_character_pollution_does_not_flip_judgement() {
        // A-28：纯黑 + 20% 白字符，连续 60 s 不翻转。
        let cfg = EngineConfig::default_v1();
        let mut judge = Judge::new();
        let mut buf = ScreenBuffer::new();
        buf.ensure(300, 48).expect("ensure");
        buf.fill_black_with_white_noise(20);
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        for y in 0..48 {
            hist.add_bgra_row(buf.row(y).expect("row"), 0, y as i32, &masks);
        }
        let stats = hist.finish().expect("统计量");
        for _ in 0..600 {
            let _ = judge.update(&stats, &cfg);
        }
        assert_eq!(judge.flip_count(), 0);
        assert!(stats.y <= 0.05);
    }

    #[test]
    fn sample_once_is_allocation_free_for_600_frames() {
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let cfg = EngineConfig::default_v1();
        let mut screen = FakeScreen::uniform(0, 0, 0);
        let mut buf = ScreenBuffer::new();
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        // 预热：确保缓冲已分配。
        let _ = sample_once(&mut screen, &mut buf, &mut hist, &masks, &geom, &cfg);

        reset_alloc_count();
        let mut ok = 0u64;
        for _ in 0..600 {
            if let Ok(SampleOutcome::Sampled(_)) =
                sample_once(&mut screen, &mut buf, &mut hist, &masks, &geom, &cfg)
            {
                ok += 1;
            }
        }
        assert_eq!(ok, 600, "600 次应全部成功");
        assert_eq!(alloc_count(), 0, "T02.2 判据①：零堆分配");
        assert_eq!(buf.ensure_count(), 1, "DIB 只创建一次");
    }

    #[test]
    fn sample_once_skips_when_fully_masked() {
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let cfg = EngineConfig::default_v1();
        let mut screen = FakeScreen::uniform(0, 0, 0);
        let mut buf = ScreenBuffer::new();
        let mut hist = LumaHistogram::new();
        let mut masks = MaskSet::new();
        masks.union_with(&[MaskRect::new(geom.tray, crate::engine::MaskKind::Occlusion)]);
        let out = sample_once(&mut screen, &mut buf, &mut hist, &masks, &geom, &cfg)
            .expect("全遮罩不得报错");
        assert_eq!(out, SampleOutcome::Skipped(SkipReason::FullyMasked));
        assert_eq!(screen.calls(), 0, "全遮罩时不必抓屏");
    }

    #[test]
    fn sample_once_skips_small_region_per_decision_5() {
        // §13-5：valid_px < 1500 跳过、保持上次判定、不报错。
        let mut src = FakeTaskbar::desktop_1080p();
        src.set_tray_size(40, 30);
        let geom = locate(&mut src).expect("几何");
        let cfg = EngineConfig::default_v1();
        let mut screen = FakeScreen::uniform(0, 0, 0);
        let mut buf = ScreenBuffer::new();
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        let out = sample_once(&mut screen, &mut buf, &mut hist, &masks, &geom, &cfg)
            .expect("过小区域不得返回 Err");
        assert!(
            matches!(
                out,
                SampleOutcome::Skipped(SkipReason::RegionTooSmall { valid_px })
                    if valid_px < MIN_VALID_PX
            ),
            "期望 RegionTooSmall(<1500)，实际 {out:?}"
        );
    }

    #[test]
    fn bitblt_failure_propagates_and_burst_is_detected() {
        // EX-24：连续 5 帧 BitBlt 失败 → BadFrameBurst，且保持上次判定。
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let cfg = EngineConfig::default_v1();
        let mut screen = FakeScreen::failing(87);
        let mut buf = ScreenBuffer::new();
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        let mut judge = Judge::new();
        let mut decision = FakeDecision::new();
        let mut diag = FakeDiag::new();
        let mut loop_state = SamplerLoop::new();

        for i in 0..6 {
            let mut ctx = BareCtxParts {
                screen: &mut screen,
                buffer: &mut buf,
                hist: &mut hist,
                judge: &mut judge,
                masks: &masks,
                geom: &geom,
                cfg: &cfg,
                decision: &mut decision,
                diag: &mut diag,
                now_ms: i * 1_000,
            }
            .build();
            let out = loop_state.tick(&mut ctx);
            if i < 4 {
                assert_eq!(out, TickOutcome::Skipped(SkipReason::BitBltFailed { code: 87 }));
            } else {
                assert_eq!(out, TickOutcome::Skipped(SkipReason::BadFrameBurst));
            }
        }
        assert_eq!(decision.count(), 0, "异常帧期间不得发布新判定");
        assert!(loop_state.report().bad_frames() >= 5);
    }

    #[test]
    fn sampler_loop_interval_and_battery_floor() {
        let mut src = FakeTaskbar::desktop_1080p();
        let geom = locate(&mut src).expect("几何");
        let cfg = EngineConfig::default_v1();
        let mut screen = FakeScreen::uniform(0, 0, 0);
        let mut buf = ScreenBuffer::new();
        let mut hist = LumaHistogram::new();
        let masks = MaskSet::new();
        let mut judge = Judge::new();
        let mut decision = FakeDecision::new();
        let mut diag = FakeDiag::new();
        let mut loop_state = SamplerLoop::new();

        {
            let mut ctx = BareCtxParts {
                screen: &mut screen,
                buffer: &mut buf,
                hist: &mut hist,
                judge: &mut judge,
                masks: &masks,
                geom: &geom,
                cfg: &cfg,
                decision: &mut decision,
                diag: &mut diag,
                now_ms: 1_000,
            }
            .build();
            let _ = loop_state.tick(&mut ctx);
        }
        assert_eq!(loop_state.next_due_ms(), 2_000, "默认间隔 1000 ms");

        let mut battery_cfg = cfg;
        battery_cfg.battery_saver = true;
        let mut loop_b = SamplerLoop::new();
        {
            let mut ctx = BareCtxParts {
                screen: &mut screen,
                buffer: &mut buf,
                hist: &mut hist,
                judge: &mut judge,
                masks: &masks,
                geom: &geom,
                cfg: &battery_cfg,
                decision: &mut decision,
                diag: &mut diag,
                now_ms: 10_000,
            }
            .build();
            ctx.on_battery = true;
            let _ = loop_b.tick(&mut ctx);
        }
        assert_eq!(loop_b.next_due_ms(), 12_000, "EX-15：电池模式 ≥2000 ms");
    }

    #[test]
    fn summary_line_has_all_mandatory_fields() {
        // §13-B：字段名可增不可删，诊断包解析依赖。
        let mut report = LoopReport::default();
        report.note_cost(800);
        report.note_success();
        let line = report.summary_line(300);
        for field in [
            "sample: summary",
            "win=300s",
            "ok=",
            "skip=",
            "cost_avg_us=",
            "cost_p95_us=",
            "y=",
            "flips=",
            "mask=",
        ] {
            assert!(line.contains(field), "汇总行缺字段 {field}: {line}");
        }
    }

    #[test]
    fn full_closed_loop_600s_with_fake_clock() {
        // A-03 的引擎级验证（与 mod.rs 的 Engine::run 测试互补）。
        let shared = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let clock = FakeClock::new(Arc::clone(&shared));
        let mut engine = Engine::default();
        engine.start();
        let (mut taskbar, mut screen, mut uia) = (
            FakeTaskbar::desktop_1080p(),
            FakeScreen::uniform(0, 0, 0),
            FakeUia::working(),
        );
        let (power, config) = (FakePower::ac(), FakeConfig::new(cfg_default()));
        let (mut decision, mut diag) = (FakeDecision::new(), FakeDiag::new());
        let mut deps = EngineDeps {
            taskbar: &mut taskbar,
            screen: &mut screen,
            uia: &mut uia,
            power: &power,
            config: &config,
            decision: &mut decision,
            diag: &mut diag,
            clock: &clock,
        };
        let stop = TestStop::after_ms(600_000, Arc::clone(&shared));
        let sleeper = TestSleeper::new(Arc::clone(&shared));
        let report = engine.run(&mut deps, &stop, &sleeper);
        assert!(report.sampled >= 590, "A-03：{} 次", report.sampled);
        assert_eq!(report.flips, 0);
        assert_eq!(report.notices, 0, "正常环境不应提示");
    }

    fn cfg_default() -> EngineConfig {
        EngineConfig::default_v1()
    }
}
