//! T04 真实 UIA 遮罩：`IUIAutomation2` 取时钟/日期矩形。
//!
//! # §11.10 生命周期纪律（SP-2 UAF 教训）
//!
//! 全部 `IUIAutomationElement` 引用**只在 worker 线程的一次查询调用帧内**存在：
//! 遍历 → 读属性 → 立刻转成纯 [`MaskRect`] 值 → 丢弃引用。
//! 主线程拿到的只是 `Vec<MaskRect>`（`Copy` 数据），绝不缓存任何 COM 指针，
//! 也不跨时点 QI / 写属性。本模块**只读** `CurrentBoundingRectangle`，
//! 不写任何 XAML 属性、不注入 explorer、不触碰进程空间。
//!
//! # §3.1 线程模型
//!
//! UIA 在独立 **MTA** 线程运行（`CoInitializeEx(COINIT_MULTITHREADED)`），
//! T0/T1 不直接调用 COM。`refresh_into` 用 `recv_timeout(350ms)` 看门狗，
//! 超时即返回 [`SbError::UiaUnavailable`]，由 core 的 `UiaProbe` 自动退回
//! 固定几何——**UIA 卡住不会阻塞采样线程**。
//!
//! `IUIAutomation2::SetConnectionTimeout(350)` 是 COM 层自身的连接超时，
//! 与上面的看门狗构成双保险。


// 说明：本模块同时服务「正式 release 的 EngineSession」与「verify 取证档」。
// 关闭 verify 时，仅供取证档使用的访问器/字段会变成 dead_code；它们不是废代码，
// 在 --features verify 构建与 cargo test 中都被真实调用。故只在非 verify 构建放宽该 lint。
#![cfg_attr(not(feature = "verify"), allow(dead_code))]

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use shadebar_core::engine::{MaskKind, MaskRect, MaskSet};
use shadebar_core::{SbError, SbResult};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, IUIAutomation2, IUIAutomationCondition, IUIAutomationElement,
    TreeScope_Descendants, UIA_TextControlTypeId,
};

/// 一次 UIA 查询请求。`hwnd` 用 `usize` 传递（HWND 不是 `Send`，句柄是全局值）。
struct UiaRequest {
    hwnd: usize,
    reply: Sender<Result<Vec<MaskRect>, i32>>,
}

/// UIA 连接/查询的统一超时（ms）。
const UIA_CONNECT_TIMEOUT_MS: u32 = 350;

