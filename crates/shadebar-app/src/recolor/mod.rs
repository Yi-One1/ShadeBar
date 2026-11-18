//! 任务栏自适应配色：宿主侧（IPC + 注入 + 采样 + 策略）。
//!
//! # 职责
//!
//! 1. **采样**：在任务栏上方取三段（时钟区 / 托盘区 / 图标区）的壁纸亮度；
//! 2. **策略**：按分区亮度各自二值化（可切换全局统一），产出四个元素的颜色；
//! 3. **下发**：把颜色写进共享内存，注入载荷（`ShadeBarTap.dll`）读取并上屏；
//! 4. **兜底**：explorer 重启后自动重新注入；退出时下发还原指令。
//!
//! # 为什么用自声明 FFI
//!
//! 本模块只用到 6 个 Win32 调用，且都不涉及 COM。直接声明签名比依赖
//! `windows` crate 的版本细节更可控，也不会因升级 crate 而漂移。
//!
//! # 线程模型
//!
//! 独立线程，500 ms 一拍；不碰 UI 线程、不持有窗口句柄。退出由
//! [`RecolorService::stop`] 置位标志，线程自行下发还原并 join。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use shadebar_core::color::Rgba;

use crate::config::{argb_to_rgba, Config, SettingKey};
use crate::engine::screen::GdiScreenSource;
use shadebar_core::engine::ScreenBuffer;

// ---------------------------------------------------------------------------
// 原始 FFI
// ---------------------------------------------------------------------------

mod raw {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

    pub const PAGE_READWRITE: u32 = 0x04;
    pub const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;

    pub const PROCESS_CREATE_THREAD: u32 = 0x0002;
    pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    pub const PROCESS_VM_OPERATION: u32 = 0x0008;
    pub const PROCESS_VM_WRITE: u32 = 0x0020;
    pub const PROCESS_VM_READ: u32 = 0x0010;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateFileMappingW(
            file: Handle, attrs: *const c_void, protect: u32,
            size_high: u32, size_low: u32, name: *const u16,
        ) -> Handle;
        pub fn MapViewOfFile(map: Handle, access: u32, off_high: u32, off_low: u32, view: usize) -> *mut c_void;
        pub fn UnmapViewOfFile(base: *const c_void) -> i32;
        pub fn CloseHandle(h: Handle) -> i32;

        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        pub fn VirtualAllocEx(proc: Handle, addr: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
        pub fn VirtualFreeEx(proc: Handle, addr: *mut c_void, size: usize, free_type: u32) -> i32;
        pub fn WriteProcessMemory(proc: Handle, base: *mut c_void, buf: *const c_void, size: usize, written: *mut usize) -> i32;
        pub fn CreateRemoteThread(
            proc: Handle, attrs: *const c_void, stack: usize,
            start: Option<unsafe extern "system" fn(*mut c_void) -> u32>,
            param: *mut c_void, flags: u32, tid: *mut u32,
        ) -> Handle;
        pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
        pub fn GetExitCodeThread(h: Handle, code: *mut u32) -> i32;
        pub fn GetModuleHandleW(name: *const u16) -> Handle;
        pub fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
        pub fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
        pub fn GetWindowRect(hwnd: Handle, rect: *mut [i32; 4]) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
    }
}

/// 共享内存名（与 `payload/ShadeBarIpc.h` 严格一致）。
const IPC_NAME: &str = "Local\\ShadeBar.Ipc.v1";
const IPC_MAGIC: u32 = 0x5342_5231;
const IPC_VERSION: u32 = 1;
const _STATUS_READY: i32 = 1;

/// 共享内存布局。**字段顺序/类型必须与 `payload/ShadeBarIpc.h` 逐字节一致**。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadeBarIpc {
    pub magic: u32,
    pub version: u32,
    pub seq: i32,
    pub enabled: i32,
    pub shutdown: i32,
    pub heartbeat: i32,
    pub status: i32,
    pub tap_seq: i32,
    pub time_color: u32,
    pub date_color: u32,
    pub tray_glyph_color: u32,
    pub indicator_normal_color: u32,
    pub indicator_selected_color: u32,
    pub reserved: [u32; 7],
}

// ---------------------------------------------------------------------------
// 载荷 DLL 释放
// ---------------------------------------------------------------------------

/// 内嵌的载荷 DLL。构建前必须先产出 `payload/build/ShadeBarTap.dll`。
static PAYLOAD_DLL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../payload/build/ShadeBarTap.dll"
));

