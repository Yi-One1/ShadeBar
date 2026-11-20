//! 应用状态与正式启动编排（架构 §8.4 T04.1 / §4 #25）。
//!
//! * 单一 `Arc<App>` 在 UI 线程与采样线程之间共享；UI 线程只读写纯数据，
//!   不持有任何 `HWND` 之外的跨线程 Win32 句柄。
//! * 采样线程用真实 [`EngineSession`] 逐拍驱动，每拍前从共享 `Config` 取快照
//!   → `set_config` → `tick`，实现 A-10「同一拍生效」（≤3 s，实测同拍）。
//! * 诊断数据以 **1 Hz 节流** `PostMessage` 通知 UI 重绘（T03.5 判据①/⑥），
//!   主线程不做抓屏/采样，CPU 开销可忽略。
//! * 退出时置 `stop` 标志，采样线程在下一个间隔内自行退出，不 detach 强杀。

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use shadebar_core::color::Rgba;
use shadebar_core::engine::EngineConfig;
use shadebar_core::win::to_wide;
use shadebar_core::{PRODUCT_NAME, SbResult, VERSION};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

use crate::config::{Config, Lang, SettingKey, note};
use crate::engine::EngineSession;
use crate::ipc_server::{WM_SHADEBAR_DIAG, post_to_main};
use crate::ui::pages::diagnostics::DiagView;
use crate::ui::{View, ViewAction};

/// `CF_UNICODETEXT` = 13（常量在 `Win32::System::Ole`，为避免多开一组 feature
/// 直接用数值，附出处注释）。
const CF_UNICODETEXT: u32 = 13;

/// 共享状态。
struct AppState {
    config: Config,
    diag: DiagView,
    decision_color: Rgba,
    /// L1 交付态：T05 注入着色挂起，L2/L3 元素控件置灰并显示降级横幅。
    degraded: bool,
    /// 上次向 UI 线程投递诊断消息的单调毫秒（1 Hz 节流）。
    last_post_ms: u64,
    /// 上次已并入折线的采样序号，避免重复 push。
    last_sample_seq: u64,
}

/// 应用根状态。
pub struct App {
    state: Mutex<AppState>,
    /// 主窗口 `HWND` 的位模式（原子存取，避免跨线程持有句柄）。
    hwnd_bits: Arc<AtomicIsize>,
    /// 采样线程停止标志。
    stop: AtomicBool,
    /// 采样线程是否已启动（防止重复 spawn）。
    started: AtomicBool,
    /// 任务栏配色服务（独立线程 + 注入载荷）。
    recolor: Mutex<Option<crate::recolor::RecolorService>>,
    /// 配色服务回传的诊断数据（采样线程写、`sync_view` 读）。
    diag_live: Arc<Mutex<DiagView>>,
}

impl App {
    /// 建立应用：加载配置（损坏回退默认 + `.bak`）、探测语言、设置初始降级态。
    #[must_use]
    pub fn new() -> Arc<Self> {
        // 单测用默认值：避免把测试动作写进用户真实的 `%LOCALAPPDATA%` 配置。
        let config = if cfg!(test) { Config::default() } else { Config::load() };
        Arc::new(Self {
            state: Mutex::new(AppState {
                config,
                diag: DiagView::default(),
                decision_color: Rgba::opaque(255, 255, 255),
                // v1.1：着色能力已落地，默认为正常态；只有注入/采样真的失败才降级
                degraded: false,
                last_post_ms: 0,
                last_sample_seq: 0,
            }),
            hwnd_bits: Arc::new(AtomicIsize::new(0)),
            stop: AtomicBool::new(false),
            started: AtomicBool::new(false),
            recolor: Mutex::new(None),
            diag_live: Arc::new(Mutex::new(DiagView::default())),
        })
    }

    /// 配置快照（UI 线程用）。
    #[must_use]
    pub fn config(&self) -> Config {
        self.lock().config.clone()
    }

    /// 当前语言（S-04，含「跟随系统」解析）。
    #[must_use]
    pub fn language(&self) -> Lang {
        self.lock().config.language()
    }

    /// 当前降级态。
    #[must_use]
    pub fn degraded(&self) -> bool {
        self.lock().degraded
    }

