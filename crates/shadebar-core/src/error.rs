//! 统一错误类型（§4 #11 / §11.2）。
//!
//! 设计约束（来自 §11.2）：
//!
//! 1. 全项目统一 `SbResult<T> = Result<T, SbError>`，不使用 `anyhow` / `thiserror`
//!    （体积原因，见 §9.1 禁止清单）。
//! 2. 每个变体带两样东西：
//!    * [`SbError::i18n_key`] —— 面向用户的文案键，交给 `i18n::t()`（T01.7）；
//!    * `Display` —— 英文技术描述，**只进日志不翻译**，且必须保留原始错误码
//!      （Win32 `GetLastError` 值 / `HRESULT`）。
//! 3. "可降级错误"（注入、UIA、主题联动）在这里只负责携带信息，
//!    *是否降级* 由调用点决定：调用点必须显式处理为降级 + 日志，不得向上传播成崩溃。

use std::fmt;

/// 项目统一结果类型。
pub type SbResult<T> = Result<T, SbError>;

/// 随影 ShadeBar 统一错误类型。
///
/// 变体按"失败发生的位置"划分，便于日志用固定词干 grep（§11.1）：
/// `config:` / `ipc:` / `inject:` / `locate:` / `hook:` / `sample:` …
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SbError {
    /// Win32 API 直接失败。`op` 是固定词干（如 `"OpenProcess"`），`code` 是 `GetLastError` 原值。
    Win32 {
        /// 失败的操作名（固定词干，可 grep）。
        op: &'static str,
        /// `GetLastError()` 原始值。
        code: u32,
    },

    /// COM / WinRT 调用返回失败 HRESULT。
    HResult {
        /// 失败的操作名。
        op: &'static str,
        /// 原始 HRESULT（`i32`，保留符号位便于对照 `winerror.h`）。
        hr: i32,
    },

    /// 配置 JSON 解析失败（NFR-16：应回退默认值并备份损坏文件，而不是崩溃）。
    ConfigParse {
        /// 解析器给出的位置信息或原因。
        detail: String,
    },

    /// 配置序列化失败（极少发生，通常是磁盘满或字段含非法 UTF-8）。
    ConfigSerialize {
        /// 失败原因。
        detail: String,
    },

    /// 目标目录不可写，需要回退到 `%LOCALAPPDATA%`（FR-10 / EX-19）。
    PathNotWritable {
        /// 探测失败的目录。
        path: String,
    },

    /// 日志系统初始化失败。
    LogInit {
        /// `GetLastError()` 原始值。
        code: u32,
    },

    /// IPC 帧头非法：魔数 / 版本 / 长度 / 校验和不符（§5.1 `Frame::decode`）。
    IpcBadFrame {
        /// 判定为非法的具体字段名。
        field: &'static str,
    },

    /// 管道对端已断开。
    ///
    /// 对注入 DLL 而言这是**正常且唯一可靠**的"主进程已死"信号（§2 D6），
    /// 收到后必须执行还原并停机，而不是当作异常重试。
    IpcDisconnected,

    /// IPC 读写超时（例如 `Decision` 帧未在 deadline 内送达）。
    IpcTimeout {
        /// 等待的毫秒数。
        deadline_ms: u32,
    },

    /// 当前进程不是管理员，L2/L3 能力不可用 → 按 NFR-05 降级为 L1。
    NotElevated,

    /// 找不到持有 `Shell_TrayWnd` 的 explorer.exe（FR-02 / FR-24）。
    TaskbarNotFound,

    /// 注入 DLL 未落盘或 SHA-256 与内嵌副本不符（§2 D2）。
    InjectDllMissing {
        /// 期望路径与实际哈希等细节。
        detail: String,
    },

    /// 远程注入某一阶段失败（`OpenProcess` / `VirtualAllocEx` / `WriteProcessMemory` /
    /// `CreateRemoteThread`）。
    ///
    /// `stage` 用固定词干，便于日志统计"卡在哪一步"（B-07 的 5 种异常构造）。
    InjectRemoteFailed {
        /// 阶段名。
        stage: &'static str,
        /// 该阶段返回的 `GetLastError()`。
        code: u32,
    },

    /// 注入后管道握手超时。
    InjectHandshakeTimeout {
        /// 等待的毫秒数。
        deadline_ms: u32,
    },

    /// 元素定位链某一档失败（C1 岛根表 / C2 模板应用点 / C3 任务栏窗口关联）。
    LocateFailed {
        /// 档位标识，如 `"C1"`。
        chain: &'static str,
    },

    /// inline hook 安装失败。
    ///
    /// **这是"安全放弃"而非异常**：指令长度解码覆盖不到该字节序列时必须放弃
    /// 并尝试下一个候选点，绝不允许半装状态（§2 D3 / FR-26）。
    HookInstallFailed {
        /// 放弃原因。
        reason: &'static str,
    },

    /// 在非 XAML 线程写 XAML 属性。
    ///
    /// 一旦出现说明 `DispatcherHandle::try_enqueue` 路径被绕过，
    /// 属于**必须修复的编程错误**（§2 D5 / §11.3），而不是运行时环境问题。
    XamlWrongThread,

    /// 采样区有效像素不足。
    ///
    /// 阈值 1500 由主理人拍板（§13-5 → 已拍板 #5）：此时跳过本次采样、
    /// 保持上次判定、只写 Debug 日志，**不报错、不弹窗**。
    SampleRegionTooSmall {
        /// 遮罩后参与统计的像素数。
        valid_px: u32,
    },

    /// 抓屏失败（`BitBlt` 返回 0 / 采样区被完全裁剪到屏幕外）。
    BitBltFailed {
        /// `GetLastError()` 原始值。
        code: u32,
    },

    /// UI Automation 不可用或被超时熔断。
    ///
    /// 此时退回固定几何 `fallback_clock_rects`，功能不降级（T02.3）。
    UiaUnavailable {
        /// 原始 HRESULT。
        hr: i32,
    },

    /// 计划任务注册 / 查询 / 删除失败（FR-05）。
    AutostartFailed {
        /// 操作名，如 `"schtasks /create"`。
        op: &'static str,
        /// 进程退出码或 `GetLastError()`。
        code: u32,
    },

    /// 主题联动写注册表失败（S-19）。
    ThemeModeFailed {
        /// 操作名，如 `"RegSetValueExW"`。
        op: &'static str,
        /// `GetLastError()` 原始值。
        code: u32,
    },

    /// 写出无压缩 ZIP 失败（诊断包，FR-13）。
    ZipWriteFailed {
        /// 失败原因。
        detail: String,
    },

    /// 当前环境不支持该能力（非 Win11 / 内部结构不匹配），走明确降级而不是崩溃
    /// （EX-12）。
    Unsupported {
        /// 不支持的能力名。
        what: &'static str,
    },
}