/// 把内嵌载荷释放到 `%LOCALAPPDATA%\ShadeBar\ShadeBarTap.dll` 并返回路径。
///
/// 每次启动都覆盖写：避免旧版本 DLL 残留导致协议不匹配。
/// 同一时间只允许一个实例（单实例守卫已保证），因此不存在覆盖正在使用的文件。
#[must_use]
pub fn ensure_payload_dll() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = std::path::PathBuf::from(base).join("ShadeBar");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    // 文件名带版本：explorer 已加载同名模块时 LoadLibrary 不会再跑 DllMain，
    // 版本化文件名可保证升级后一定加载到新代码（v1.1 实测踩坑）。
    // 文件名带版本 + 内容指纹：explorer 已加载同名模块时 LoadLibrary 不会再跑
    // DllMain，指纹可保证**内容变了就一定加载新代码**（v1.1 实测踩坑）。
    let name = format!(
        "ShadeBarTap_v{}_{}.dll",
        env!("CARGO_PKG_VERSION"),
        env!("SB_PAYLOAD_TAG")
    );
    let path = dir.join(name);
    // 文件名已含内容指纹，因此"同名"即"同内容"，复用是安全的；
    // 早期版本还有"大小相同则复用"的判断，那会在内容变化但尺寸巧合时复用旧 DLL
    // （v1.1 实测踩坑），指纹文件名从根本上消除了这个隐患。
    //
    // 先写：正常情况下去覆盖旧文件，保证磁盘上的就是当前版本。
    if std::fs::write(&path, PAYLOAD_DLL).is_ok() {
        return Some(path);
    }
    // 写失败的最常见原因是**该文件已被 explorer 映射**（按需加载锁）。
    // 此时不能当作失败——同名即同内容，直接复用即可。
    // 早期版本在这里直接返回 None，导致"第二次启动随影后彻底不工作"，
    // 且没有任何日志（v1.1 实测踩坑，排查耗时最久的一个）。
    match std::fs::metadata(&path) {
        Ok(m) if m.len() == PAYLOAD_DLL.len() as u64 => Some(path),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 颜色计划
// ---------------------------------------------------------------------------

/// 元素掩码位。写进 IPC 的 `reserved[0]`，载荷据此决定改哪些元素；
/// **未置位的元素会被还原成系统原色**，从而让设置里的开关即时生效。
pub mod elem_mask {
    /// 时间文字。
    pub const TIME: u32 = 1 << 0;
    /// 日期文字。
    pub const DATE: u32 = 1 << 1;
    /// 托盘系统字形图标。
    pub const TRAY: u32 = 1 << 2;
    /// 图标状态条（正常 + 选中）。
    pub const INDICATOR: u32 = 1 << 3;
    /// 四项全开。
    pub const ALL: u32 = TIME | DATE | TRAY | INDICATOR;
}

/// 四个元素的目标颜色（`0xAARRGGBB`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorPlan {
    /// 时间文字。
    pub time: u32,
    /// 日期文字。
    pub date: u32,
    /// 托盘系统字形图标。
    pub tray_glyph: u32,
    /// 图标状态条 · 正常态。
    pub indicator_normal: u32,
    /// 图标状态条 · 选中态。
    pub indicator_selected: u32,
    /// 本次要应用的 [`elem_mask`] 位集合。
    pub mask: u32,
}

#[allow(dead_code)]
impl ColorPlan {
    /// 全暗（用于"未就绪"）。
    pub const NONE: Self = Self {
        time: 0,
        date: 0,
        tray_glyph: 0,
        indicator_normal: 0,
        indicator_selected: 0,
        mask: 0,
    };

    /// 同一前景色派生出全部元素色彩。
    ///
    /// `normal_alpha` 是状态条正常态的透明度（默认 0x72，与系统一致）。
    #[must_use]
    pub fn from_foreground(fg: Rgba, normal_alpha: u8) -> Self {
        let argb = u32::from_be_bytes([fg.a, fg.r, fg.g, fg.b]);
        let rgb = argb & 0x00FF_FFFF;
        Self {
            time: argb,
            date: argb,
            tray_glyph: argb,
            indicator_normal: (u32::from(normal_alpha) << 24) | rgb,
            indicator_selected: 0xFF00_0000 | rgb,
            mask: elem_mask::ALL,
        }
    }
}

/// 配色策略配置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolicyCfg {
    /// 亮度阈值（0..=1）：高于此值判为亮背景。
    pub threshold: f32,
    /// 双阈值迟滞半宽。
    pub hysteresis: f32,
    /// 暗背景使用的前景色。
    pub on_dark: Rgba,
    /// 亮背景使用的前景色。
    pub on_light: Rgba,
    /// 状态条正常态透明度。
    pub normal_alpha: u8,
    /// EWMA 平滑系数（越小越平滑，见 S-18）。
    pub ewma_alpha: f32,
    /// 强制端色（S-10）：0 = 自动判定，1 = 一律用亮端色，2 = 一律用暗端色。
    pub force: u8,
}

