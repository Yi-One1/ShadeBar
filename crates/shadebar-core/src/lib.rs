//! 随影 ShadeBar 共享核心库。
//!
//! 本 crate **不含任何 UI、窗口、线程**，只提供可单元测试的纯逻辑与 FFI 小工具，
//! 被 `shadebar-app`（主进程）与 `shadebar-inject`（注入 DLL）共同依赖。
//!
//! # 模块划分（docs/02-architecture.md §4）
//!
//! | 模块 | 职责 | 归属任务 |
//! |---|---|---|
//! | [`error`] | 统一错误类型与其 i18n key / 技术描述 | T01.8 |
//! | [`win`] | 共享 FFI 小工具（`wide!`、`HResultExt`、字符串互转） | T01.8 |
//!
//! 后续任务将按 §4 的编号继续补充：
//! `config` / `config_store` / `state`（T01.2）、`color` / `stats`（T01.3）、
//! `judge`（T01.4）、`ipc`（T01.5）、`log` / `zipstore`（T01.6）、
//! `i18n`（T01.7）。
//!
//! # 全局约定（§11）
//!
//! * 所有尺寸单位：屏幕/采样链路用**物理像素**，UI 链路用 **dip**（§11.4）。
//! * 颜色内部统一 `straight alpha sRGB`，边界转换只发生在明确的边界模块（§11.5）。
//! * 不使用 `unwrap` / `expect` / `panic!`（测试代码除外，§11.2）。
//!
//! # T02 采样与判定引擎
//!
//! [`engine`] 是 L1 的核心（架构 §8.2）。为满足「无窗口、无线程、可单测」，
//! 它把全部平台能力抽成对象安全的 trait，几何 / 遮罩 / 直方图 / 判定 / 闭环调度
//! 都是纯逻辑；真实 `FindWindowW` / `BitBlt` / `IUIAutomation` 由 `shadebar-app`
//! 在 T04 注入。详见 [`engine`] 模块文档。

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod color;
pub mod engine;
pub mod error;
pub mod win;