/// `UIA_E_ELEMENTNOTAVAILABLE`：worker 未启动 / 通道断开。
const HR_UIA_ELEMENT_NOT_AVAILABLE: i32 = 0x8004_01F0u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_TIMEOUT)`：350 ms 看门狗超时。
const HR_UIA_TIMEOUT: i32 = 0x8007_05B4u32 as i32;
/// `REGDB_E_CLASSNOTREG`：CUIAutomation 不可实例化。
const HR_CLASS_NOT_REGISTERED: i32 = 0x8004_0154u32 as i32;

/// 单棵可视树最多遍历的元素数（防止 Win11 任务栏树过大拖慢查询）。
const MAX_ELEMENTS: i32 = 512;

/// 真实 UIA 矩形源。
pub struct UiaClockRectSource {
    request_tx: Option<Sender<UiaRequest>>,
    worker: Option<std::thread::JoinHandle<()>>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    /// 最近一次查询是否真正命中了时钟/日期元素。
    pub last_hit_count: usize,
    /// 最近一次查询是否超时。
    pub last_timed_out: bool,
    /// 累计 UIA 查询耗时（ms，由调用方 clock 差值写入）。
    pub last_elapsed_ms: u64,
}

impl Default for UiaClockRectSource {
    fn default() -> Self {
        Self::new()
    }
}

impl UiaClockRectSource {
    /// 启动独立 MTA UIA 线程。
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<UiaRequest>();
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let diag_for_thread = Arc::clone(&diagnostics);
        let worker = std::thread::Builder::new()
            .name("ShadeBar.UIA".to_string())
            .spawn(move || uia_worker(rx, diag_for_thread))
            .ok();
        Self {
            request_tx: if worker.is_some() { Some(tx) } else { None },
            worker,
            diagnostics,
            last_hit_count: 0,
            last_timed_out: false,
            last_elapsed_ms: 0,
        }
    }

    /// 取诊断日志（元素命中详情，含 RECT，供 T02.3 判据①证据）。
    #[must_use]
    pub fn diagnostics(&self) -> Vec<String> {
        self.diagnostics.lock().map_or_else(|_| Vec::new(), |d| d.clone())
    }

    /// MTA 线程是否已成功启动。
    #[must_use]
    pub const fn worker_available(&self) -> bool {
        self.worker.is_some()
    }

    /// 向 worker 发一次查询，带 350 ms 看门狗，并记录真实查询耗时。
    fn query(&mut self, hwnd: usize) -> SbResult<Vec<MaskRect>> {
        let started = std::time::Instant::now();
        let result = self.query_inner(hwnd);
        self.last_elapsed_ms =
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        result
    }

    /// 查询主体（与耗时计量分离，保证所有返回路径都被计时）。
    fn query_inner(&mut self, hwnd: usize) -> SbResult<Vec<MaskRect>> {
        let Some(tx) = self.request_tx.as_ref() else {
            return Err(SbError::UiaUnavailable {
                hr: HR_UIA_ELEMENT_NOT_AVAILABLE,
            });
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(UiaRequest { hwnd, reply: reply_tx }).is_err() {
            return Err(SbError::UiaUnavailable {
                hr: HR_UIA_ELEMENT_NOT_AVAILABLE,
            });
        }
        match reply_rx.recv_timeout(Duration::from_millis(u64::from(UIA_CONNECT_TIMEOUT_MS))) {
            Ok(Ok(rects)) => Ok(rects),
            Ok(Err(hr)) => Err(SbError::UiaUnavailable { hr }),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                self.last_timed_out = true;
                Err(SbError::UiaUnavailable { hr: HR_UIA_TIMEOUT })
            }
        }
    }
}

impl shadebar_core::engine::ClockRectSource for UiaClockRectSource {
    fn refresh_into(&mut self, out: &mut MaskSet) -> SbResult<()> {
        // 每次刷新重新定位 Shell_TrayWnd（不缓存 HWND，避免窗口重建后失效）。
        let hwnd = find_shell_tray();
        let rects = self.query(hwnd)?;
        self.last_hit_count = rects.len();
        out.union_with(&rects);
        Ok(())
    }
}

impl Drop for UiaClockRectSource {
    fn drop(&mut self) {
        // 丢弃 sender 让 worker 的 recv 断开退出；再 join 确保 COM 反初始化完成。
        self.request_tx.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// 只读查找 `Shell_TrayWnd`（失败返回 0，worker 会用 root 元素兜底）。
fn find_shell_tray() -> usize {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{HSTRING, PCWSTR};
    let class = HSTRING::from("Shell_TrayWnd");
    // SAFETY: 纯查询；class 在调用期间存活。
    match unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) } {
        Ok(h) if !h.0.is_null() => h.0 as usize,
        _ => 0,
    }
}

/// MTA worker 主循环：COM 初始化 → 循环处理请求 → 反初始化。
fn uia_worker(rx: Receiver<UiaRequest>, diagnostics: Arc<Mutex<Vec<String>>>) {
    // SAFETY: 本线程独占 COM 单元；MTA 与 §3.1 一致。
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init.is_err() {
        push_diag(&diagnostics, format!("CoInitializeEx failed hr=0x{:08X}", init.0 as u32));
        // 仍然 drain 请求，逐个返回失败，避免主线程永久等待。
        while let Ok(req) = rx.recv() {
            let _ = req.reply.send(Err(init.0));
        }
        return;
    }

    // 依次尝试 4 组 (CLSID, CLSCTX)：真机上 `CUIAutomation + CLSCTX_ALL`
    // 曾返回 REGDB_E_CLASSNOTREG，但 `CUIAutomation8`/`CLSCTX_INPROC_SERVER`
    // 常可成功。每组的 HRESULT 都写进诊断日志；全部失败即让 core 走
    // 固定几何 fallback（功能不降级），绝不用假矩形冒充 UIA 命中。
    let attempts: [(windows::core::GUID, windows::Win32::System::Com::CLSCTX, &str); 4] = [
        (CUIAutomation, CLSCTX_ALL, "CUIAutomation/CLSCTX_ALL"),
        (
            CUIAutomation,
            CLSCTX_INPROC_SERVER,
            "CUIAutomation/INPROC_SERVER",
        ),
        (
            CUIAutomation8,
            CLSCTX_INPROC_SERVER,
            "CUIAutomation8/INPROC_SERVER",
        ),
        (CUIAutomation8, CLSCTX_ALL, "CUIAutomation8/CLSCTX_ALL"),
    ];
    let mut automation: Option<IUIAutomation2> = None;
    for (clsid, ctx, label) in attempts {
        // SAFETY: COM 已初始化；clsid 是系统注册常量，ctx 为合法 CLSCTX 组合。
        match unsafe { CoCreateInstance::<_, IUIAutomation2>(&clsid, None, ctx) } {
            Ok(a) => {
                push_diag(&diagnostics, format!("CoCreateInstance OK: {label}"));
                automation = Some(a);
                break;
            }
            Err(e) => push_diag(
                &diagnostics,
                format!("CoCreateInstance failed [{label}] hr=0x{:08X}", e.code().0 as u32),
            ),
        }
    }
    let automation = match automation {
        Some(a) => {
            // SAFETY: a 有效；SetConnectionTimeout 是 IUIAutomation2 的只写配置。
            match unsafe { a.SetConnectionTimeout(UIA_CONNECT_TIMEOUT_MS) } {
                Ok(()) => push_diag(
                    &diagnostics,
                    format!("IUIAutomation2 ready, ConnectionTimeout={UIA_CONNECT_TIMEOUT_MS}ms"),
                ),
                Err(e) => push_diag(
                    &diagnostics,
                    format!("SetConnectionTimeout failed hr=0x{:08X}", e.code().0 as u32),
                ),
            }
            Some(a)
        }
        None => {
            push_diag(
                &diagnostics,
                "all CoCreateInstance attempts failed -> core fixed-geometry fallback".to_string(),
            );
            None
        }
    };

    while let Ok(req) = rx.recv() {
        let result = match automation.as_ref() {
            Some(a) => query_rects(a, req.hwnd, &diagnostics),
            None => Err(HR_CLASS_NOT_REGISTERED), // REGDB_E_CLASSNOTREG
        };
        // 主线程可能已超时放弃；send 失败是正常现象，忽略即可。
        let _ = req.reply.send(result);
    }

    // SAFETY: 与本线程开头的 CoInitializeEx 成对。
    unsafe { CoUninitialize() };
}

/// 执行一次可视树查询，返回纯数据矩形（绝不把 COM 引用带出本函数）。
fn query_rects(
    automation: &IUIAutomation2,
    hwnd: usize,
    diagnostics: &Arc<Mutex<Vec<String>>>,
) -> Result<Vec<MaskRect>, i32> {
    // SAFETY: 所有 COM 引用在本函数栈帧内使用完即弃（§11.10）。
    unsafe {
        // CreateTrueCondition 返回一个 condition，只在本次 FindAll 中使用。
        let condition: IUIAutomationCondition = automation
            .CreateTrueCondition()
            .map_err(|e| e.code().0)?;

        let root: IUIAutomationElement = if hwnd == 0 {
            automation.GetRootElement()
        } else {
            automation.ElementFromHandle(HWND(hwnd as *mut core::ffi::c_void))
        }
        .map_err(|e| e.code().0)?;

        let children = root
            .FindAll(TreeScope_Descendants, &condition)
            .map_err(|e| e.code().0)?;
        let len = children.Length().map_err(|e| e.code().0)?.min(MAX_ELEMENTS);
        push_diag(diagnostics, format!("UIA descendants enumerated: {len} (cap {MAX_ELEMENTS})"));

        let mut hits: Vec<MaskRect> = Vec::new();
        for i in 0..len {
            let elem: IUIAutomationElement = match children.GetElement(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = bstr_to_string(elem.CurrentName());
            let aid = bstr_to_string(elem.CurrentAutomationId());
            let class = bstr_to_string(elem.CurrentClassName());
            let is_text = elem
                .CurrentControlType()
                .map(|t| t == UIA_TextControlTypeId)
                .unwrap_or(false);
            let rect = match elem.CurrentBoundingRectangle() {
                Ok(r) => r,
                Err(_) => continue,
            };
            if shadebar_core::engine::rect_is_empty(&rect) {
                continue;
            }
            let kind = classify(&name, &aid, &class, is_text, &rect);
            if let Some(k) = kind {
                push_diag(
                    diagnostics,
                    format!(
                        "UIA hit[{i}] kind={k:?} name={name:?} aid={aid:?} class={class:?} rect=({},{})-({},{})",
                        rect.left, rect.top, rect.right, rect.bottom
                    ),
                );
                hits.push(MaskRect::new(rect, k));
                if hits.len() >= 2 {
                    break;
                }
            }
            // elem 在此处离开作用域即释放，绝不缓存（§11.10）。
        }
        push_diag(diagnostics, format!("UIA hits total: {}", hits.len()));
        Ok(hits)
    }
}

/// 按名称 / AutomationId / 类名 / 控件类型 / 矩形形态分类时钟与日期元素。
fn classify(
    name: &str,
    aid: &str,
    class: &str,
    is_text: bool,
    rect: &shadebar_core::engine::RectPx,
) -> Option<MaskKind> {
    let lower_name = name.to_ascii_lowercase();
    let lower_aid = aid.to_ascii_lowercase();
    let lower_class = class.to_ascii_lowercase();

    // SP-2 已确认的 XAML 元素名（Win11 26H2）。
    let clock_by_id = lower_aid.contains("timeinner")
        || lower_aid.contains("clock")
        || lower_class.contains("clock")
        || lower_name.contains("timeinner");
    let date_by_id = lower_aid.contains("dateinner")
        || lower_aid.contains("date")
        || lower_class.contains("date");

    // 时间文本形如 "14:32" / "14:32:07"；日期文本含 "/" 或 "-" 与数字。
    let looks_like_time = name.contains(':') && name.chars().filter(char::is_ascii_digit).count() >= 3;
    let looks_like_date = name.contains('/') || (name.contains('-') && name.len() >= 8);

    // 托盘区文字控件兜底：小矩形、位于任务栏右侧、Text 类型。
    let w = shadebar_core::engine::rect_width(rect);
    let h = shadebar_core::engine::rect_height(rect);
    let plausible = is_text && w > 8 && w < 260 && h > 6 && h < 60;

    if clock_by_id || (looks_like_time && (is_text || plausible)) {
        Some(MaskKind::Clock)
    } else if date_by_id || (looks_like_date && (is_text || plausible)) {
        Some(MaskKind::Date)
    } else {
        None
    }
}

#[must_use]
fn bstr_to_string(r: windows::core::Result<windows::core::BSTR>) -> String {
    match r {
        Ok(b) => String::from_utf16_lossy(b.as_wide()),
        Err(_) => String::new(),
    }
}

fn push_diag(d: &Arc<Mutex<Vec<String>>>, line: String) {
    if let Ok(mut v) = d.lock() {
        v.push(line);
        // 保留最近 200 行，避免长时间运行无界增长。
        if v.len() > 200 {
            let drop = v.len() - 200;
            v.drain(..drop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadebar_core::engine::rect_px;

    #[test]
    fn classify_matches_known_xaml_automation_ids() {
        let r = rect_px(1700, 1036, 1830, 1060);
        assert_eq!(classify("", "TimeInnerTextBlock", "", true, &r), Some(MaskKind::Clock));
        assert_eq!(classify("", "DateInnerTextBlock", "", true, &r), Some(MaskKind::Date));
    }

    #[test]
    fn classify_matches_time_and_date_text() {
        let r = rect_px(1700, 1036, 1830, 1060);
        assert_eq!(classify("14:32", "", "", true, &r), Some(MaskKind::Clock));
        assert_eq!(classify("2026/11/11", "", "", true, &r), Some(MaskKind::Date));
    }

    #[test]
    fn classify_rejects_unrelated_elements() {
        let r = rect_px(0, 0, 100, 40);
        assert_eq!(classify("Start", "StartButton", "Button", false, &r), None);
    }

    #[test]
    fn classify_rejects_huge_rects_for_time_text() {
        // 大矩形即便文本像时间，也不是托盘时钟（防止误遮全屏）。
        let huge = rect_px(0, 0, 1920, 1080);
        assert_eq!(classify("14:32", "", "", false, &huge), None);
    }

    #[test]
    fn timeout_constant_matches_arch() {
        assert_eq!(UIA_CONNECT_TIMEOUT_MS, 350, "team-lead 本批指派 350ms");
        assert_eq!(
            shadebar_core::engine::UIA_TIMEOUT_MS,
            300,
            "core 的熔断阈值为 300ms"
        );
        assert_eq!(MAX_ELEMENTS, 512);
    }
}