impl Default for PolicyCfg {
    fn default() -> Self {
        Self {
            threshold: 0.55,
            hysteresis: 0.06,
            // 暗背景用近白，亮背景用近黑；不用纯黑纯白，避免与 Fluent 观感割裂
            on_dark: Rgba::opaque(0xF2, 0xF2, 0xF2),
            on_light: Rgba::opaque(0x1A, 0x1A, 0x1A),
            normal_alpha: 0x72,
            ewma_alpha: 0.3,
            force: 0,
        }
    }
}

impl PolicyCfg {
    /// 由用户配置派生策略（S-10/S-12/S-13/S-14/S-15/S-18）。
    ///
    /// 判定始终是自动的；S-13/S-14/S-15 只影响"暗背景用哪种浅色 / 亮背景用哪种深色"，
    /// 不构成手动指定颜色的路径。
    #[must_use]
    pub fn from_config(c: &Config) -> Self {
        let mut p = Self::default();
        p.threshold = c.get(SettingKey::S12) as f32 / 1000.0;
        p.ewma_alpha = (c.get(SettingKey::S18) as f32 / 1000.0).clamp(0.05, 1.0);
        if c.get(SettingKey::S13) != 0 {
            p.on_dark = argb_to_rgba(c.get(SettingKey::S14));
            p.on_light = argb_to_rgba(c.get(SettingKey::S15));
        }
        // S-10：强制端色。用"直接返回端色"实现，而不是把阈值推到 0/1 ——
        // 阈值加减迟滞带后仍会落进区间，实测强制不彻底（v1.1 修）。
        p.force = match c.get(SettingKey::S10) {
            1 => 1,
            2 => 2,
            _ => 0,
        };
        p
    }
}

/// 带迟滞的亮度判定器（每个分区各持一个）。
#[derive(Debug, Clone, Copy)]
pub struct RegionJudge {
    /// 当前是否判为亮背景。
    pub is_light: bool,
    /// 平滑后的亮度。
    pub luma: f32,
}

impl Default for RegionJudge {
    fn default() -> Self {
        Self { is_light: false, luma: 0.0 }
    }
}

impl RegionJudge {
    /// 用新亮度更新判定（EWMA 平滑 + 双阈值迟滞）。
    pub fn update(&mut self, luma: f32, cfg: &PolicyCfg) {
        if !luma.is_finite() {
            return;
        }
        self.luma = if self.luma <= 0.0 { luma } else { self.luma + (luma - self.luma) * cfg.ewma_alpha };
        let hi = cfg.threshold + cfg.hysteresis;
        let lo = cfg.threshold - cfg.hysteresis;
        if self.luma >= hi {
            self.is_light = true;
        } else if self.luma <= lo {
            self.is_light = false;
        }
    }