// ---------------------------------------------------------------------------
// 测试用 GlobalAlloc 计数器（T02.2 判据①「600 次零堆分配」的证明工具）
//
// 只在 cfg(test) 生效，release 构建完全不链接这段代码。计数是 **线程局部** 的：
// cargo test 并行运行各测试时互不干扰；`const` 初始化的 TLS 不触发堆分配，
// 因此不会与分配器自身形成递归。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod test_alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
    }

    /// 转发到 `System` 并计数 `alloc` / `realloc` / `alloc_zeroed` 的分配器。
    pub struct CountingAllocator;

    // SAFETY: 每个方法都直接转发给 `System`，仅额外更新一个线程局部计数器；
    // 计数器的 `try_with` 在 TLS 未就绪时静默跳过，不调用分配器，故不会递归。
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
            // SAFETY: 参数原样转发，`System` 的契约由调用方保证。
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: 参数原样转发。
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
            // SAFETY: 参数原样转发。
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
            // SAFETY: 参数原样转发。
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    /// 当前线程的累计分配次数。
    pub fn count() -> u64 {
        ALLOC_COUNT.try_with(|c| c.get()).unwrap_or(0)
    }

    /// 归零当前线程的计数。
    pub fn reset() {
        let _ = ALLOC_COUNT.try_with(|c| c.set(0));
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::CountingAllocator = test_alloc::CountingAllocator;

pub use color::Rgba;
pub use error::{SbError, SbResult};

// ---------------------------------------------------------------------------
// 全局常量（§4 #10：模块导出、全局常量 —— 版本、管道名格式）
// ---------------------------------------------------------------------------

/// 版本号。**唯一来源是 workspace 的 `Cargo.toml`**，任何界面都不得硬编码
/// （FR-16 / A-16：文件名、关于页、exe 属性三处必须一致）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 产品中文名。
pub const PRODUCT_NAME_ZH: &str = "随影";

/// 产品英文名。
pub const PRODUCT_NAME_EN: &str = "ShadeBar";

/// 产品全名（中英并列），用于窗口标题、`关于` 页与 VERSIONINFO 的 `ProductName`。
pub const PRODUCT_NAME: &str = "随影 ShadeBar";

/// 清单与注册表项使用的稳定应用标识（与 `resources/app.manifest` 中一致）。
///
/// 用途：`RegisterApplicationRestart` 的 `pwzAppId`、诊断包环境信息。
pub const APP_ID: &str = "Yi-One1.ShadeBar";

/// 单实例互斥体名（§2 D12）。
///
/// 用 `Local\` 前缀而非 `Global\`：多用户会话之间互不影响，且不依赖
/// `SeCreateGlobalPrivilege`。
pub const SINGLE_INSTANCE_MUTEX: &str = r"Local\ShadeBar.SingleInstance";

/// 安装/便携数据目录名（`%LOCALAPPDATA%\ShadeBar\`）。
pub const DATA_DIR_NAME: &str = "ShadeBar";

/// 日志文件名格式前缀：`shadebar-YYYYMMDD.log`（FR-13）。
pub const LOG_FILE_PREFIX: &str = "shadebar-";

/// 进程间通信协议魔数 `"SHDB"`（§5.1 `IPC_MAGIC`）。
///
/// 放在 `lib.rs` 是因为主进程与注入 DLL 都要用它做帧校验，
/// `ipc` 模块（T01.5）会 re-export 本常量而不是另起一个。
pub const IPC_MAGIC: u32 = 0x5348_4442;

/// 设备前缀后缀分隔符：`\\.\pipe\`。
const PIPE_DEVICE_PREFIX: &str = r"\\.\pipe\";

/// 判定下发管道名（主进程为服务器，DLL 为客户端，§2 D6）。
///
/// 形如 `\\.\pipe\ShadeBar.12345`。用 PID 后缀避免多实例/多会话冲突。
pub fn decision_pipe_name(pid: u32) -> String {
    let mut s = String::with_capacity(PIPE_DEVICE_PREFIX.len() + 16);
    s.push_str(PIPE_DEVICE_PREFIX);
    s.push_str(DATA_DIR_NAME);
    s.push('.');
    // u32 的十进制表示最长 10 位，上面 reserve 了 16 字节，不会重分配。
    push_u32_decimal(&mut s, pid);
    s
}

/// 第二实例用于激活已有实例的管道名（§2 D12）。
///
/// 与判定管道**分开**，避免第二实例的激活帧干扰正在进行的判定下发。
pub const ACTIVATE_PIPE_NAME: &str = r"\\.\pipe\ShadeBar.activate";

/// 激活管道名的函数形态。§5.1 的接口签名写的是 `activate_pipe_name()`，
/// 这里保留该签名，值直接取自 [`ACTIVATE_PIPE_NAME`]，两者物理上不可能不一致。
#[must_use]
pub const fn activate_pipe_name() -> &'static str {
    ACTIVATE_PIPE_NAME
}

/// 把 `u32` 以十进制追加到字符串，避免 `format!` 的分配开销。
fn push_u32_decimal(out: &mut String, mut value: u32) {
    if value == 0 {
        out.push('0');
        return;
    }
    // u32 最大 4_294_967_295，十进制至多 10 位。
    let mut buf = [0u8; 10];
    let mut idx = buf.len();
    while value > 0 {
        idx -= 1;
        // SAFETY(逻辑安全，非 unsafe)：value % 10 < 10，必定落在 b'0'..=b'9'。
        buf[idx] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    for &b in &buf[idx..] {
        out.push(char::from(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_three_part_semver() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "VERSION 必须是 major.minor.patch：{VERSION}");
        for p in parts {
            assert!(p.parse::<u32>().is_ok(), "VERSION 段必须是数字：{p}");
        }
    }

    #[test]
    fn decision_pipe_name_has_pid_suffix() {
        assert_eq!(decision_pipe_name(0), r"\\.\pipe\ShadeBar.0");
        assert_eq!(decision_pipe_name(12345), r"\\.\pipe\ShadeBar.12345");
        assert_eq!(
            decision_pipe_name(u32::MAX),
            r"\\.\pipe\ShadeBar.4294967295"
        );
    }

    #[test]
    fn activate_pipe_differs_from_decision_pipe() {
        assert_ne!(ACTIVATE_PIPE_NAME, decision_pipe_name(1));
    }
}
