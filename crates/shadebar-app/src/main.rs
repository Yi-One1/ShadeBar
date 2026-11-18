//! 随影 ShadeBar —— 进程入口。
//!
//! # 当前进度
//!
//! | 部分 | 状态 |
//! |---|---|
//! | 构建骨架（T01.1） | 完成 |
//! | 渲染骨架 + 窗口（T03.1，B3 上屏方案） | 完成 |
//! | 采样与判定引擎（T02，core 纯逻辑） | 完成 |
//! | 真实 Win32/COM 适配 + 引擎接线（T04） | 完成 |
//! | 设置页 35 项 + 双语（T03.2~T03.6） | 完成 |
//! | 单实例 + 托盘外壳（T04.1） | 完成 |
//! | 注入 DLL / L2 / L3 着色（T05） | 挂起，待 A-29 人工验收 |
//!
//! # 两种产物形态
//!
//! * **正式发布版**（默认，`verify` 特性关闭）：双击 = 单实例常驻托盘 +
//!   真实采样线程 + 设置窗口；`evidence.rs` / `visual.rs` / `png.rs` 与
//!   `--verify` 分支**完全不编译**（需求基线 v1.4 §7 K-07 / NFR-01）。
//! * **取证档**（`--features verify`）：额外提供 `--verify fast|full|endpoints`，
//!   用于 A-02 / A-13 / A-14 / A-20 / EX-24 真机取证。取证代码保留在仓库中，
//!   A-29 与后续版本仍要使用，只是不进正式产物。
//!
//! ```text
//! shadebar-app.exe                       最终形态：单实例 + 托盘常驻
//! shadebar-app.exe --ui                  直接打开设置窗口（阶段验收用）
//! shadebar-app.exe --version             打印 产品名 v版本
//! shadebar-app.exe --help                打印用法
//! shadebar-app.exe --verify fast|full|endpoints [dir]   （仅取证档）
//! ```
//!
//! # 权限模型
//!
//! 全程普通权限（Medium IL），manifest `requestedExecutionLevel = asInvoker`，
//! **UAC = 0**；无「以管理员身份重启」入口（FR-20 / NFR-11）。
//!
//! # 环境纪律
//!
//! 采样链路只读：不注入 explorer、不重启 explorer、不动 TranslucentTB、
//! 不改注册表、不改壁纸、不动用户已打开的窗口。取证用的可视化窗口一律是本进程
//! **新建**的 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` 窗口，用完即销毁。

// 窗口子系统：双击时不弹控制台；stdout 仍可被管道捕获，`--version` 在脚本里可用。
#![windows_subsystem = "windows"]

mod app;
mod autostart;
mod config;
mod engine;
mod ipc_server;
mod recolor;
mod render;
mod tray;
mod ui;
mod window;

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

#[cfg(feature = "verify")]
use std::path::PathBuf;

use shadebar_core::{PRODUCT_NAME, VERSION};

use crate::config::SettingKey;

/// 正常退出。
const EXIT_OK: u8 = 0;
/// 命令行参数无法识别。
const EXIT_USAGE: u8 = 2;
/// 运行时失败（窗口/渲染初始化失败）。
const EXIT_RUNTIME: u8 = 3;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);

    match args.next() {
        // 无参数 = 最终形态：单实例 + 托盘常驻 + 真实采样线程。
        None => run_app(),
        Some(raw) => match raw.to_str() {
            Some("--version" | "-V") => print_version(),
            Some("--help" | "-h" | "/?") => print_usage(),
            Some("--ui") => run_ui(),
            #[cfg(feature = "verify")]
            Some("--verify") => run_verify(&mut args),
            // 非 UTF-8 参数或未知参数一律按用法错误处理，不做猜测式降级。
            _ => ExitCode::from(EXIT_USAGE),
        },
    }
}