    /// 本分区应使用的前景色（S-10 强制端色优先于判定结果）。
    #[must_use]
    pub fn foreground(&self, cfg: &PolicyCfg) -> Rgba {
        match cfg.force {
            1 => cfg.on_dark,
            2 => cfg.on_light,
            _ => {
                if self.is_light {
                    cfg.on_light
                } else {
                    cfg.on_dark
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 共享内存
// ---------------------------------------------------------------------------

/// 共享内存视图（宿主持有写端）。
pub struct Ipc {
    map: raw::Handle,
    view: *mut ShadeBarIpc,
}

// SAFETY: 视图指针指向进程内映射内存，仅本结构访问；跨线程使用由调用方保证不并发写。
unsafe impl Send for Ipc {}

impl Ipc {
    /// 创建（或打开已存在）共享内存并初始化。
    ///
    /// # Errors
    /// 映射失败时返回错误描述。
    pub fn create() -> Result<Self, String> {
        let name: Vec<u16> = IPC_NAME.encode_utf16().chain(std::iter::once(0)).collect();
        let size = std::mem::size_of::<ShadeBarIpc>();
        // SAFETY: 参数均为本进程合法值；文件句柄用 INVALID_HANDLE_VALUE 表示匿名页文件。
        let map = unsafe {
            raw::CreateFileMappingW(
                raw::INVALID_HANDLE_VALUE,
                std::ptr::null(),
                raw::PAGE_READWRITE,
                0,
                size as u32,
                name.as_ptr(),
            )
        };
        if map.is_null() {
            return Err(format!("CreateFileMappingW failed: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: 句柄有效，映射长度与创建时一致。
        let view = unsafe { raw::MapViewOfFile(map, raw::FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if view.is_null() {
            // SAFETY: 句柄有效。
            unsafe { raw::CloseHandle(map) };
            return Err(format!("MapViewOfFile failed: {}", std::io::Error::last_os_error()));
        }
        let this = Self { map, view: view.cast::<ShadeBarIpc>() };
        // SAFETY: 视图已按 ShadeBarIpc 大小映射；首次创建时清零初始化。
        unsafe {
            let p = this.view;
            if (*p).magic != IPC_MAGIC {
                std::ptr::write_bytes(p, 0, 1);
                (*p).magic = IPC_MAGIC;
                (*p).version = IPC_VERSION;
            } else {
                // 复用上一次会话的内存：必须清掉 shutdown，否则载荷会在启动瞬间
                // 又执行一次"宿主已退出"的还原流程（v1.1 实测：先还原再立刻被覆盖）
                (*p).shutdown = 0;
                (*p).enabled = 0;
            }
        }
        Ok(this)
    }

    /// 只读取载荷回报状态。
    #[must_use]
    pub fn status(&self) -> (i32, i32) {
        // SAFETY: 视图有效且布局已对齐。
        unsafe { ((*self.view).status, (*self.view).heartbeat) }
    }

    /// 下发配色（`enabled = 1`）。
    pub fn apply(&mut self, plan: &ColorPlan) {
        // SAFETY: 视图有效；写入为单字宽字段，载荷侧以 volatile 读。
        unsafe {
            let p = self.view;
            (*p).time_color = plan.time;
            (*p).date_color = plan.date;
            (*p).tray_glyph_color = plan.tray_glyph;
            (*p).indicator_normal_color = plan.indicator_normal;
            (*p).indicator_selected_color = plan.indicator_selected;
            // reserved[0] = 元素掩码，载荷按位决定改哪些、还原哪些
            (*p).reserved[0] = plan.mask;
            (*p).enabled = 1;
            (*p).seq = (*p).seq.wrapping_add(1);
        }
    }

    /// 要求载荷还原并退出。
    pub fn shutdown(&mut self) {
        // SAFETY: 同上。
        unsafe {
            let p = self.view;
            (*p).enabled = 0;
            (*p).seq = (*p).seq.wrapping_add(1);
            (*p).shutdown = 1;
        }
    }

    /// 宿主心跳：载荷据此判断宿主是否还活着（超时即自行还原退出）。
    pub fn beat(&mut self) {
        // SAFETY: 视图有效。
        unsafe {
            let p = self.view;
            (*p).heartbeat = (*p).heartbeat.wrapping_add(1);
        }
    }

    /// 仅还原（不退出载荷）。
    pub fn restore(&mut self) {
        // SAFETY: 同上。
        unsafe {
            let p = self.view;
            (*p).enabled = 0;
            (*p).reserved[0] = 0;
            (*p).seq = (*p).seq.wrapping_add(1);
        }
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        // SAFETY: 视图与句柄均由本结构创建。
        unsafe {
            raw::UnmapViewOfFile(self.view.cast::<c_void>());
            raw::CloseHandle(self.map);
        }
    }
}

// ---------------------------------------------------------------------------
// 注入
// ---------------------------------------------------------------------------

/// 找到宿主任务栏所属进程（即 explorer）。
#[must_use]
pub fn taskbar_pid() -> Option<u32> {
    let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: 传入合法 UTF-16 字符串指针。
    let hwnd = unsafe { raw::FindWindowW(class.as_ptr(), std::ptr::null()) };
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: hwnd 有效，pid 指针合法。
    unsafe { raw::GetWindowThreadProcessId(hwnd, &mut pid) };
    (pid != 0).then_some(pid)
}

/// 把载荷 DLL 注入指定进程。
///
/// # Errors
/// 打开进程 / 分配内存 / 创建远程线程失败时返回错误描述。
pub fn inject(dll_path: &std::path::Path) -> Result<u32, String> {
    let pid = taskbar_pid().ok_or_else(|| "找不到 Shell_TrayWnd".to_string())?;
    let wide: Vec<u16> = dll_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: 参数均为合法值；句柄在使用后关闭。
    unsafe {
        let proc = raw::OpenProcess(
            raw::PROCESS_CREATE_THREAD
                | raw::PROCESS_QUERY_INFORMATION
                | raw::PROCESS_VM_OPERATION
                | raw::PROCESS_VM_WRITE
                | raw::PROCESS_VM_READ,
            0,
            pid,
        );
        if proc.is_null() {
            return Err(format!("OpenProcess({pid}) failed: {}", std::io::Error::last_os_error()));
        }

        let bytes = wide.len() * 2;
        let remote = raw::VirtualAllocEx(
            proc,
            std::ptr::null_mut(),
            bytes,
            raw::MEM_COMMIT | raw::MEM_RESERVE,
            raw::PAGE_READWRITE,
        );
        if remote.is_null() {
            raw::CloseHandle(proc);
            return Err("VirtualAllocEx failed".to_string());
        }

        let ok = raw::WriteProcessMemory(proc, remote, wide.as_ptr().cast(), bytes, std::ptr::null_mut());
        if ok == 0 {
            raw::VirtualFreeEx(proc, remote, 0, raw::MEM_RELEASE);
            raw::CloseHandle(proc);
            return Err("WriteProcessMemory failed".to_string());
        }

        let kernel: Vec<u16> = "kernel32.dll".encode_utf16().chain(std::iter::once(0)).collect();
        let k32 = raw::GetModuleHandleW(kernel.as_ptr());
        let load = raw::GetProcAddress(k32, c"LoadLibraryW".as_ptr().cast());
        if load.is_null() {
            raw::VirtualFreeEx(proc, remote, 0, raw::MEM_RELEASE);
            raw::CloseHandle(proc);
            return Err("GetProcAddress(LoadLibraryW) failed".to_string());
        }

        type ThreadStart = unsafe extern "system" fn(*mut c_void) -> u32;
        let start: ThreadStart = std::mem::transmute(load);
        let th = raw::CreateRemoteThread(proc, std::ptr::null(), 0, Some(start), remote, 0, std::ptr::null_mut());
        if th.is_null() {
            raw::VirtualFreeEx(proc, remote, 0, raw::MEM_RELEASE);
            raw::CloseHandle(proc);
            return Err(format!("CreateRemoteThread failed: {}", std::io::Error::last_os_error()));
        }
        raw::WaitForSingleObject(th, 15_000);
        let mut code = 0u32;
        raw::GetExitCodeThread(th, &mut code);
        raw::CloseHandle(th);
        raw::VirtualFreeEx(proc, remote, 0, raw::MEM_RELEASE);
        raw::CloseHandle(proc);

        if code == 0 {
            return Err("远程 LoadLibraryW 失败（杀软拦截或架构不匹配）".to_string());
        }
        Ok(pid)
    }
}

// ---------------------------------------------------------------------------
// 亮度采样
// ---------------------------------------------------------------------------

/// 任务栏在屏幕上的矩形（含自动隐藏判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarRect {
    /// 左。
    pub left: i32,
    /// 上。
    pub top: i32,
    /// 右。
    pub right: i32,
    /// 下。
    pub bottom: i32,
}

impl TaskbarRect {
    /// 通过 `Shell_TrayWnd` 取主屏任务栏真实矩形（物理像素）。
    #[must_use]
    pub fn bottom_taskbar() -> Option<Self> {
        let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: 传入合法 UTF-16 字符串指针；失败时返回空指针。
        let hwnd = unsafe { raw::FindWindowW(class.as_ptr(), std::ptr::null()) };
        if hwnd.is_null() {
            return None;
        }
        let mut r = [0i32; 4];
        // SAFETY: hwnd 有效，r 为 4 个 i32（等价 RECT）。
        if unsafe { raw::GetWindowRect(hwnd, &mut r) } == 0 {
            return None;
        }
        let (left, top, right, bottom) = (r[0], r[1], r[2], r[3]);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self { left, top, right, bottom })
    }
}

/// 采样区微调（S-20 ~ S-22，仅作用于**图标区**那一段）。
///
/// 时钟与托盘两段必须贴住任务栏两端，位置固定；中间图标区才允许用户微调，
/// 用来避开某些壁纸上的强色块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionCfg {
    /// 图标区起点（占任务栏宽度百分比，0~50）。
    pub icons_start_pct: u8,
    /// 图标区宽度（占任务栏宽度百分比，10~100）。
    pub icons_width_pct: u8,
    /// 采样带相对任务栏顶边的垂直偏移（像素，0~45）。
    pub top_offset_px: u8,
}

impl Default for RegionCfg {
    fn default() -> Self {
        // 默认覆盖整条任务栏：中位数采样本身抗图标干扰，不必预先缩小范围
        Self { icons_start_pct: 0, icons_width_pct: 100, top_offset_px: 0 }
    }
}

impl RegionCfg {
    /// 由用户配置派生。
    #[must_use]
    pub fn from_config(c: &Config) -> Self {
        Self {
            icons_start_pct: c.get(SettingKey::S20) as u8,
            icons_width_pct: c.get(SettingKey::S21) as u8,
            top_offset_px: c.get(SettingKey::S22) as u8,
        }
    }

    /// 图标区水平范围（百分比）。
    #[must_use]
    fn icons_span(self) -> (f32, f32) {
        let start = (self.icons_start_pct as f32 / 100.0).clamp(0.0, 0.5);
        let width = (self.icons_width_pct as f32 / 100.0).clamp(0.1, 1.0);
        (start, (start + width).min(1.0))
    }
}

/// 三段亮度：时钟区 / 托盘区 / 图标区。
#[derive(Debug, Clone, Copy, Default)]
pub struct RegionLuma {
    /// 时钟与日期所在区（任务栏最右端文字）。
    pub clock: Option<f32>,
    /// 托盘图标区（时间左侧一段）。
    pub tray: Option<f32>,
    /// 任务栏运行图标区（中部）。
    pub icons: Option<f32>,
}

/// 常驻的分区采样器：**一次 BitBlt 抓条带，再从内存取点**。
///
/// 为什么不是直接 `GetPixel`：屏幕上的任务栏是 DWM 合成后的画面，
/// `GetPixel` 每次调用都要读回合成表面，按本产品的密度（每拍约 864 点、
/// 间隔 500 ms）常驻 CPU 会被顶到 1% 以上，而 NFR-03 要求 ≤0.1%。
/// 改成「一次抓到复用 DIB + 内存取值」后，单拍只剩一次 BitBlt。
///
/// 与 [`sample_regions`] 的像素口径**完全一致**（同样的行、同样的步长、同样取中位数），
/// 区别只在取值方式。保留旧实现供单测与回退对照。
pub struct RegionSampler {
    src: GdiScreenSource,
    buf: ScreenBuffer,
}

impl Default for RegionSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// 采样条带高度：够覆盖 3/5/7 三行加最大垂直偏移（S-22 上限 45 px）。
const BAND_HEIGHT_PX: u32 = 60;

impl RegionSampler {
    /// 新建采样器（DC 与 DIB 延迟到首次采样按实际尺寸创建）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            src: GdiScreenSource::new(),
            buf: ScreenBuffer::new(),
        }
    }

    /// 采集三段亮度。失败时返回全 `None`（调用方沿用上次判定）。
    pub fn sample(&mut self, region: &RegionCfg) -> RegionLuma {
        let Some(rect) = TaskbarRect::bottom_taskbar() else {
            return RegionLuma::default();
        };
        let width = (rect.right - rect.left).max(1) as u32;
        if self
            .src
            .grab_screen_region(rect.left, rect.top, width, BAND_HEIGHT_PX, &mut self.buf)
            .is_err()
        {
            return RegionLuma::default();
        }
        let bytes = self.buf.as_bytes();
        let dy = u32::from(region.top_offset_px);
        let rows = [3 + dy, 5 + dy, 7 + dy];

        // 行内按步长抽样后取三行中位数（与原实现同口径）。
        let measure = |x_ratio0: f32, x_ratio1: f32| -> Option<f32> {
            let x0 = (width as f32 * x_ratio0) as u32;
            let x1 = (width as f32 * x_ratio1) as u32;
            if x1 <= x0 {
                return None;
            }
            let step = ((x1 - x0) / 96).max(1);
            let mut row_vals: Vec<f32> = Vec::with_capacity(rows.len());
            for ry in rows {
                if ry >= BAND_HEIGHT_PX {
                    continue;
                }
                let mut sum = 0.0f64;
                let mut n = 0u32;
                let mut x = x0;
                while x < x1 {
                    let idx = ((ry * width + x) as usize).saturating_mul(4);
                    if let Some(px) = bytes.get(idx..idx + 3) {
                        let b = f32::from(px[0]);
                        let g = f32::from(px[1]);
                        let r = f32::from(px[2]);
                        sum += f64::from(0.2126 * r + 0.7152 * g + 0.0722 * b);
                        n += 1;
                    }
                    x += step;
                }
                if n > 0 {
                    row_vals.push((sum / f64::from(n) / 255.0) as f32);
                }
            }
            row_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            row_vals.get(row_vals.len() / 2).copied()
        };

        // 时钟贴最右、托盘紧随其左，图标区范围可调（S-20/S-21）。
        let clock = measure(0.93, 0.99);
        let tray = measure(0.84, 0.93);
        let (i0, i1) = region.icons_span();
        let icons = measure(i0, i1);
        RegionLuma { clock, tray, icons }
    }
}

// ---------------------------------------------------------------------------
// 服务
// ---------------------------------------------------------------------------

/// 配色是否分区独立判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum JudgeMode {
    /// 每个分区各自判定（默认）。
    PerRegion,
    /// 全任务栏统一判定。
    Global,
}

/// 运行时配置快照：UI 线程可随时热更新，采样线程每拍读取。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeCfg {
    /// 总开关（S-01）。关闭时下发还原，但线程保留以便随时恢复。
    pub enabled: bool,
    /// 是否跟随壁纸实时采样（S-11）。关闭则沿用上次判定色。
    pub follow: bool,
    /// 采样间隔（S-16）。
    pub interval_ms: u32,
    /// 分区独立判定 / 全局统一判定。
    pub mode: JudgeMode,
    /// 亮度阈值与端色（S-10/S-12/S-13/S-14/S-15/S-18）。
    pub policy: PolicyCfg,
    /// 采样区微调（S-20 ~ S-22）。
    pub region: RegionCfg,
    /// 元素开关掩码（S-30 ~ S-33）。
    pub mask: u32,
}

impl Default for RuntimeCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            follow: true,
            interval_ms: 500,
            mode: JudgeMode::PerRegion,
            policy: PolicyCfg::default(),
            region: RegionCfg::default(),
            mask: elem_mask::ALL,
        }
    }
}