impl SbError {
    /// 面向用户的文案键，交给 `i18n::t()` 翻译（T01.7 / FR-18）。
    ///
    /// 键命名遵循 `err.<domain>.<what>`，与 `resources/i18n/*.tsv` 一一对应。
    /// 返回 `&'static str` 而非 `String`：错误对象在热路径上创建，不能分配。
    #[must_use]
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Win32 { .. } => "err.win32",
            Self::HResult { .. } => "err.hresult",
            Self::ConfigParse { .. } => "err.config.parse",
            Self::ConfigSerialize { .. } => "err.config.serialize",
            Self::PathNotWritable { .. } => "err.config.path_not_writable",
            Self::LogInit { .. } => "err.log.init",
            Self::IpcBadFrame { .. } => "err.ipc.bad_frame",
            Self::IpcDisconnected => "err.ipc.disconnected",
            Self::IpcTimeout { .. } => "err.ipc.timeout",
            Self::NotElevated => "err.inject.not_elevated",
            Self::TaskbarNotFound => "err.taskbar.not_found",
            Self::InjectDllMissing { .. } => "err.inject.dll_missing",
            Self::InjectRemoteFailed { .. } => "err.inject.remote_failed",
            Self::InjectHandshakeTimeout { .. } => "err.inject.handshake_timeout",
            Self::LocateFailed { .. } => "err.locate.failed",
            Self::HookInstallFailed { .. } => "err.hook.install_failed",
            Self::XamlWrongThread => "err.xaml.wrong_thread",
            Self::SampleRegionTooSmall { .. } => "err.sample.region_too_small",
            Self::BitBltFailed { .. } => "err.sample.bitblt_failed",
            Self::UiaUnavailable { .. } => "err.uia.unavailable",
            Self::AutostartFailed { .. } => "err.autostart.failed",
            Self::ThemeModeFailed { .. } => "err.thememode.failed",
            Self::ZipWriteFailed { .. } => "err.zip.write_failed",
            Self::Unsupported { .. } => "err.unsupported",
        }
    }

    /// 日志用固定词干（§11.1 的 `inject:` / `sample:` 等前缀）。
    ///
    /// 与 `i18n_key()` 分开是刻意的：日志永不被翻译，否则 grep 与工单对照会漂移。
    #[must_use]
    pub fn log_stem(&self) -> &'static str {
        match self {
            Self::Win32 { .. } | Self::HResult { .. } => "win32",
            Self::ConfigParse { .. } | Self::ConfigSerialize { .. } => "config",
            Self::PathNotWritable { .. } => "config",
            Self::LogInit { .. } => "log",
            Self::IpcBadFrame { .. } | Self::IpcDisconnected | Self::IpcTimeout { .. } => "ipc",
            Self::NotElevated
            | Self::InjectDllMissing { .. }
            | Self::InjectRemoteFailed { .. }
            | Self::InjectHandshakeTimeout { .. } => "inject",
            Self::TaskbarNotFound => "taskbar",
            Self::LocateFailed { .. } => "locate",
            Self::HookInstallFailed { .. } => "hook",
            Self::XamlWrongThread => "xaml",
            Self::SampleRegionTooSmall { .. } | Self::BitBltFailed { .. } => "sample",
            Self::UiaUnavailable { .. } => "uia",
            Self::AutostartFailed { .. } => "autostart",
            Self::ThemeModeFailed { .. } => "thememode",
            Self::ZipWriteFailed { .. } => "diag",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

impl fmt::Display for SbError {
    /// 英文技术描述，**只进日志**。原始错误码必须出现，否则无法定位
    /// （§11.2：`HRESULT` / `Win32 error` 必须保留原始数值进日志）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Win32 { op, code } => write!(f, "{op} failed, GetLastError={code}"),
            Self::HResult { op, hr } => write!(f, "{op} failed, hr=0x{:08X}", *hr as u32),
            Self::ConfigParse { detail } => write!(f, "config parse failed: {detail}"),
            Self::ConfigSerialize { detail } => write!(f, "config serialize failed: {detail}"),
            Self::PathNotWritable { path } => write!(f, "path not writable: {path}"),
            Self::LogInit { code } => write!(f, "log init failed, GetLastError={code}"),
            Self::IpcBadFrame { field } => write!(f, "ipc frame rejected: bad {field}"),
            Self::IpcDisconnected => f.write_str("ipc peer disconnected"),
            Self::IpcTimeout { deadline_ms } => {
                write!(f, "ipc timeout after {deadline_ms} ms")
            }
            Self::NotElevated => f.write_str("process is not elevated"),
            Self::TaskbarNotFound => f.write_str("no explorer.exe owns Shell_TrayWnd"),
            Self::InjectDllMissing { detail } => write!(f, "inject dll unavailable: {detail}"),
            Self::InjectRemoteFailed { stage, code } => {
                write!(f, "remote inject failed at {stage}, GetLastError={code}")
            }
            Self::InjectHandshakeTimeout { deadline_ms } => {
                write!(f, "inject handshake timeout after {deadline_ms} ms")
            }
            Self::LocateFailed { chain } => write!(f, "locate chain {chain} failed"),
            Self::HookInstallFailed { reason } => write!(f, "hook install aborted: {reason}"),
            Self::XamlWrongThread => f.write_str("xaml write attempted off the xaml thread"),
            Self::SampleRegionTooSmall { valid_px } => {
                write!(f, "sample region too small, valid_px={valid_px}")
            }
            Self::BitBltFailed { code } => write!(f, "BitBlt failed, GetLastError={code}"),
            Self::UiaUnavailable { hr } => {
                write!(f, "ui automation unavailable, hr=0x{:08X}", *hr as u32)
            }
            Self::AutostartFailed { op, code } => {
                write!(f, "autostart {op} failed, code={code}")
            }
            Self::ThemeModeFailed { op, code } => {
                write!(f, "theme mode {op} failed, GetLastError={code}")
            }
            Self::ZipWriteFailed { detail } => write!(f, "zip write failed: {detail}"),
            Self::Unsupported { what } => write!(f, "unsupported on this system: {what}"),
        }
    }
}

