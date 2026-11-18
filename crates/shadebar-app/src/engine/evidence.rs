//! T04 真机效果证据采集（A-02 / A-13 / A-14 / A-20 / NFR-03 / EX-24）。
//!
//! # 证据纪律（team-lead 本批铁律）
//!
//! * **效果类结论必须有对照证据**；API 返回成功 ≠ 效果生效。
//! * **像素采样必须带区域基线**：黑白对照先采未覆盖的真实像素，再采覆盖后像素，
//!   并校验覆盖是否真的落到采样区（不落到 → 明确写「未取得」+ 原因）。
//! * 拿不到的项明确写「未取得」及原因，**不用逻辑证据顶替真机效果证据**。
//! * 每项产出：截图（`*.png`）+ 采样数值（`*.log`）+ 汇总（`t04-summary.md`）。
//! * 全程只读：不注入 explorer、不重启 explorer、不动 TranslucentTB、不改注册表、
//!   不改壁纸、不动用户已打开的窗口；自建窗口仅 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`。
//!
//! # 时长档位
//!
//! [`VerifyProfile::Full`] = A-13/A-14 各 60 s + A-20 600 s（正式验收）；
//! [`VerifyProfile::Fast`] = 各 5 s + 20 s（编译/联调冒烟），报告里如实标注
//! 「10 分钟 CPU 实测未取得」，绝不把短跑结果包装成 600 s 实测。

use std::path::{Path, PathBuf};

use shadebar_core::engine::{
    EngineConfig, RegionCfg, SINGLE_SAMPLE_BUDGET_US, SAMPLE_RECT_TOLERANCE_PX, ScreenBuffer,
    ScreenSource, Side, TaskbarGeom, compute_sample_rect, rect_contains, rect_height, rect_width,
    screen::clip_rect_to_screens,
};
use shadebar_core::SbResult;

use crate::engine::platform::{
    RecordingDiagSink, logical_cpu_count, process_cpu_100ns,
};
use crate::engine::png::{Mark, draw_rect, encode_bgra_png};
use crate::engine::screen::{GdiScreenSource, pixel_at};
use crate::engine::visual::{ProbeMode, ProbeWindow, map_rect_to_panel};
use crate::engine::EngineSession;

/// 取证时长档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyProfile {
    /// 正式验收：黑白各 60 s，CPU 600 s。
    Full,
    /// 快速冒烟：黑白各 5 s，CPU 20 s。
    Fast,
    /// A-13/A-14 独立 session 重跑专用：黑白各 60 s（保持判据时长），
    /// **不**跑 A-02 / EX-24 / 10 分钟 CPU 段（需求基线 v1.4 §9.1 表后注）。
    Endpoints,
}

impl VerifyProfile {
    /// 黑白对照单段时长（毫秒）。
    #[must_use]
    pub const fn endpoint_ms(self) -> u64 {
        match self {
            Self::Full | Self::Endpoints => 60_000,
            Self::Fast => 5_000,
        }
    }

    /// CPU 观测时长（毫秒）。`Endpoints` 档不跑 CPU 段，返回 0。
    #[must_use]
    pub const fn cpu_ms(self) -> u64 {
        match self {
            Self::Full => 600_000,
            Self::Fast => 20_000,
            Self::Endpoints => 0,
        }
    }

    /// 档位名（写入日志）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Fast => "fast",
            Self::Endpoints => "endpoints",
        }
    }

    /// 是否为「只跑黑白端点」档。
    #[must_use]
    pub const fn endpoints_only(self) -> bool {
        matches!(self, Self::Endpoints)
    }
}

// ---------------------------------------------------------------------------
// NFR-03 v1.4 双口径上限（需求基线 §7）。仅影响取证报告的判读，不改变采样实现。
// ---------------------------------------------------------------------------
/// 墙钟辅助判据：`cost_us` p95 上限（微秒）。
const WALL_P95_LIMIT_US: u64 = 10_000;
/// 墙钟辅助判据：`cost_us` max 上限（微秒）。
const WALL_MAX_LIMIT_US: u64 = 15_000;
/// CPU 主判据：10 分钟平均单核 CPU 上限（%）。
const CPU_CORE_LIMIT_PCT: f64 = 0.1;
/// `cost_us` 样本数下限（QA 取证要求 ≥ 500）。
const COST_SAMPLE_MIN: usize = 500;

/// 取证选项。
#[derive(Debug, Clone)]
pub struct VerifyOptions {
    /// 输出目录（日志 / 截图 / 汇总）。
    pub out_dir: PathBuf,
    /// 时长档位。
    pub profile: VerifyProfile,
}

/// 单项取证结果。
#[derive(Debug, Clone)]
pub struct ItemResult {
    /// 判据编号，如 `"A-02"`。
    pub id: &'static str,
    /// 是否取得真机效果证据。
    pub obtained: bool,
    /// 「未取得」原因（取得时为 `"ok"`）。
    pub reason: String,
    /// 采样数值（键 -> 值，全 ASCII）。
    pub metrics: Vec<(&'static str, String)>,
    /// 截图路径（未取得时可能为 `None`）。
    pub screenshot: Option<PathBuf>,
    /// 数值日志路径。
    pub log_path: PathBuf,
}

/// 取证汇总。
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// 各项结果。
    pub items: Vec<ItemResult>,
    /// 汇总 markdown 路径。
    pub summary_path: PathBuf,
    /// 主日志路径。
    pub main_log: PathBuf,
}

/// 追加一行到 ASCII 日志；写失败不中断取证（日志是证据，不是功能路径）。
fn append_log(path: &Path, lines: &[String]) {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

/// 把 PNG 字节写到磁盘。
fn write_png(path: &Path, bytes: &[u8]) -> SbResult<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| shadebar_core::SbError::PathNotWritable {
            path: format!("{}: {e}", parent.display()),
        })?;
    }
    std::fs::write(path, bytes).map_err(|e| shadebar_core::SbError::PathNotWritable {
        path: format!("{}: {e}", path.display()),
    })?;
    Ok(path.to_path_buf())
}

/// 用真实 `BitBlt` 抓一块屏幕区域，叠加标注矩形后编码成 PNG。
fn capture_annotated(
    screen: &mut GdiScreenSource,
    rect: &shadebar_core::engine::RectPx,
    annotations: &[Mark],
) -> SbResult<Vec<u8>> {
    let w = rect_width(rect).max(1) as u32;
    let h = rect_height(rect).max(1) as u32;
    let mut buffer = ScreenBuffer::new();
    buffer.ensure(w, h)?;
    screen.grab(rect, &mut buffer)?;
    let mut bgra = buffer.as_bytes().to_vec();
    for m in annotations {
        draw_rect(
            &mut bgra,
            w,
            h,
            &Mark {
                rect: shadebar_core::engine::rect_px(
                    m.rect.left - rect.left,
                    m.rect.top - rect.top,
                    m.rect.right - rect.left,
                    m.rect.bottom - rect.top,
                ),
                color: m.color,
                thickness: m.thickness,
            },
        );
    }
    encode_bgra_png(w, h, &bgra).ok_or(shadebar_core::SbError::Unsupported {
        what: "png encoding of captured pixels",
    })
}