/// 正式启动：单实例守卫 → 配置 → 主窗口（隐藏）+ 托盘 → 采样线程 → 消息循环。
///
/// 第 2~5 次双击会在 [`ipc_server::activate_existing`] 里向既有窗口投递激活消息，
/// 然后立即退出（≤1 s），进程数恒为 1（A-01 / FR-01）。
fn run_app() -> ExitCode {
    let Some(_guard) = ipc_server::acquire_single_instance() else {
        let _ = ipc_server::activate_existing();
        return ExitCode::from(EXIT_OK);
    };

    let app = app::App::new();
    let start_minimized = app.config().get(SettingKey::S03) != 0;
    let hwnd = match window::create_main_window(Arc::clone(&app)) {
        Ok(h) => h,
        Err(e) => {
            let mut err = std::io::stderr();
            let _ = writeln!(err, "window init failed: {e}");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    app.set_hwnd(hwnd);
    // 开机自启项以配置为准修正一次：exe 被移动过、被任务管理器禁用过都能拉回
    if crate::autostart::reconcile(app.config().get(SettingKey::S02) != 0) {
        config::note("autostart registry entry reconciled");
    }
    app.start_sampling();
    app.start_recolor();
    let code = window::run_message_loop(hwnd, !start_minimized);
    app.stop_recolor();
    app.request_exit();
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// 开发入口：直接打开设置窗口（T03 阶段验收用），同样跑真实采样线程。
fn run_ui() -> ExitCode {
    let app = app::App::new();
    let hwnd = match window::create_main_window(Arc::clone(&app)) {
        Ok(h) => h,
        Err(e) => {
            let mut err = std::io::stderr();
            let _ = writeln!(err, "window init failed: {e}");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    app.set_hwnd(hwnd);
    app.start_sampling();
    app.start_recolor();
    let code = window::run_message_loop(hwnd, true);
    app.stop_recolor();
    app.request_exit();
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// 真机取证入口（**仅 `verify` 特性下编译**）：`--verify fast|full|endpoints [output_dir]`。
///
/// `endpoints` 档只跑 A-13 / A-14，各起独立 `EngineSession`、各 60 s，
/// 不跑 A-02 / EX-24 / 10 分钟 CPU 段（需求基线 v1.4 §9.1 表后注）。
/// 输出目录默认为 `evidence/t04`。
#[cfg(feature = "verify")]
fn run_verify(args: &mut impl Iterator<Item = std::ffi::OsString>) -> ExitCode {
    let profile = match args.next() {
        Some(p) => match p.to_str() {
            Some("fast") => engine::evidence::VerifyProfile::Fast,
            Some("full") => engine::evidence::VerifyProfile::Full,
            Some("endpoints") => engine::evidence::VerifyProfile::Endpoints,
            _ => return ExitCode::from(EXIT_USAGE),
        },
        None => return ExitCode::from(EXIT_USAGE),
    };
    let out_dir = match args.next() {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from("evidence/t04"),
    };
    let opts = engine::evidence::VerifyOptions { out_dir, profile };
    match engine::evidence::run(&opts) {
        Ok(report) => {
            let mut out = std::io::stdout();
            let obtained = report.items.iter().filter(|i| i.obtained).count();
            let _ = writeln!(
                out,
                "T04 verify done: {obtained}/{} obtained; summary={}",
                report.items.len(),
                report.summary_path.display()
            );
            ExitCode::from(EXIT_OK)
        }
        Err(e) => {
            let mut err = std::io::stderr();
            let _ = writeln!(err, "verify failed: {e}");
            ExitCode::from(EXIT_RUNTIME)
        }
    }
}

/// 打印 `产品名 v版本`，用于 A-16「三处版本号一致」的自动化核对。
fn print_version() -> ExitCode {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{PRODUCT_NAME} v{VERSION}");
    let _ = out.flush();
    ExitCode::from(EXIT_OK)
}

/// 打印命令行用法。
fn print_usage() -> ExitCode {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{PRODUCT_NAME} v{VERSION}");
    let _ = writeln!(out, "Usage:");
    let _ = writeln!(out, "  --version, -V              print version and exit");
    let _ = writeln!(out, "  --ui                       open settings window (stage check)");
    let _ = writeln!(out, "  --verify fast|full|endpoints [dir]   evidence capture (verify build only)");
    let _ = writeln!(out, "       fast = 5s+5s+20s smoke; full = 60s+60s+600s acceptance");
    let _ = writeln!(out, "       endpoints = A-13/A-14 only, independent sessions, no 10min CPU");
    let _ = writeln!(out, "  (no args)                  single instance + tray resident");
    let _ = out.flush();
    ExitCode::from(EXIT_OK)
}