impl std::error::Error for SbError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个变体都必须同时有 i18n key 与日志词干，且不为空。
    fn all_variants() -> Vec<SbError> {
        vec![
            SbError::Win32 {
                op: "OpenProcess",
                code: 5,
            },
            SbError::HResult {
                op: "CoCreateInstance",
                hr: -2_147_467_247,
            },
            SbError::ConfigParse {
                detail: "eof".into(),
            },
            SbError::ConfigSerialize {
                detail: "io".into(),
            },
            SbError::PathNotWritable {
                path: r"C:\Program Files\ShadeBar".into(),
            },
            SbError::LogInit { code: 5 },
            SbError::IpcBadFrame { field: "magic" },
            SbError::IpcDisconnected,
            SbError::IpcTimeout { deadline_ms: 1000 },
            SbError::NotElevated,
            SbError::TaskbarNotFound,
            SbError::InjectDllMissing {
                detail: "hash mismatch".into(),
            },
            SbError::InjectRemoteFailed {
                stage: "CreateRemoteThread",
                code: 5,
            },
            SbError::InjectHandshakeTimeout { deadline_ms: 1000 },
            SbError::LocateFailed { chain: "C1" },
            SbError::HookInstallFailed {
                reason: "undecodable insn",
            },
            SbError::XamlWrongThread,
            SbError::SampleRegionTooSmall { valid_px: 900 },
            SbError::BitBltFailed { code: 87 },
            SbError::UiaUnavailable { hr: -2_147_467_247 },
            SbError::AutostartFailed {
                op: "schtasks /create",
                code: 1,
            },
            SbError::ThemeModeFailed {
                op: "RegSetValueExW",
                code: 5,
            },
            SbError::ZipWriteFailed {
                detail: "disk full".into(),
            },
            SbError::Unsupported {
                what: "DWMWA_SYSTEMBACKDROP_TYPE",
            },
        ]
    }

    #[test]
    fn every_variant_has_i18n_key_and_log_stem() {
        for e in all_variants() {
            assert!(e.i18n_key().starts_with("err."), "bad key: {}", e.i18n_key());
            assert!(!e.log_stem().is_empty());
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn technical_description_keeps_raw_codes() {
        let e = SbError::Win32 {
            op: "BitBlt",
            code: 87,
        };
        assert!(e.to_string().contains("87"), "丢掉了原始错误码：{e}");
        assert!(e.to_string().contains("BitBlt"));

        let h = SbError::HResult {
            op: "RoInitialize",
            hr: 0x8004_0102u32 as i32,
        };
        assert!(h.to_string().contains("80040102"), "HRESULT 必须可读：{h}");
    }

    #[test]
    fn i18n_keys_are_unique_per_variant_family() {
        // 同一个 key 允许映射多个变体（如所有 Win32 失败共用一条用户文案），
        // 但日志词干必须能把它们区分到正确的域。
        let e = SbError::NotElevated;
        assert_eq!(e.i18n_key(), "err.inject.not_elevated");
        assert_eq!(e.log_stem(), "inject");
    }
}