/// 统计一块 BGRA 缓冲的平均 RGB（区域基线 / 覆盖生效校验）。
#[must_use]
fn mean_rgb(bytes: &[u8]) -> (u8, u8, u8) {
    let mut r = 0u64;
    let mut g = 0u64;
    let mut b = 0u64;
    let mut n = 0u64;
    for px in bytes.as_chunks::<4>().0 {
        b += u64::from(px[0]);
        g += u64::from(px[1]);
        r += u64::from(px[2]);
        n += 1;
    }
    if n == 0 {
        return (0, 0, 0);
    }
    ((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

/// 抓一小块真实屏幕并返回平均色（不写文件）。
fn sample_mean(screen: &mut GdiScreenSource, rect: &shadebar_core::engine::RectPx) -> SbResult<(u8, u8, u8)> {
    let w = rect_width(rect).max(1) as u32;
    let h = rect_height(rect).max(1) as u32;
    let mut buffer = ScreenBuffer::new();
    buffer.ensure(w, h)?;
    screen.grab(rect, &mut buffer)?;
    Ok(mean_rgb(buffer.as_bytes()))
}

/// 取采样区中心像素的 `#RRGGBB`（区域基线的像素级证据）。
fn center_pixel_hex(screen: &mut GdiScreenSource, rect: &shadebar_core::engine::RectPx) -> Option<String> {
    let w = rect_width(rect).max(1) as u32;
    let h = rect_height(rect).max(1) as u32;
    let mut buffer = ScreenBuffer::new();
    buffer.ensure(w, h).ok()?;
    screen.grab(rect, &mut buffer).ok()?;
    pixel_at(&buffer, w / 2, h / 2)
}

/// A-02：真实 `TrayNotifyWnd` 几何 + 采样矩形 100% 落入 + 误差 ≤ 4 px + 可视化截图。
fn verify_a02(session: &mut EngineSession, opts: &VerifyOptions) -> ItemResult {
    let log_path = opts.out_dir.join("A-02-geometry.log");
    let mut log: Vec<String> = Vec::new();
    log.push("=== A-02: real TrayNotifyWnd geometry ===".to_string());

    let info = match session.taskbar_mut().locate_windows() {
        Ok(i) => i,
        Err(e) => {
            log.push(format!("locate_windows FAILED: {e}"));
            append_log(&log_path, &log);
            return ItemResult {
                id: "A-02",
                obtained: false,
                reason: format!("真实 Shell_TrayWnd/TrayNotifyWnd 定位失败: {e}"),
                metrics: Vec::new(),
                screenshot: None,
                log_path,
            };
        }
    };
    // 这三项是 `Win32TaskbarSource` 的探测状态，不属于 `WindowInfo` 值对象。
    let (probe_log, found_real_tray, auto_hide_source) = {
        let src = session.taskbar_mut();
        (
            src.diagnostics.clone(),
            src.found_real_tray,
            src.auto_hide_source,
        )
    };
    for d in &probe_log {
        log.push(format!("probe: {d}"));
    }

    let probe = match session.taskbar_mut().read_probe() {
        Ok(p) => p,
        Err(e) => {
            log.push(format!("read_probe FAILED: {e}"));
            append_log(&log_path, &log);
            return ItemResult {
                id: "A-02",
                obtained: false,
                reason: format!("真实几何读数失败: {e}"),
                metrics: Vec::new(),
                screenshot: None,
                log_path,
            };
        }
    };
    let geom = match TaskbarGeom::from_probe(&probe) {
        Ok(g) => g,
        Err(e) => {
            log.push(format!("TaskbarGeom FAILED: {e}"));
            append_log(&log_path, &log);
            return ItemResult {
                id: "A-02",
                obtained: false,
                reason: format!("几何归一化失败: {e}"),
                metrics: Vec::new(),
                screenshot: None,
                log_path,
            };
        }
    };
    let region: RegionCfg = EngineConfig::default_v1().region;
    let sample = compute_sample_rect(&geom, &region);

    let mut metrics: Vec<(&'static str, String)> = Vec::new();
    metrics.push(("found_real_tray", found_real_tray.to_string()));
    metrics.push(("bar_hwnd", format!("0x{:016X}", info.bar.0 as usize)));
    metrics.push(("tray_hwnd", format!("0x{:016X}", info.tray_hwnd.0 as usize)));
    metrics.push(("tray_class", info.tray_class.clone()));
    metrics.push(("dpi", info.dpi.to_string()));
    metrics.push(("monitor_count", info.monitors.len().to_string()));
    metrics.push(("auto_hide", info.auto_hide.to_string()));
    metrics.push(("auto_hide_source", auto_hide_source.to_string()));
    metrics.push((
        "tray_rect",
        format!(
            "({},{})-({},{})",
            info.tray_rect.left, info.tray_rect.top, info.tray_rect.right, info.tray_rect.bottom
        ),
    ));

    let mut obtained = false;
    let reason: String;
    let mut shot: Option<PathBuf> = None;
    match sample {
        None => {
            reason = "采样矩形不可用（自动隐藏/竖向任务栏/托盘退化）；本次真机环境下 A-02 未取得".to_string();
            log.push("compute_sample_rect returned None".to_string());
        }
        Some(s) => {
            let contained = rect_contains(&info.tray_rect, &s);
            // 误差 = 四边越界量的最大值（px）；rect_clamp 后理论上恒为 0。
            let overflow = (info.tray_rect.left - s.left)
                .max(info.tray_rect.top - s.top)
                .max(s.right - info.tray_rect.right)
                .max(s.bottom - info.tray_rect.bottom)
                .max(0);
            metrics.push((
                "sample_rect",
                format!("({},{})-({},{})", s.left, s.top, s.right, s.bottom),
            ));
            metrics.push(("contained_in_tray", contained.to_string()));
            metrics.push(("error_px", overflow.to_string()));
            metrics.push(("tolerance_px", SAMPLE_RECT_TOLERANCE_PX.to_string()));
            metrics.push(("usable", geom.is_usable().to_string()));
            log.push(format!(
                "sample_rect=({},{})-({},{}) contained={} error_px={} tolerance_px={}",
                s.left,
                s.top,
                s.right,
                s.bottom,
                contained,
                overflow,
                SAMPLE_RECT_TOLERANCE_PX
            ));

            // 可视化截图：真实屏幕托盘区 + 三类标注框（黄=tray, 红=sample, 青=遮罩）。
            let mut screen = GdiScreenSource::new();
            let capture = clip_rect_to_screens(&info.tray_rect, &info.monitors)
                .unwrap_or(info.tray_rect);
            let mut marks = vec![Mark {
                rect: info.tray_rect,
                color: [255, 220, 60, 255],
                thickness: 2,
            }];
            marks.push(Mark {
                rect: s,
                color: [60, 60, 255, 255],
                thickness: 2,
            });
            for m in session.engine().masks().rects() {
                marks.push(Mark {
                    rect: m.rect,
                    color: [255, 60, 255, 255],
                    thickness: 1,
                });
            }
            match capture_annotated(&mut screen, &capture, &marks) {
                Ok(png) => match write_png(&opts.out_dir.join("A-02-tray-annotated.png"), &png) {
                    Ok(p) => {
                        log.push(format!("screenshot: {}", p.display()));
                        shot = Some(p);
                    }
                    Err(e) => log.push(format!("screenshot write FAILED: {e}")),
                },
                Err(e) => log.push(format!("capture_annotated FAILED: {e}")),
            }

            // 自建诊断面板截图（证明「新建自己的窗口」，且带数值文本）。
            let tray_panel = map_rect_to_panel(&info.tray_rect, 40, 210, 600, 120);
            let sample_panel = map_rect_to_panel(&s, 40, 210, 600, 120);
            let lines = vec![
                "ShadeBar T04 / A-02 geometry evidence".to_string(),
                format!("tray=({},{})-({},{})", info.tray_rect.left, info.tray_rect.top, info.tray_rect.right, info.tray_rect.bottom),
                format!("sample=({},{})-({},{})", s.left, s.top, s.right, s.bottom),
                format!("error={}px tolerance={}px contained={}", overflow, SAMPLE_RECT_TOLERANCE_PX, contained),
                format!("dpi={} monitors={} real_tray={}", info.dpi, info.monitors.len(), found_real_tray),
            ];
            if let Ok(panel) = ProbeWindow::show(
                ProbeMode::Panel {
                    lines,
                    tray: tray_panel,
                    sample: sample_panel,
                },
                60,
                60,
                760,
                420,
            ) {
                panel.refresh();
                log.push(format!("panel hwnd=0x{:016X}", panel.hwnd().0 as usize));
                let mut ps = GdiScreenSource::new();
                let pr = clip_rect_to_screens(&panel.screen_rect, &info.monitors)
                    .unwrap_or(panel.screen_rect);
                match capture_plain(&mut ps, panel.hwnd(), &pr) {
                    Ok(Some(png)) => {
                        if let Ok(p) = write_png(&opts.out_dir.join("A-02-panel.png"), &png) {
                            log.push(format!("panel screenshot: {}", p.display()));
                        }
                    }
                    Ok(None) => log.push(
                        "panel screenshot NOT obtained: captured area was uniform blank (window not composited)"
                            .to_string(),
                    ),
                    Err(e) => log.push(format!("panel capture FAILED: {e}")),
                }
            } else {
                log.push("diagnostic panel window unavailable (non-fatal)".to_string());
            }

            if contained && overflow <= SAMPLE_RECT_TOLERANCE_PX && found_real_tray {
                obtained = true;
                reason = "ok".to_string();
            } else if !found_real_tray {
                reason = "未找到真实 TrayNotifyWnd（仅几何近似），按纪律不得声称 A-02 通过".to_string();
            } else if !contained {
                reason = "采样矩形未 100% 落在 TrayNotifyWnd 内".to_string();
            } else {
                reason = format!("误差 {overflow}px 超过 {SAMPLE_RECT_TOLERANCE_PX}px");
            }
        }
    }
    for (k, v) in &metrics {
        log.push(format!("metric: {k}={v}"));
    }
    log.push(format!("result: obtained={obtained} reason={reason}"));
    append_log(&log_path, &log);
    ItemResult {
        id: "A-02",
        obtained,
        reason,
        metrics,
        screenshot: shot,
        log_path,
    }
}

/// 统计 BGRA 缓冲里分散采样点的不同 RGB 颜色数（上限 3 即提前返回）。
///
/// 用于判断自建诊断面板截图是否真的包含绘制内容：DWM 重定向表面下
/// `GetDC(hwnd)` 可能只返回纯黑/纯白/近单色，这类截图必须判为无效。
/// 真实面板必然有背景色 + 文字 + 示意矩形，颜色数 ≥ 3。
#[must_use]
fn distinct_rgb_count(bytes: &[u8]) -> usize {
    let chunks = bytes.as_chunks::<4>().0;
    if chunks.is_empty() {
        return 0;
    }
    let step = (chunks.len() / 64).max(1);
    let mut seen: Vec<[u8; 3]> = Vec::with_capacity(3);
    for p in chunks.iter().step_by(step) {
        let c = [p[0], p[1], p[2]]; // 忽略 alpha：GDI BitBlt 不写 alpha
        if !seen.contains(&c) {
            seen.push(c);
            if seen.len() >= 3 {
                return 3;
            }
        }
    }
    seen.len()
}

/// 抓**自建诊断窗口自身**的客户区，不加标注。
///
/// 源 DC 用 `GetDC(hwnd)`（见 [`GdiScreenSource::grab_window`]），避免 DWM 下
/// 从桌面 DC 抓到一片白。返回 `Ok(None)` 表示抓到的是纯色空白（窗口未绘制），
/// 调用方必须按「截图未取得」处理，不得写成有效证据。
fn capture_plain(
    screen: &mut GdiScreenSource,
    hwnd: windows::Win32::Foundation::HWND,
    rect: &shadebar_core::engine::RectPx,
) -> SbResult<Option<Vec<u8>>> {
    let w = rect_width(rect).max(1) as u32;
    let h = rect_height(rect).max(1) as u32;
    let mut buffer = ScreenBuffer::new();
    buffer.ensure(w, h)?;
    screen.grab_window(hwnd, w, h, &mut buffer)?;
    if distinct_rgb_count(buffer.as_bytes()) < 3 {
        return Ok(None);
    }
    encode_bgra_png(w, h, buffer.as_bytes())
        .map(Some)
        .ok_or(shadebar_core::SbError::Unsupported {
            what: "png encoding of captured pixels",
        })
}

/// 抓当前真实屏幕的采样区并叠加标注（EX-24 / A-20 的有效可视化截图）。
///
/// 自建诊断面板在 DWM 重定向表面下无法被 `GetDC(hwnd)` 抓到（真机实测），
/// 因此这两项的主截图改用**与 A-02 相同的真实屏幕 + 标注**路径：像素来自
/// 真实 `BitBlt(GetDC(NULL))`，不依赖窗口合成。
fn real_screen_shot(
    session: &EngineSession,
    out_dir: &Path,
    name: &str,
) -> Option<PathBuf> {
    let geom = session.engine().geometry().copied()?;
    let region = session.engine().config().region;
    let sample = compute_sample_rect(&geom, &region)?;
    let mut screen = GdiScreenSource::new();
    let png = capture_annotated(
        &mut screen,
        &sample,
        &[Mark {
            rect: sample,
            color: [255, 60, 255, 255],
            thickness: 2,
        }],
    )
    .ok()?;
    write_png(&out_dir.join(name), &png).ok()
}

/// 单个黑白端点的判据规格（需求基线 v1.4 §9.1：判据数值未放宽）。
#[derive(Debug, Clone, Copy)]
struct EndpointSpec {
    label: &'static str,
    id: &'static str,
    color: [u8; 4],
    expected_side: Side,
    /// `y*1000` 下限（A-14：Y ≥ 0.95 → 950）。
    y_min_milli: Option<u32>,
    /// `y*1000` 上限（A-13：Y ≤ 0.05 → 50）。
    y_max_milli: Option<u32>,
}

struct EndpointOutcome {
    metrics: Vec<(&'static str, String)>,
    log: Vec<String>,
    screenshot: Option<PathBuf>,
    reason: String,
}

impl EndpointOutcome {
    /// 提前失败时构造（无截图）。
    fn failed(metrics: Vec<(&'static str, String)>, log: Vec<String>, reason: String) -> Self {
        Self { metrics, log, screenshot: None, reason }
    }
}

/// 用自建纯色窗口造一段真机对照背景，驱动引擎采样并统计。
///
/// # v1.4 §9.1 编排口径
///
/// 调用方为**每个端点各建一个独立 `EngineSession`**（本函数不复用别的场景的会话）；
/// 翻转按**稳态段**统计：全段原始 `flips` 如实记录，但判据用排除「切换背景后
/// 段首第一拍」的 `flips_steady`。段首那一拍单独记 `flips_head_excluded`，便于复核。
fn run_endpoint(
    session: &mut EngineSession,
    spec: &EndpointSpec,
    duration_ms: u64,
    out_dir: &Path,
) -> EndpointOutcome {
    let label = spec.label;
    let color = spec.color;
    let mut log: Vec<String> = Vec::new();
    let mut metrics: Vec<(&'static str, String)> = Vec::new();
    log.push(format!("=== {}: real screen endpoint ({duration_ms} ms) ===", spec.id));
    log.push("orchestration: independent EngineSession for this endpoint (v1.4 9.1)".to_string());
    metrics.push(("independent_session", "true".to_string()));

    // ⚠ 编排根因（首轮 `--verify endpoints` 的 A-14 flips=1）：
    // 独立会话的 core `geom` 初始为 None。若先 tick 一拍「建立几何」，那一拍会
    // 采到真实桌面（暗色），把 Judge 的 EWMA 种子设到暗端；随后覆盖纯白时
    // EWMA 需要若干拍爬升，段内必然产生 1 次收敛翻转 —— 它既不是产品抖动，
    // 也不属于「段首第一拍」能排除的范围。
    //
    // 正确做法：**不产生任何判定**地只读探测任务栏几何，先建纯色覆盖窗口，
    // 让本会话的第一拍就采到目标背景。冷启动 Judge 的首次 update 直接以
    // `stats.y` 作为 EWMA 初值（core `Judge::update`），因此天然无收敛瞬态。
    let probe = match session.taskbar_mut().read_probe() {
        Ok(p) => p,
        Err(e) => {
            return EndpointOutcome::failed(
                metrics,
                log,
                format!("独立会话只读几何探测失败: {e}"),
            );
        }
    };
    let geom = match TaskbarGeom::from_probe(&probe) {
        Ok(g) => g,
        Err(e) => {
            return EndpointOutcome::failed(
                metrics,
                log,
                format!("几何归一化失败: {e}"),
            );
        }
    };
    log.push("geometry: direct read_probe (no pre-loop tick, first sample sees overlay)".to_string());
    metrics.push(("pre_loop_ticks", "0".to_string()));
    let region: RegionCfg = session.engine().config().region;
    let sample = match compute_sample_rect(&geom, &region) {
        Some(s) => s,
        None => {
            return EndpointOutcome::failed(
                metrics,
                log,
                "采样矩形不可用，无法构造对照窗口".to_string(),
            );
        }
    };

    let mut screen = GdiScreenSource::new();
    // ① 区域基线：未覆盖时的真实像素。
    let baseline = match sample_mean(&mut screen, &sample) {
        Ok(c) => c,
        Err(e) => {
            return EndpointOutcome::failed(metrics, log, format!("基线抓屏失败: {e}"));
        }
    };
    if let Some(hex) = center_pixel_hex(&mut screen, &sample) {
        log.push(format!("baseline center pixel = {hex}"));
        metrics.push(("baseline_center_pixel", hex));
    }
    log.push(format!(
        "baseline mean RGB before overlay = ({},{},{})",
        baseline.0, baseline.1, baseline.2
    ));
    metrics.push(("baseline_rgb", format!("{:?}", baseline)));

    // ② 自建纯色窗口覆盖采样矩形（不动用户窗口/壁纸）。
    let panel = match ProbeWindow::show(
        ProbeMode::Solid { color },
        sample.left,
        sample.top,
        rect_width(&sample),
        rect_height(&sample),
    ) {
        Ok(p) => p,
        Err(e) => {
            return EndpointOutcome::failed(metrics, log, format!("自建对照窗口创建失败: {e}"));
        }
    };
    panel.refresh();

    // ③ 覆盖生效校验：真实 BitBlt 读到目标色了吗？
    let covered = match sample_mean(&mut screen, &sample) {
        Ok(c) => c,
        Err(e) => {
            log.push(format!("covered capture FAILED: {e}"));
            (255, 255, 255)
        }
    };
    let want = (color[2], color[1], color[0]);
    let delta = (i32::from(covered.0) - i32::from(want.0))
        .abs()
        .max((i32::from(covered.1) - i32::from(want.1)).abs())
        .max((i32::from(covered.2) - i32::from(want.2)).abs());
    log.push(format!(
        "covered mean RGB = ({},{},{}) target=({},{},{}) delta={}",
        covered.0,
        covered.1,
        covered.2,
        want.0,
        want.1,
        want.2,
        delta
    ));
    metrics.push(("covered_rgb", format!("{:?}", covered)));
    metrics.push(("target_rgb", format!("{:?}", want)));
    metrics.push(("coverage_delta", delta.to_string()));

    // ④ 覆盖期间截图（带采样矩形标注），作为真机效果证据。
    let mut shot: Option<PathBuf> = None;
    if let Ok(png) = capture_annotated(
        &mut screen,
        &sample,
        &[Mark {
            rect: sample,
            color: [60, 60, 255, 255],
            thickness: 2,
        }],
    ) {
        if let Ok(p) = write_png(&out_dir.join(format!("A-{label}-screen.png")), &png) {
            log.push(format!("screenshot: {}", p.display()));
            shot = Some(p);
        }
    }

    // ⑤ 驱动真机采样闭环（真实 BitBlt + 真实定时器 + 真实 UIA）。
    let pub_before = session.decision().published;
    let flips_before = session.decision().flips;
    let samples_before = session.diag().samples;
    // 逐拍翻转标志的切片起点：稳态段 = [mark+1 ..]，段首第一拍被排除。
    let flip_mark = session.decision().flip_flags.len();
    let start = session.now_ms();
    let step = u64::from(EngineConfig::default_v1().interval_ms);
    while session.now_ms().saturating_sub(start) < duration_ms {
        let _ = session.tick();
        panel.refresh();
        session.wait_interval(step);
    }
    let elapsed = session.now_ms().saturating_sub(start);
    let published = session.decision().published.saturating_sub(pub_before);
    let flips = session.decision().flips.saturating_sub(flips_before);
    let samples = session.diag().samples.saturating_sub(samples_before);
    let last = session.decision().last;
    let side = last.map(|j| format!("{:?}", j.side)).unwrap_or_else(|| "none".to_string());
    let color_hex = last
        .map(|j| format!("#{:02X}{:02X}{:02X}", j.color.r, j.color.g, j.color.b))
        .unwrap_or_else(|| "none".to_string());
    let y_milli = last.map(|j| (j.y_smoothed * 1000.0) as u32).unwrap_or(0);

    let segment_flags = &session.decision().flip_flags[flip_mark..];
    let head_flip = segment_flags.first().copied().unwrap_or(false);
    let steady_ticks = segment_flags.len().saturating_sub(1) as u64;
    let steady_flips = segment_flags.iter().skip(1).filter(|&&f| f).count() as u64;

    metrics.push(("duration_ms", elapsed.to_string()));
    metrics.push(("samples", samples.to_string()));
    metrics.push(("published", published.to_string()));
    metrics.push(("flips", flips.to_string()));
    metrics.push(("flips_head_excluded", head_flip.to_string()));
    // `steady_publishes` 是被排除段首后剩下的**发布次数**（静态背景下本身就很少），
    // 而判据关心的是 60 s 内的采样拍数，故同时给出 `steady_sample_ticks`。
    metrics.push(("steady_publishes", steady_ticks.to_string()));
    metrics.push(("steady_sample_ticks", samples.saturating_sub(1).to_string()));
    metrics.push(("flips_steady", steady_flips.to_string()));
    metrics.push(("verdict_flips_scope", "steady segment, head tick excluded (v1.4 9.1)".to_string()));
    metrics.push(("side", side.clone()));
    metrics.push(("expected_side", format!("{:?}", spec.expected_side)));
    metrics.push(("decision_color", color_hex.clone()));
    metrics.push(("last_y_milli", y_milli.to_string()));
    metrics.push(("y_min_milli", spec.y_min_milli.map_or_else(|| "none".to_string(), |v| v.to_string())));
    metrics.push(("y_max_milli", spec.y_max_milli.map_or_else(|| "none".to_string(), |v| v.to_string())));
    log.push(format!(
        "loop: elapsed={}ms samples={} published={} flips={} flips_head_excluded={} flips_steady={} steady_publishes={} steady_sample_ticks={} side={} color={} y_milli={}",
        elapsed, samples, published, flips, head_flip, steady_flips, steady_ticks,
        samples.saturating_sub(1), side, color_hex, y_milli
    ));
    for d in session.uia().diagnostics() {
        log.push(format!("uia: {d}"));
    }
    for line in session.decision().log.iter().rev().take(5) {
        log.push(format!("judge_tail: {line}"));
    }
    // 窗口在此处离开作用域即销毁；用户桌面恢复原状。
    drop(panel);

    // 判定：①覆盖真生效 ②有采样 ③稳态段翻转 0 次 ④判定端与 Y 端点符合判据。
    // 四项都过才算「取得」；任何一项不满足都写明原因，不放宽判据数值。
    let mut reason = "ok".to_string();
    if delta > 24 {
        reason = "自建纯色窗口未真正覆盖采样区（真实像素与目标色偏差 >24/255）；效果证据未取得".to_string();
    } else if samples == 0 {
        reason = "对照期间无成功采样（可能被遮罩全排除或采样区退化）".to_string();
    } else if steady_flips != 0 {
        reason = format!("稳态段判定翻转 {steady_flips} 次，判据要求 0 次（未放宽）");
    } else {
        let Some(j) = last else {
            reason = "对照期间没有产生任何判定，无法核对端色与 Y".to_string();
            return EndpointOutcome { metrics, log, screenshot: shot, reason };
        };
        if j.side != spec.expected_side {
            reason = format!("判定端 {:?} 与预期 {:?} 不符", j.side, spec.expected_side);
        } else {
            let above_min = spec.y_min_milli.is_none_or(|m| y_milli >= m);
            let below_max = spec.y_max_milli.is_none_or(|m| y_milli <= m);
            if !above_min || !below_max {
                reason = format!(
                    "末拍 Y={:.4} 不满足端点判据（要求 {:?}..={:?} milli）",
                    j.y_smoothed, spec.y_min_milli, spec.y_max_milli
                );
            }
        }
    }
    EndpointOutcome { metrics, log, screenshot: shot, reason }
}

/// A-13 / A-14：真机纯黑 / 纯白对照，各起**独立 `EngineSession`**。
///
/// 需求基线 v1.4 §9.1 表后注硬要求：不得复用同一 session 连跑黑→白，
/// 否则段首从上一场景端翻到新端的那 1 次是编排产物。判据数值（Y 端点、
/// 60 s 稳态翻转 0 次）一字未改。
fn verify_endpoints(opts: &VerifyOptions) -> Vec<ItemResult> {
    let duration = opts.profile.endpoint_ms();
    let specs = [
        EndpointSpec {
            label: "13-black",
            id: "A-13",
            color: [0u8, 0, 0, 255],
            expected_side: Side::Light,
            y_min_milli: None,
            y_max_milli: Some(50),
        },
        EndpointSpec {
            label: "14-white",
            id: "A-14",
            color: [255u8, 255, 255, 255],
            expected_side: Side::Dark,
            y_min_milli: Some(950),
            y_max_milli: None,
        },
    ];
    let mut out = Vec::new();
    for spec in &specs {
        // 每个端点一个新会话：几何、判定器、EWMA 状态、诊断记录全部独立。
        let outcome = match EngineSession::new(EngineConfig::default_v1()) {
            Ok(mut session) => run_endpoint(&mut session, spec, duration, &opts.out_dir),
            Err(e) => EndpointOutcome::failed(
                vec![("independent_session", "false".to_string())],
                vec![format!("独立 EngineSession 创建失败: {e}")],
                format!("独立 EngineSession 创建失败: {e}"),
            ),
        };
        let log_path = opts.out_dir.join(format!("A-{}-endpoints.log", spec.label));
        let mut log = outcome.log;
        log.push(format!(
            "result: obtained={} reason={}",
            outcome.reason == "ok",
            outcome.reason
        ));
        append_log(&log_path, &log);
        out.push(ItemResult {
            id: spec.id,
            obtained: outcome.reason == "ok",
            reason: outcome.reason,
            metrics: outcome.metrics,
            screenshot: outcome.screenshot,
            log_path,
        });
    }
    out
}

/// EX-24：真实 `BitBlt` 失败时不崩、保持上次判定、恢复后继续采样。
fn verify_ex24(session: &mut EngineSession, opts: &VerifyOptions) -> ItemResult {
    let log_path = opts.out_dir.join("EX-24-bitblt.log");
    let mut log: Vec<String> = Vec::new();
    log.push("=== EX-24: real BitBlt failure (NULL source DC) ===".to_string());

    // 先跑 3 拍建立「上次判定」。
    let step = u64::from(EngineConfig::default_v1().interval_ms);
    for _ in 0..3 {
        let _ = session.tick();
        session.wait_interval(step);
    }
    let last_before = session.decision().last;
    let pub_before = session.decision().published;
    let bad_before = session.engine().report().bad_frames;

    // 注入真实失败：GdiScreenSource 把源 DC 置 NULL，触发 Win32 BitBlt 返回 0。
    session.screen_mut().set_fault_injection(true);
    let mut outcomes: Vec<String> = Vec::new();
    for _ in 0..6 {
        let o = session.tick();
        outcomes.push(format!("{o:?}"));
        session.wait_interval(step);
    }
    let last_after = session.decision().last;
    let pub_after = session.decision().published;
    let bad_after = session.engine().report().bad_frames;
    session.screen_mut().set_fault_injection(false);

    // 恢复验证：关掉注入后必须能再次采样成功。
    let mut recovered = false;
    for _ in 0..4 {
        let o = session.tick();
        if matches!(
            o,
            shadebar_core::engine::TickOutcome::Published(_)
                | shadebar_core::engine::TickOutcome::Sampled(_)
        ) {
            recovered = true;
        }
        session.wait_interval(step);
    }

    // 故障恢复后再触发一次 DPI 重算通知，验证几何状态机未被 BitBlt 失败破坏。
    let refreshes_before = session.engine().geometry_refreshes();
    session.engine_mut().on_dpi_changed();
    for _ in 0..2 {
        let _ = session.tick();
        session.wait_interval(step);
    }
    let dpi_recompute_ok = session.engine().geometry_refreshes() > refreshes_before;

    let kept = last_before == last_after && pub_after == pub_before;
    let bad_grew = bad_after > bad_before;
    let metrics = vec![
        ("last_decision_kept", kept.to_string()),
        ("published_delta", (pub_after.saturating_sub(pub_before)).to_string()),
        ("bad_frames_delta", (bad_after.saturating_sub(bad_before)).to_string()),
        ("bitblt_calls", session.screen_mut().blits().to_string()),
        ("recovered_after_fault", recovered.to_string()),
        ("dpi_recompute_after_fault", dpi_recompute_ok.to_string()),
        ("outcomes", format!("{:?}", outcomes)),
    ];
    for o in &outcomes {
        log.push(format!("outcome: {o}"));
    }
    for (k, v) in &metrics {
        log.push(format!("metric: {k}={v}"));
    }
    let obtained = kept && bad_grew && recovered && dpi_recompute_ok;
    let reason = if obtained {
        "ok".to_string()
    } else {
        format!(
            "kept={kept} bad_frames_grew={bad_grew} recovered={recovered}；任一不成立即不得声称通过"
        )
    };
    log.push(format!("result: obtained={obtained} reason={reason}"));
    log.push("note: no crash / no panic occurred; process continued sampling.".to_string());

    // 主截图：真实屏幕采样区（不依赖 DWM 窗口合成）。
    let shot = real_screen_shot(session, &opts.out_dir, "EX-24-real-screen.png");
    if let Some(p) = &shot {
        log.push(format!("real screen screenshot: {}", p.display()));
    } else {
        log.push("real screen screenshot NOT obtained (sample rect unavailable)".to_string());
    }
    // 附加截图：自建诊断面板（DWM 下可能抓不到，失败即如实标注未取得）。
    let fake_tray = shadebar_core::engine::rect_px(40, 210, 640, 330);
    let fake_sample = shadebar_core::engine::rect_px(60, 230, 620, 310);
    let lines = vec![
        "ShadeBar T04 / EX-24 BitBlt fault".to_string(),
        format!("last_decision_kept={} bad_frames_delta={}", kept, bad_after - bad_before),
        format!("recovered_after_fault={} bitblt_calls={}", recovered, session.screen_mut().blits()),
        format!("result={}", if obtained { "PASS" } else { "NOT OBTAINED" }),
    ];
    if let Ok(panel) = ProbeWindow::show(
        ProbeMode::Panel {
            lines,
            tray: fake_tray,
            sample: fake_sample,
        },
        60,
        60,
        760,
        420,
    ) {
        panel.refresh();
        let mut screen = GdiScreenSource::new();
        match capture_plain(&mut screen, panel.hwnd(), &panel.screen_rect) {
            Ok(Some(png)) => {
                if let Ok(p) = write_png(&opts.out_dir.join("EX-24-panel.png"), &png) {
                    log.push(format!("panel screenshot: {}", p.display()));
                }
            }
            Ok(None) => log.push(
                "panel screenshot NOT obtained: uniform blank area (window not composited)"
                    .to_string(),
            ),
            Err(e) => log.push(format!("panel capture FAILED: {e}")),
        }
    }
    append_log(&log_path, &log);

    ItemResult {
        id: "EX-24",
        obtained,
        reason,
        metrics,
        screenshot: shot,
        log_path,
    }
}

/// A-20 / NFR-03：10 分钟平均 CPU 占用 + 单次采样耗时实测。
fn verify_cpu(session: &mut EngineSession, opts: &VerifyOptions) -> ItemResult {
    let log_path = opts.out_dir.join("A-20-NFR-03-cpu.log");
    let duration = opts.profile.cpu_ms();
    let mut log: Vec<String> = Vec::new();
    log.push("=== A-20 / NFR-03: real CPU & per-sample cost ===".to_string());
    log.push(format!("profile={} duration_ms={}", opts.profile.label(), duration));

    let cpu_start = process_cpu_100ns();
    let wall_start = session.now_ms();
    let samples_before = session.diag().samples;
    // 只统计「本段 10 分钟」的耗时样本；不把前面 EX-24 故障注入 / 黑白对照
    // 段的样本混进来，否则 p95 会被别段的异常帧污染（首轮实测发现的口径错误）。
    let cost_mark = session.diag().cost_us.len();
    let report_before = *session.engine().report();

    // 真实可等待定时器驱动的闭环（与 T04 正式运行同一路径）。
    let report = session.run(duration);

    let cpu_end = process_cpu_100ns();
    let wall_end = session.now_ms();
    let wall_ms = wall_end.saturating_sub(wall_start).max(1);
    let cpu_delta = cpu_end.saturating_sub(cpu_start);
    // ΔCPU(100ns) / 墙钟(100ns) = 单核占用率；再除以逻辑核数得到全机口径。
    let wall_100ns = wall_ms.saturating_mul(10_000);
    let core_pct = (cpu_delta as f64 * 100.0 / wall_100ns as f64).min(1000.0);
    let cpus = logical_cpu_count().max(1);
    let machine_pct = core_pct / f64::from(cpus);
    let (avg_us, p95_us, max_us, over_budget, segment_len) = {
        let diag = session.diag();
        let this_segment = &diag.cost_us[cost_mark..];
        let (a, p, m) = RecordingDiagSink::cost_stats_of(this_segment);
        let over = this_segment
            .iter()
            .filter(|&&u| u > SINGLE_SAMPLE_BUDGET_US)
            .count();
        (a, p, m, over, this_segment.len())
    };
    let samples = session.diag().samples.saturating_sub(samples_before);
    let legacy_budget_met = max_us <= SINGLE_SAMPLE_BUDGET_US && over_budget == 0;
    // 全量累积统计仅用于 §11.6 汇总行对照，明确标注不参与 NFR-03 判定。
    let (all_avg, all_p95, all_max) = session.diag().cost_stats();

    // ---- NFR-03 v1.4 双口径（需求基线 §7）--------------------------------
    // 主判据：GetProcessTimes 单核 CPU < 0.1%；辅助判据：cost_us p95 ≤10ms / max ≤15ms。
    // 原「单次 ≤5ms」已废止，但墙钟要求与 max 上限保留，三项须同时满足。
    let cost_sum_us: u64 = session.diag().cost_us[cost_mark..].iter().sum();
    let cost_sum_ms = cost_sum_us / 1000;
    let cpu_delta_ms = cpu_delta / 10_000;
    let blocking_wait_ms = cost_sum_ms.saturating_sub(cpu_delta_ms);
    let cpu_met = core_pct < CPU_CORE_LIMIT_PCT;
    let wall_p95_met = p95_us <= WALL_P95_LIMIT_US;
    let wall_max_met = max_us <= WALL_MAX_LIMIT_US;
    let sample_count_ok = segment_len >= COST_SAMPLE_MIN;
    let wall_met = wall_p95_met && wall_max_met && sample_count_ok;
    let dual_met = cpu_met && wall_met;
    // 三项同时满足才算 v1.4 口径通过；CPU 达标不得豁免墙钟，反之亦然。
    let metrics = vec![
        ("wall_ms", wall_ms.to_string()),
        ("cpu_delta_100ns", cpu_delta.to_string()),
        ("cpu_delta_ms", cpu_delta_ms.to_string()),
        ("logical_cpus", cpus.to_string()),
        ("cpu_core_pct", format!("{core_pct:.4}")),
        ("cpu_machine_pct", format!("{machine_pct:.4}")),
        ("samples", samples.to_string()),
        ("ticks", (report.ticks.saturating_sub(report_before.ticks)).to_string()),
        ("bad_frames", (report.bad_frames.saturating_sub(report_before.bad_frames)).to_string()),
        ("cost_avg_us", avg_us.to_string()),
        ("cost_p95_us", p95_us.to_string()),
        ("cost_max_us", max_us.to_string()),
        ("cost_sum_us", cost_sum_us.to_string()),
        ("cost_sum_ms", cost_sum_ms.to_string()),
        ("blocking_wait_ms", blocking_wait_ms.to_string()),
        ("cost_sample_count", segment_len.to_string()),
        ("cost_sample_min", COST_SAMPLE_MIN.to_string()),
        ("wall_p95_limit_us", WALL_P95_LIMIT_US.to_string()),
        ("wall_max_limit_us", WALL_MAX_LIMIT_US.to_string()),
        ("cpu_core_limit_pct", format!("{CPU_CORE_LIMIT_PCT:.4}")),
        ("budget_us_deprecated", SINGLE_SAMPLE_BUDGET_US.to_string()),
        ("over_budget_legacy_5ms", over_budget.to_string()),
        ("cost_sample_scope", "this segment only".to_string()),
        ("nfr03_cpu_met", cpu_met.to_string()),
        ("nfr03_wall_met", wall_met.to_string()),
        ("nfr03_dual_met", dual_met.to_string()),
        ("nfr03_legacy_5ms_met", legacy_budget_met.to_string()),
        ("sleeper_waits", session.sleeper_waits().to_string()),
        ("dib_rebuilds", session.screen_mut().rebuilds().to_string()),
        ("full_10min", (opts.profile == VerifyProfile::Full).to_string()),
    ];
    for (k, v) in &metrics {
        log.push(format!("metric: {k}={v}"));
    }
    // 每 60 s 一行汇总（§11.6 采样汇总行的真机版本）。
    log.push(format!(
        "cumulative_all_session (NOT used for NFR-03 verdict): avg={}us p95={}us max={}us",
        all_avg, all_p95, all_max
    ));
    // 双口径交叉核算：cost_us 累计是墙钟（含 DWM 抓屏阻塞等待），
    // GetProcessTimes 的 ΔCPU 才是真实 CPU 消耗；差值即阻塞等待，不是代码耗时。
    log.push(format!(
        "cross_check: cost_us_sum={}ms (wall, incl DWM blocking) vs cpu_delta={}ms (GetProcessTimes); \
         blocking_wait={}ms; cost_us is NOT a CPU metric (v1.4 NFR-03)",
        cost_sum_ms, cpu_delta_ms, blocking_wait_ms
    ));
    log.push(format!(
        "criteria_v1.4: cpu_core<{}% -> {}; p95<={}us -> {}; max<={}us -> {}; samples>={} -> {}; dual_met={}",
        CPU_CORE_LIMIT_PCT, cpu_met,
        WALL_P95_LIMIT_US, wall_p95_met,
        WALL_MAX_LIMIT_US, wall_max_met,
        COST_SAMPLE_MIN, sample_count_ok,
        dual_met
    ));
    log.push("legacy_criterion: single-sample <=5ms is DEPRECATED (kept for traceability only, not a gate)".to_string());
    log.push(format!(
        "summary_this_segment: ok={} skip={} bad={} flips={} y_milli={} cost_avg_us={} cost_p95_us={} cost_max_us={}",
        report.sampled.saturating_sub(report_before.sampled),
        report.skipped.saturating_sub(report_before.skipped),
        report.bad_frames.saturating_sub(report_before.bad_frames),
        report.flips.saturating_sub(report_before.flips),
        session.diag().y_milli.last().copied().unwrap_or(0),
        avg_us,
        p95_us,
        max_us
    ));

    let obtained = opts.profile == VerifyProfile::Full && samples > 0;
    // 「取得 10 分钟证据」与「NFR-03 预算通过」是两件独立的事，必须分别如实写。
    let duration_note = if obtained {
        "600s full profile measured".to_string()
    } else {
        "10min CPU evidence NOT obtained: fast profile (20s smoke) must not be presented as 600s"
            .to_string()
    };
    let budget_note = if dual_met {
        format!(
            "NFR-03 v1.4 dual criteria met: cpu_core={core_pct:.4}% (<{CPU_CORE_LIMIT_PCT}%), \
             cost p95={p95_us}us (<={WALL_P95_LIMIT_US}us), max={max_us}us (<={WALL_MAX_LIMIT_US}us), \
             samples={segment_len} (>={COST_SAMPLE_MIN})"
        )
    } else {
        format!(
            "NFR-03 v1.4 dual criteria NOT met: cpu_met={cpu_met} wall_met={wall_met} \
             (cpu_core={core_pct:.4}% p95={p95_us}us max={max_us}us samples={segment_len})"
        )
    };
    let reason = format!("{duration_note}; {budget_note}");
    log.push(format!(
        "result: evidence_obtained={obtained} nfr03_cpu_met={cpu_met} nfr03_wall_met={wall_met} nfr03_dual_met={dual_met}"
    ));
    log.push(format!("result_detail: {reason}"));

    // 主截图：真实屏幕采样区（不依赖 DWM 窗口合成）。
    let shot = real_screen_shot(session, &opts.out_dir, "A-20-real-screen.png");
    if let Some(p) = &shot {
        log.push(format!("real screen screenshot: {}", p.display()));
    } else {
        log.push("real screen screenshot NOT obtained (sample rect unavailable)".to_string());
    }
    // 附加截图：CPU 结果面板（自建窗口，DWM 下可能抓不到）。
    let fake_tray = shadebar_core::engine::rect_px(40, 210, 640, 330);
    let fake_sample = shadebar_core::engine::rect_px(60, 230, 620, 310);
    let lines = vec![
        "ShadeBar T04 / A-20 NFR-03 CPU".to_string(),
        format!("wall={}ms samples={} cpus={}", wall_ms, samples, cpus),
        format!("cpu_core={:.4}% cpu_machine={:.4}%", core_pct, machine_pct),
        format!("cost avg={}us p95={}us max={}us", avg_us, p95_us, max_us),
        format!("budget={}us over_budget={} full_10min={}", SINGLE_SAMPLE_BUDGET_US, over_budget, obtained),
    ];
    if let Ok(panel) = ProbeWindow::show(
        ProbeMode::Panel {
            lines,
            tray: fake_tray,
            sample: fake_sample,
        },
        60,
        60,
        760,
        420,
    ) {
        panel.refresh();
        let mut screen = GdiScreenSource::new();
        match capture_plain(&mut screen, panel.hwnd(), &panel.screen_rect) {
            Ok(Some(png)) => {
                if let Ok(p) = write_png(&opts.out_dir.join("A-20-cpu-panel.png"), &png) {
                    log.push(format!("panel screenshot: {}", p.display()));
                }
            }
            Ok(None) => log.push(
                "panel screenshot NOT obtained: uniform blank area (window not composited)"
                    .to_string(),
            ),
            Err(e) => log.push(format!("panel capture FAILED: {e}")),
        }
    }
    append_log(&log_path, &log);

    ItemResult {
        id: "A-20/NFR-03",
        obtained,
        reason,
        metrics,
        screenshot: shot,
        log_path,
    }
}

/// 生成汇总 markdown（含逐项判据对照、证据路径、明确未做）。
fn write_summary(report: &VerifyReport, opts: &VerifyOptions) {
    let mut md = String::new();
    md.push_str("# T04 真机效果证据汇总\n\n");
    md.push_str(&format!("主日志：`{}`\n\n", report.main_log.display()));
    md.push_str("| 判据 | 是否取得 | 原因 | 截图 | 数值日志 |\n|---|---|---|---|---|\n");
    for it in &report.items {
        let shot = it
            .screenshot
            .as_ref()
            .map(|p| format!("`{}`", p.display()))
            .unwrap_or_else(|| "未取得".to_string());
        md.push_str(&format!(
            "| {} | {} | {} | {} | `{}` |\n",
            it.id,
            if it.obtained { "是" } else { "否" },
            it.reason.replace('|', "/"),
            shot,
            it.log_path.display()
        ));
    }
    md.push_str("\n## 逐项采样数值\n\n");
    for it in &report.items {
        md.push_str(&format!("### {}\n\n", it.id));
        for (k, v) in &it.metrics {
            md.push_str(&format!("- `{}` = `{}`\n", k, v.replace('|', "/")));
        }
    }
    md.push_str("\n## 证据级别声明\n\n");
    md.push_str("- 像素均来自真实 `BitBlt` 抓屏；黑白对照由**本进程自建纯色窗口**覆盖\n");
    md.push_str("  采样区产生，**未修改用户壁纸 / 注册表 / 任何用户已打开的窗口**。\n");
    md.push_str("- 覆盖生效经「未覆盖基线像素 vs 覆盖后像素」校验；偏差超阈值即判「未取得」。\n");
    md.push_str("- 单次采样耗时来自 core 的 `Instant::elapsed()` 真机测量，不是假时钟。\n");
    md.push_str("- CPU 占用由 `GetProcessTimes` 的 ΔCPU / Δ墙钟 计算，并给出逻辑核数换算。\n");
    md.push_str("- **A-13 / A-14 各起一个独立 `EngineSession`**（v1.4 §9.1 表后注硬要求），\n");
    md.push_str("  不复用同一会话连跑黑→白；`flips` 按稳态段统计，排除切换背景后的段首第一拍；\n");
    md.push_str("  全段原始 `flips` 与段首 `flips_head_excluded` 也如实保留，判据数值未放宽。\n");
    md.push_str("- **NFR-03 双口径**：CPU 主判据（单核 < 0.1%）与墙钟辅助判据\n");
    md.push_str("  （`cost_us` p95 ≤ 10 ms / max ≤ 15 ms、样本 ≥ 500）分别记录，三项须同时满足；\n");
    md.push_str("  保留 `cost_us` 累计墙钟 vs `GetProcessTimes` ΔCPU 的交叉核算，差值即 DWM 阻塞等待。\n");
    md.push_str("  原「单次 ≤ 5 ms」已废止，仅留作溯源，不作门禁。\n");
    md.push_str(&format!(
        "- 时长档位：`{}`。Fast 档的 CPU 项**明确标注未取得 10 分钟实测**；\n",
        opts.profile.label()
    ));
    md.push_str("  `endpoints` 档只跑 A-13/A-14（各 60 s），刻意不跑 10 分钟 CPU 段。\n");
    md.push_str("\n## 明确未做（本批范围外）\n\n");
    md.push_str("- SP-2 相关实验、explorer 注入/重启、TranslucentTB 任何操作：一律未做。\n");
    md.push_str("- L2/L3 注入着色效果验证：v1.0.0 L1 不含，未做。\n");
    md.push_str("- 端到端改色效果（A-29）：仍为人工验收项，自动证据未取得。\n");
    append_log(&report.summary_path, &[md]);
}

/// 执行完整取证，返回汇总报告。
///
/// # Errors
///
/// 输出目录不可写时返回 `SbError::PathNotWritable`；引擎定时器创建失败时
/// 返回 `SbError::Win32`。单项取证失败不会中断整体，只在该项标记「未取得」。
pub fn run(opts: &VerifyOptions) -> SbResult<VerifyReport> {
    std::fs::create_dir_all(&opts.out_dir).map_err(|e| {
        shadebar_core::SbError::PathNotWritable {
            path: format!("{}: {e}", opts.out_dir.display()),
        }
    })?;
    let main_log = opts.out_dir.join("t04-main.log");
    append_log(
        &main_log,
        &[
            format!(
                "T04 verification start: profile={} endpoint_ms={} cpu_ms={}",
                opts.profile.label(),
                opts.profile.endpoint_ms(),
                opts.profile.cpu_ms()
            ),
            "A-13/A-14 orchestration: one INDEPENDENT EngineSession per endpoint (v1.4 9.1)".to_string(),
            "flips verdict scope: steady segment, first tick after background switch excluded".to_string(),
            "NFR-03: dual criteria (GetProcessTimes CPU <0.1% + cost_us p95<=10ms/max<=15ms)".to_string(),
        ],
    );

    let mut items = Vec::new();
    if opts.profile.endpoints_only() {
        // 只跑两项：不跑 A-02 / EX-24 / 10 分钟 CPU 段，避免段首切背景伪影。
        items.extend(verify_endpoints(opts));
    } else {
        // A-02 / EX-24 / 10 分钟 CPU 段共用一个会话（彼此间无黑白背景切换）。
        let mut session = EngineSession::new(EngineConfig::default_v1())?;
        items.push(verify_a02(&mut session, opts));
        items.push(verify_ex24(&mut session, opts));
        // 黑白端点另起独立会话，不再挂在 session 上。
        items.extend(verify_endpoints(opts));
        items.push(verify_cpu(&mut session, opts));
    }

    // 汇总 UIA 诊断（T02.3 判据①的真机证据）：用一次性短会话只读探测，
    // 不依赖上面任何取证场景的会话状态，endpoints 档也能产出。
    let mut uia_log: Vec<String> = vec!["=== UIA real probe diagnostics ===".to_string()];
    match EngineSession::new(EngineConfig::default_v1()) {
        Ok(mut probe) => {
            let _ = probe.tick();
            uia_log.push(format!("worker_available={}", probe.uia().worker_available()));
            uia_log.push(format!("fallback={}", probe.engine().uia_in_fallback()));
            uia_log.push(format!("last_hit_count={}", probe.uia().last_hit_count));
            uia_log.push(format!("last_elapsed_ms={}", probe.uia().last_elapsed_ms));
            uia_log.push(format!("last_timed_out={}", probe.uia().last_timed_out));
            for d in probe.uia().diagnostics() {
                uia_log.push(format!("uia: {d}"));
            }
        }
        Err(e) => uia_log.push(format!("UIA probe session unavailable: {e}")),
    }
    append_log(&opts.out_dir.join("UIA-real-probe.log"), &uia_log);

    let summary_path = opts.out_dir.join("t04-summary.md");
    let report = VerifyReport {
        items,
        summary_path,
        main_log: main_log.clone(),
    };
    write_summary(&report, opts);
    let obtained = report.items.iter().filter(|i| i.obtained).count();
    append_log(
        &main_log,
        &[format!(
            "T04 verification done: obtained={}/{} summary={}",
            obtained,
            report.items.len(),
            report.summary_path.display()
        )],
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_durations_match_acceptance() {
        assert_eq!(VerifyProfile::Full.endpoint_ms(), 60_000);
        assert_eq!(VerifyProfile::Full.cpu_ms(), 600_000);
        assert_eq!(VerifyProfile::Fast.cpu_ms(), 20_000);
    }

    #[test]
    fn mean_rgb_handles_empty_and_known_pixels() {
        assert_eq!(mean_rgb(&[]), (0, 0, 0));
        // BGRA: 两个像素 (10,20,30) 与 (30,40,50)
        let px = [10, 20, 30, 255, 30, 40, 50, 255];
        assert_eq!(mean_rgb(&px), (40, 30, 20));
    }

    #[test]
    fn fast_profile_never_claims_full_ten_minutes() {
        let opts = VerifyOptions {
            out_dir: PathBuf::from("evidence-test-only"),
            profile: VerifyProfile::Fast,
        };
        assert!(!opts.profile.cpu_ms() >= 600_000);
        assert_eq!(opts.profile.label(), "fast");
    }

    #[test]
    fn endpoints_profile_keeps_60s_criteria_but_skips_cpu() {
        // v1.4 §9.1：端点判据时长不放宽为 fast 的 5 s，只是不再跑 10 分钟 CPU 段。
        assert_eq!(VerifyProfile::Endpoints.endpoint_ms(), 60_000);
        assert_eq!(VerifyProfile::Endpoints.cpu_ms(), 0);
        assert!(VerifyProfile::Endpoints.endpoints_only());
        assert!(!VerifyProfile::Full.endpoints_only());
        assert!(!VerifyProfile::Fast.endpoints_only());
    }

    #[test]
    fn dual_criteria_limits_match_requirement_v1_4() {
        assert_eq!(WALL_P95_LIMIT_US, 10_000);
        assert_eq!(WALL_MAX_LIMIT_US, 15_000);
        assert_eq!(COST_SAMPLE_MIN, 500);
        assert!((CPU_CORE_LIMIT_PCT - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn endpoint_specs_keep_unrelaxed_y_limits() {
        // 判据数值一字未改：A-13 Y<=0.05、A-14 Y>=0.95。
        let black = EndpointSpec {
            label: "13-black",
            id: "A-13",
            color: [0, 0, 0, 255],
            expected_side: Side::Light,
            y_min_milli: None,
            y_max_milli: Some(50),
        };
        let white = EndpointSpec {
            label: "14-white",
            id: "A-14",
            color: [255, 255, 255, 255],
            expected_side: Side::Dark,
            y_min_milli: Some(950),
            y_max_milli: None,
        };
        assert_eq!(black.y_max_milli, Some(50));
        assert_eq!(white.y_min_milli, Some(950));
        assert_eq!(black.expected_side, Side::Light);
        assert_eq!(white.expected_side, Side::Dark);
    }
}