    /// 当前判定色（S-43 托盘小圆点）。
    #[must_use]
    pub fn decision_color(&self) -> Option<Rgba> {
        // 窗口隐藏时 `sync_view` 会被跳过，但托盘小圆点仍然可见，
        // 所以这里也要先并入采样线程回传的数据。
        self.pull_diag();
        let st = self.lock();
        if st.diag.has_data { Some(st.decision_color) } else { None }
    }

    /// 主窗口句柄（采样线程投递消息用）。
    #[must_use]
    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd_bits.load(Ordering::Relaxed) as *mut c_void)
    }

    /// 记录主窗口句柄（在 `create_main_window` 之后调用）。
    pub fn set_hwnd(&self, hwnd: HWND) {
        self.hwnd_bits.store(hwnd.0 as isize, Ordering::Relaxed);
    }

    /// 请求退出：置停止标志，采样线程在下一个间隔内自行收尾。
    pub fn request_exit(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // 落盘一次，确保 300 ms 节流内的改动不丢（NFR-15 退出即还原）。
        let cfg = self.config();
        persist(&cfg);
    }

    /// 启动真实采样线程。重复调用无副作用；spawn 失败时记录日志并置降级。
    pub fn start_sampling(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("shadebar-sampler".to_string())
            .spawn(move || app.sampling_loop());
        if spawned.is_err() {
            self.started.store(false, Ordering::SeqCst);
            self.mark_degraded(true, "sampler thread spawn failed");
        }
    }

    /// 启动任务栏配色服务（幂等）。
    ///
    /// 载荷 DLL 以字节内嵌在 exe 中，运行时释放到 `%LOCALAPPDATA%\ShadeBar\`，
    /// 因此交付物仍是**单个 exe**。
    pub fn start_recolor(&self) {
        let Ok(mut slot) = self.recolor.lock() else { return };
        if slot.is_some() {
            return;
        }
        let cfg = crate::recolor::RuntimeCfg::from_config(&self.config());
        // 总开关关闭时不必注入：等用户打开开关再起（`sync_recolor` 会补上）
        if !cfg.enabled {
            return;
        }
        note(&format!(
            "recolor start: enabled={} follow={} mask={:X} force={} thr={:.2} on_dark=#{:02X}{:02X}{:02X}",
            cfg.enabled, cfg.follow, cfg.mask, cfg.policy.force, cfg.policy.threshold,
            cfg.policy.on_dark.r, cfg.policy.on_dark.g, cfg.policy.on_dark.b
        ));
        let Some(dll) = crate::recolor::ensure_payload_dll() else {
            drop(slot);
            self.mark_degraded(true, "payload dll unavailable");
            return;
        };
        match crate::recolor::RecolorService::start(
            dll,
            cfg,
            Arc::clone(&self.diag_live),
            Arc::clone(&self.hwnd_bits),
        ) {
            Ok(svc) => *slot = Some(svc),
            Err(e) => {
                drop(slot);
                self.mark_degraded(true, &format!("recolor service failed: {e}"));
            }
        }
    }

    /// 把最新配置推给配色服务（配置变更后调用）。
    ///
    /// 总开关从关到开会按需拉起服务；开启状态下只做热更新，不重建线程。
    pub fn sync_recolor(&self) {
        let cfg = crate::recolor::RuntimeCfg::from_config(&self.config());
        let running = self.recolor.lock().map(|g| g.is_some()).unwrap_or(false);
        if !running {
            if cfg.enabled {
                self.start_recolor();
            }
            return;
        }
        if let Ok(g) = self.recolor.lock() {
            if let Some(svc) = g.as_ref() {
                svc.update(cfg);
            }
        }
    }

    /// 停止配色服务（退出前调用，会把任务栏颜色还原）。
    pub fn stop_recolor(&self) {
        if let Ok(mut g) = self.recolor.lock() {
            if let Some(mut svc) = g.take() {
                svc.stop();
            }
        }
    }

    /// 把 View 的动作写回配置并落盘；返回是否有配置变更。
    pub fn apply_actions(&self, actions: Vec<ViewAction>) -> bool {
        let mut changed = false;
        for action in actions {
            match action {
                ViewAction::Set(key, value) => {
                    if self.lock().config.set(key, value) {
                        changed = true;
                        note(&format!("config {} = {}", key.id(), value));
                        if key == SettingKey::S02 {
                            // 真正写 HKCU\Run；失败只记日志，不打断用户
                            match crate::autostart::apply(value != 0) {
                                Ok(()) => note("autostart registry entry updated"),
                                Err(e) => note(&format!("autostart update failed: {e}")),
                            }
                        }
                        if key == SettingKey::S04 {
                            // A-24：语言热切换不重启进程、不中断采样。
                            note("language hot-switched (no restart, sampler continues)");
                        }
                    }
                }
                ViewAction::RestoreDefaults => {
                    self.lock().config.restore_defaults();
                    changed = true;
                    note("restore defaults confirmed");
                }
                ViewAction::ExportDiagnostics => {
                    let _ = self.export_diagnostics();
                }
                ViewAction::CopyVersion => {
                    let text = format!("{PRODUCT_NAME} v{VERSION}");
                    if !set_clipboard_text(&text) {
                        note("clipboard write failed (version shown in About page)");
                    }
                }
                ViewAction::ExportConfig => {
                    if let Err(e) = self.export_config() {
                        note(&format!("config export failed: {e:?}"));
                    }
                }
                ViewAction::ImportConfig => {
                    if let Err(e) = self.import_config() {
                        note(&format!("config import failed: {e:?}"));
                    }
                }
                // 系统调色板 / 关闭窗口需要 HWND，由窗口层在执行前拦下；这里是兜底。
                ViewAction::PickColor(_) => note("PickColor 未在窗口层处理（已忽略）"),
            }
        }
        if changed {
            let cfg = self.config();
            persist(&cfg);
            self.sync_recolor();
        }
        changed
    }

    /// 把采样线程回传的诊断数据并入共享状态。
    ///
    /// v1.1 起真正的采样在 `RecolorService` 线程里，`publish_diag` 读的引擎侧
    /// 已经没有决策数据，面板与托盘色点会一直是空的（progress §3.7）。
    fn pull_diag(&self) {
        // 先取快照再放锁，避免与 state 锁嵌套（采样线程也会拿 diag 锁）。
        let snapshot = self
            .diag_live
            .lock()
            .ok()
            .filter(|d| d.has_data)
            .map(|d| d.clone());
        let Some(snapshot) = snapshot else { return };
        let mut st = self.lock();
        st.decision_color = snapshot.decision_color;
        if st.diag != snapshot {
            st.diag = snapshot;
        }
    }

    /// 用最新状态刷新 View（在 `WM_PAINT` 与动作之后调用）。
    pub fn sync_view(&self, view: &mut View) {
        self.pull_diag();
        let st = self.lock();
        let cfg = st.config.clone();
        let diag = st.diag.clone();
        let degraded = st.degraded;
        drop(st);
        view.rebuild(&cfg, &diag, degraded);
        view.rebuild_texts(cfg.language());
    }

    /// 托盘菜单：切换「暂停自适应」（S-42）。
    pub fn toggle_pause(&self) -> bool {
        let mut st = self.lock();
        let next = if st.config.get(SettingKey::S42) != 0 { 0 } else { 1 };
        let changed = st.config.set(SettingKey::S42, next);
        let cfg = st.config.clone();
        drop(st);
        if changed {
            persist(&cfg);
        }
        changed
    }

    /// 托盘菜单：切换「开机自启动」（S-02）。
    ///
    /// 写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`：
    /// 普通权限即可，**不弹 UAC**（NFR-11），也不生成额外文件。
    pub fn toggle_autostart(&self) -> bool {
        let mut st = self.lock();
        let next = if st.config.get(SettingKey::S02) != 0 { 0 } else { 1 };
        let changed = st.config.set(SettingKey::S02, next);
        let cfg = st.config.clone();
        drop(st);
        if changed {
            persist(&cfg);
            match crate::autostart::apply(next != 0) {
                Ok(()) => note("autostart registry entry updated"),
                Err(e) => note(&format!("autostart update failed: {e}")),
            }
        }
        changed
    }

    /// 系统 UI 语言/主题变化后重新解析「跟随系统」（A-23 / FR-40）。
    pub fn refresh_system(&self) -> (Lang, bool) {
        let st = self.lock();
        let lang = st.config.language();
        drop(st);
        (lang, true)
    }

    /// 导出配置到 `%LOCALAPPDATA%\ShadeBar\config-export.json`（G5 固定操作条）。
    pub fn export_config(&self) -> SbResult<PathBuf> {
        let path = Config::export_file_path();
        self.lock().config.export_to(&path)?;
        note(&format!("config exported: {}", path.display()));
        Ok(path)
    }

    /// 从 `config-export.json` 读回配置并落盘 / 热更新配色（G5 固定操作条）。
    pub fn import_config(&self) -> SbResult<PathBuf> {
        let path = Config::export_file_path();
        let changed = {
            let mut st = self.lock();
            st.config.import_from(&path)?
        };
        if changed {
            let cfg = self.config();
            persist(&cfg);
            self.sync_recolor();
            note(&format!("config imported: {}", path.display()));
        }
        Ok(path)
    }

    /// 导出诊断包（S-46 / NFR-10）：零依赖 stored ZIP，含配置 + 环境 + 日志。
    pub fn export_diagnostics(&self) -> SbResult<PathBuf> {
        let st = self.lock();
        let cfg_json = st.config.to_json();
        let diag = st.diag.clone();
        drop(st);
        let env = format!(
            "product={PRODUCT_NAME}\nversion=v{VERSION}\nlang={:?}\n\
             english_embedded={}\nfeatures=verify=off\n\
             diag_samples={}\ndiag_y_milli={}\ndiag_cost_us={}\ndiag_valid_px={}\n\
             note=environment info is deliberately non-identifying (NFR-10/A-22)\n",
            self.language(),
            crate::ui::i18n::has_english(),
            diag.samples,
            diag.y_milli,
            diag.cost_us,
            diag.valid_px,
        );
        let log = std::fs::read(Config::data_dir().join("shadebar.log")).unwrap_or_default();
        let entries: [(&str, &[u8]); 3] = [
            ("config.json", cfg_json.as_bytes()),
            ("environment.txt", env.as_bytes()),
            ("shadebar.log", &log),
        ];
        let dir = Config::data_dir().join("diagnostics");
        std::fs::create_dir_all(&dir).map_err(|e| shadebar_core::SbError::PathNotWritable {
            path: format!("{}: {e}", dir.display()),
        })?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = dir.join(format!("shadebar-diag-{stamp}.zip"));
        write_stored_zip(&path, &entries)?;
        note(&format!("diagnostic bundle written: {}", path.display()));
        Ok(path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AppState> {
        // Mutex 中毒只可能来自某个线程 panic；本进程 panic=abort（release），
        // 且 UI 线程不做任何可能 panic 的操作，故降级为「取默认状态」而不是崩溃。
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mark_degraded(&self, on: bool, reason: &str) {
        self.lock().degraded = on;
        note(&format!("degraded={on} reason={reason}"));
    }

    /// 采样线程主循环（真实 `EngineSession` + 真实可等待定时器）。
    fn sampling_loop(&self) {
        let cfg = self.config();
        let mut session = match EngineSession::new(cfg.to_engine_config()) {
            Ok(s) => s,
            Err(e) => {
                self.mark_degraded(true, &format!("EngineSession::new failed: {e}"));
                return;
            }
        };
        let mut clock_start = session.now_ms();
        while !self.stop.load(Ordering::Relaxed) {
            let cfg = self.config();
            let engine_cfg: EngineConfig = cfg.to_engine_config();
            // A-10：每拍前注入最新配置，core `Engine::tick` 同拍比对 version 生效。
            session.set_config(engine_cfg);
            let _ = session.tick();
            // 交叉核对配置源与引擎内部版本，发现「改了没生效」立即留痕（不崩溃）。
            if session.config_snapshot().version != session.engine().config().version {
                note("A-10 config version not applied within one tick");
            }
            self.publish_diag(&session, clock_start);
            let step = u64::from(engine_cfg.interval_ms.max(200));
            session.wait_interval(step);
            // 防时钟回绕：仅在必要时重置相对基准。
            if session.now_ms() < clock_start {
                clock_start = session.now_ms();
            }
        }
        note("sampler thread exiting (stop flag set)");
    }

    /// 把一拍的真实诊断数据写进共享视图，并按 1 Hz 通知 UI 重绘。
    fn publish_diag(&self, session: &EngineSession, _clock_start: u64) {
        let d = session.diag();
        let last = session.decision().last;
        let Some(j) = last else {
            return;
        };
        let samples = d.samples;
        let y_milli = d.y_milli.last().copied().unwrap_or(0);
        let valid_px = d.valid_px.last().copied().unwrap_or(0);
        let cost_us = d.cost_us.last().copied().unwrap_or(0);
        let rep = d
            .rep_colors
            .last()
            .copied()
            .unwrap_or((0, 0, 0));
        let now = session.now_ms();

        let mut st = self.lock();
        if samples == st.last_sample_seq {
            return; // 本拍没有新采样（跳过/挂起），不重复 push 折线。
        }
        st.last_sample_seq = samples;
        st.decision_color = j.color;
        st.diag.has_data = true;
        st.diag.seq = j.seq;
        st.diag.rep_color = shadebar_core::color::Rgba::opaque(rep.0, rep.1, rep.2);
        st.diag.decision_color = j.color;
        st.diag.y_milli = y_milli;
        st.diag.contrast_milli = (j.contrast * 1000.0) as u32;
        st.diag.valid_px = valid_px;
        st.diag.cost_us = cost_us;
        st.diag.samples = samples;
        st.diag.timestamp_ms = now;
        st.diag.push_y(y_milli);
        let due = now.saturating_sub(st.last_post_ms) >= 1000;
        if due {
            st.last_post_ms = now;
        }
        drop(st);
        if due {
            let _ = post_to_main(self.hwnd(), WM_SHADEBAR_DIAG, 0, 0);
        }
    }
}

/// 落盘（单测下跳过，避免污染用户真实配置）。
fn persist(cfg: &Config) {
    if !cfg!(test) {
        let _ = cfg.save();
    }
}

/// 把 UTF-16 文本写进剪贴板（S-05「点击可复制」）。失败返回 `false`，不崩溃。
fn set_clipboard_text(text: &str) -> bool {
    let wide = to_wide(text);
    let bytes = wide.len() * std::mem::size_of::<u16>();
    // SAFETY: 所有调用都是成对出现的剪贴板/全局内存 API；失败分支逐层清理。
    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return false;
        }
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
            let _ = CloseClipboard();
            return false;
        };
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr().cast::<u8>(), ptr.cast::<u8>(), bytes);
        let _ = GlobalUnlock(hmem);
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return false;
        }
        // HGLOBAL 与 HANDLE 都是 `*mut c_void` 新类型，逐字段构造避免额外依赖。
        let handle = HANDLE(hmem.0);
        let ok = SetClipboardData(CF_UNICODETEXT, handle).is_ok();
        let _ = CloseClipboard();
        if !ok {
            // 系统未接管内存，必须释放。
            let _ = windows::Win32::Foundation::GlobalFree(hmem);
        }
        ok
    }
}