impl RuntimeCfg {
    /// 由用户配置派生。这是"设置 → 配色服务"的唯一换算点。
    #[must_use]
    pub fn from_config(c: &Config) -> Self {
        let mut mask = 0u32;
        if c.get(SettingKey::S30) != 0 {
            mask |= elem_mask::TIME;
        }
        if c.get(SettingKey::S31) != 0 {
            mask |= elem_mask::DATE;
        }
        if c.get(SettingKey::S32) != 0 {
            mask |= elem_mask::TRAY;
        }
        if c.get(SettingKey::S33) != 0 {
            mask |= elem_mask::INDICATOR;
        }
        Self {
            enabled: c.get(SettingKey::S01) != 0,
            follow: c.get(SettingKey::S11) != 0,
            interval_ms: (c.get(SettingKey::S16) as u32).clamp(200, 5000),
            mode: JudgeMode::PerRegion,
            policy: PolicyCfg::from_config(c),
            region: RegionCfg::from_config(c),
            mask,
        }
    }
}

/// 配色服务：owns IPC + 采样线程 + 注入生命周期。
pub struct RecolorService {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    ipc: Option<Ipc>,
    /// 运行时配置（UI 线程写、采样线程读）。
    cfg: Arc<Mutex<RuntimeCfg>>,
}