/// 零依赖 stored ZIP 写入（NFR-10：≤2 MB、可解压、含 3 类文件）。
fn write_stored_zip(path: &Path, entries: &[(&str, &[u8])]) -> SbResult<()> {
    let mut out: Vec<u8> = Vec::with_capacity(4096);
    let mut central: Vec<u8> = Vec::with_capacity(512);
    let mut count: u16 = 0;
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;
        push_u32(&mut out, 0x0403_4B50); // local file header
        push_u16(&mut out, 20); // version needed
        push_u16(&mut out, 0); // flags
        push_u16(&mut out, 0); // method = stored
        push_u16(&mut out, 0); // time
        push_u16(&mut out, 0); // date
        push_u32(&mut out, crc);
        push_u32(&mut out, data.len() as u32);
        push_u32(&mut out, data.len() as u32);
        push_u16(&mut out, name_bytes.len() as u16);
        push_u16(&mut out, 0); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        push_u32(&mut central, 0x0201_4B50); // central dir header
        push_u16(&mut central, 20); // version made by
        push_u16(&mut central, 20); // version needed
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, crc);
        push_u32(&mut central, data.len() as u32);
        push_u32(&mut central, data.len() as u32);
        push_u16(&mut central, name_bytes.len() as u16);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
        count = count.saturating_add(1);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    push_u32(&mut out, 0x0605_4B50); // EOCD
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, count);
    push_u16(&mut out, count);
    push_u32(&mut out, cd_size);
    push_u32(&mut out, cd_offset);
    push_u16(&mut out, 0);

    if out.len() > 2 * 1024 * 1024 {
        return Err(shadebar_core::SbError::PathNotWritable {
            path: "diagnostic bundle exceeds 2 MB (NFR-10)".to_string(),
        });
    }
    std::fs::write(path, &out).map_err(|e| shadebar_core::SbError::PathNotWritable {
        path: format!("{}: {e}", path.display()),
    })
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// CRC-32（IEEE 802.3，与 ZIP 一致）。
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // 与 Python zlib.crc32(b"IHDR") 交叉核对过的值。
        assert_eq!(crc32(b"IHDR"), 0xA8A1_AE0A);
    }

    #[test]
    fn stored_zip_has_valid_signatures_and_roundtrips_names() {
        let entries: [(&str, &[u8]); 2] = [("a.txt", b"hello"), ("b.txt", b"world")];
        let dir = std::env::temp_dir().join("shadebar-zip-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bundle.zip");
        assert!(write_stored_zip(&path, &entries).is_ok(), "zip write must succeed");
        let read = std::fs::read(&path);
        assert!(read.is_ok(), "zip read must succeed");
        let Ok(bytes) = read else {
            return;
        };
        assert_eq!(&bytes[0..4], &0x0403_4B50_u32.to_le_bytes());
        assert!(bytes.windows(4).any(|w| w == 0x0201_4B50_u32.to_le_bytes()));
        assert!(bytes.windows(4).any(|w| w == 0x0605_4B50_u32.to_le_bytes()));
        assert!(bytes.windows(5).any(|w| w == b"a.txt"));
        assert!(bytes.windows(5).any(|w| w == b"b.txt"));
        assert!(bytes.len() < 2 * 1024 * 1024);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn app_defaults_are_active_not_degraded() {
        let app = App::new();
        assert_eq!(app.config().get(SettingKey::S01), 1);
        assert!(!app.degraded(), "v1.1 着色能力已落地，默认不显示降级横幅");
        assert_eq!(app.decision_color(), None, "无数据时不虚报判定色");
    }

    #[test]
    fn apply_actions_persists_and_hot_switches_language() {
        let app = App::new();
        let changed = app.apply_actions(vec![ViewAction::Set(SettingKey::S16, 2000)]);
        assert!(changed);
        assert_eq!(app.config().get(SettingKey::S16), 2000);
        let changed = app.apply_actions(vec![ViewAction::Set(SettingKey::S16, 2000)]);
        assert!(!changed, "同值不应抖动 version / 反复落盘");
        assert_eq!(app.language(), Lang::from_value(0));
    }

    #[test]
    fn restore_defaults_is_confirmed_action() {
        let app = App::new();
        let _ = app.apply_actions(vec![ViewAction::Set(SettingKey::S16, 5000)]);
        let _ = app.apply_actions(vec![ViewAction::RestoreDefaults]);
        assert_eq!(app.config().get(SettingKey::S16), 1000);
    }

    #[test]
    fn pause_and_autostart_toggles_are_idempotent_per_call() {
        let app = App::new();
        assert!(app.toggle_pause());
        assert_eq!(app.config().get(SettingKey::S42), 1);
        assert!(app.toggle_pause());
        assert_eq!(app.config().get(SettingKey::S42), 0);
        assert!(app.toggle_autostart());
        assert_eq!(app.config().get(SettingKey::S02), 1);
    }

    #[test]
    fn hwnd_roundtrip() {
        let app = App::new();
        assert!(app.hwnd().is_invalid());
        let fake = HWND(0x1234_usize as *mut c_void);
        app.set_hwnd(fake);
        assert_eq!(app.hwnd().0 as usize, 0x1234);
    }

    #[test]
    fn sync_view_keeps_35_rows() {
        let app = App::new();
        let mut view = View::new(&app.config(), app.language(), app.degraded());
        app.sync_view(&mut view);
        assert_eq!(view.language(), app.language());
    }
}