impl RecolorService {
    /// 建立服务并启动线程。
    ///
    /// # Errors
    /// 共享内存创建失败时返回错误（此时不启动线程）。
    pub fn start(dll_path: std::path::PathBuf, cfg: RuntimeCfg) -> Result<Self, String> {
        let mut ipc = Ipc::create()?;
        // 初始下发"未启用"，等第一拍判定后再 apply
        ipc.restore();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let shared = Arc::new(Mutex::new(cfg));
        let cfg_thread = Arc::clone(&shared);

        let handle = std::thread::Builder::new()
            .name("shadebar-recolor".to_string())
            .spawn(move || {
                run_loop(&mut ipc, &dll_path, &cfg_thread, &stop_thread);
            })
            .map_err(|e| format!("spawn recolor thread failed: {e}"))?;

        Ok(Self { stop, handle: Some(handle), ipc: None, cfg: shared })
    }

    /// 热更新运行时配置（下一拍生效）。UI 线程调用，不阻塞。
    pub fn update(&self, cfg: RuntimeCfg) {
        if let Ok(mut g) = self.cfg.lock() {
            *g = cfg;
        }
    }

    /// 请求停止（线程内会把颜色还原并关闭载荷）。
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for RecolorService {
    fn drop(&mut self) {
        self.stop();
        let _ = self.ipc.take();
    }
}

fn run_loop(
    ipc: &mut Ipc,
    dll_path: &std::path::Path,
    shared: &Mutex<RuntimeCfg>,
    stop: &AtomicBool,
) {
    let mut j_clock = RegionJudge::default();
    let mut j_tray = RegionJudge::default();
    let mut j_icons = RegionJudge::default();
    let mut injected_pid = 0u32;
    let mut last_plan = ColorPlan::NONE;
    let mut ticks: u32 = 0;
    let mut sampler = RegionSampler::new();

    while !stop.load(Ordering::SeqCst) {
        let cfg = shared.lock().map(|g| *g).unwrap_or_default();

        // 0) 先打心跳：注入会阻塞到远程 LoadLibrary 返回，期间不能让载荷误判宿主已死
        ipc.beat();

        // 总开关关闭：下发还原，但保留线程（重新开启时无需重新注入，零卡顿）
        if !cfg.enabled {
            if last_plan != ColorPlan::NONE {
                ipc.restore();
                last_plan = ColorPlan::NONE;
            }
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }

        // 1) 保活注入：explorer 重启后 pid 会变，必须重新注入
        let cur = taskbar_pid().unwrap_or(0);
        if cur != 0 && cur != injected_pid {
            match inject(dll_path) {
                Ok(pid) => {
                    injected_pid = pid;
                    last_plan = ColorPlan::NONE;
                }
                Err(_) => {
                    injected_pid = 0;
                }
            }
        }

        // 2) 采样 + 判定（S-11 关闭时不再采样，沿用上次判定色）
        let luma = if cfg.follow { sampler.sample(&cfg.region) } else { RegionLuma::default() };
        if let Some(v) = luma.clock {
            j_clock.update(v, &cfg.policy);
        }
        if let Some(v) = luma.tray {
            j_tray.update(v, &cfg.policy);
        }
        if let Some(v) = luma.icons {
            j_icons.update(v, &cfg.policy);
        }

        let (fg_clock, fg_tray, fg_icons) = match cfg.mode {
            JudgeMode::PerRegion => (
                j_clock.foreground(&cfg.policy),
                j_tray.foreground(&cfg.policy),
                j_icons.foreground(&cfg.policy),
            ),
            JudgeMode::Global => {
                // 全局模式：三区平均后统一判定
                let avg = [luma.clock, luma.tray, luma.icons]
                    .into_iter()
                    .flatten()
                    .fold((0.0f32, 0u32), |(s, n), v| (s + v, n + 1));
                let mut j = RegionJudge::default();
                if avg.1 > 0 {
                    j.update(avg.0 / avg.1 as f32, &cfg.policy);
                }
                let fg = j.foreground(&cfg.policy);
                (fg, fg, fg)
            }
        };

        // 3) 组计划（时间/日期用时钟区，托盘字形用托盘区，状态条用图标区）
        let plan = ColorPlan {
            time: u32::from_be_bytes([fg_clock.a, fg_clock.r, fg_clock.g, fg_clock.b]),
            date: u32::from_be_bytes([fg_clock.a, fg_clock.r, fg_clock.g, fg_clock.b]),
            tray_glyph: u32::from_be_bytes([fg_tray.a, fg_tray.r, fg_tray.g, fg_tray.b]),
            indicator_normal: {
                let c = u32::from_be_bytes([fg_icons.a, fg_icons.r, fg_icons.g, fg_icons.b]);
                (u32::from(cfg.policy.normal_alpha) << 24) | (c & 0x00FF_FFFF)
            },
            indicator_selected: {
                let c = u32::from_be_bytes([fg_icons.a, fg_icons.r, fg_icons.g, fg_icons.b]);
                0xFF00_0000 | (c & 0x00FF_FFFF)
            },
            mask: cfg.mask,
        };

        // 4) 变化才下发，避免无谓唤醒载荷
        if plan != last_plan {
            ipc.apply(&plan);
            last_plan = plan;
        } else if ticks % 2 == 0 {
            // 周期重发：任务栏重建、或指示条的视觉状态把 Fill 改回系统色时，
            // 靠这次重发在 2 秒内收敛回目标色（实测选中态切换会覆盖一次）
            ipc.apply(&plan);
        }
        ticks = ticks.wrapping_add(1);

        // 5) 读载荷状态（顺带确认它还活着）
        let _ = ipc.status();

        let sleep_ms = if cfg.follow { cfg.interval_ms } else { 1000 };
        std::thread::sleep(Duration::from_millis(u64::from(sleep_ms)));
    }

    // 退出：先还原再关载荷
    ipc.restore();
    std::thread::sleep(Duration::from_millis(300));
    ipc.shutdown();
    std::thread::sleep(Duration::from_millis(300));
}

// ---------------------------------------------------------------------------
// 自检
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_layout_is_stable() {
        // 与 C 结构体保持 80 字节
        assert_eq!(std::mem::size_of::<ShadeBarIpc>(), 80);
        assert_eq!(std::mem::align_of::<ShadeBarIpc>(), 4);
    }

    #[test]
    fn hysteresis_prevents_flapping() {
        let cfg = PolicyCfg::default();
        let mut j = RegionJudge::default();
        // 亮背景
        for _ in 0..60 {
            j.update(0.9, &cfg);
        }
        assert!(j.is_light);
        // 略微变暗（未越过下阈值）不应翻转
        for _ in 0..30 {
            j.update(0.52, &cfg);
        }
        assert!(j.is_light, "0.52 仍在迟滞带内，不应翻回暗端");
        // 明显变暗才翻转
        for _ in 0..60 {
            j.update(0.2, &cfg);
        }
        assert!(!j.is_light);
    }

    #[test]
    fn plan_derives_indicator_from_foreground() {
        let fg = Rgba::opaque(0xF2, 0xF2, 0xF2);
        let p = ColorPlan::from_foreground(fg, 0x72);
        assert_eq!(p.time, 0xFFF2_F2F2);
        assert_eq!(p.indicator_normal, 0x72F2_F2F2);
        assert_eq!(p.indicator_selected, 0xFFF2_F2F2);
    }
}
