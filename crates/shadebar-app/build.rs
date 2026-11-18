//! 构建脚本：把 `resources/app.rc` 编译成 `.res` 并交给链接器。
//!
//! 为什么自己调 `rc.exe` 而不是用 `embed-resource` crate：
//! `embed-resource` 是纯构建期依赖，理论上不进产物，但它会额外引入 5 个
//! 传递依赖与一层不必要的编译；本项目只需要"跑一次 rc.exe"这一件事，
//! 自写 40 行即可，且能精确控制 `/I`（生成的头文件在 `OUT_DIR`）。
//!
//! 版本号只有 `Cargo.toml` 一个来源：本脚本用 `CARGO_PKG_VERSION` 生成
//! `OUT_DIR/sb_version.h`，`app.rc` 通过宏引用它。故
//! **文件名 / 关于页 / exe 属性三处版本不可能漂移**（FR-16 / A-16）。
//!
//! # 为什么这里允许 `panic!`
//!
//! §11.2 禁止运行时代码 panic；构建脚本是例外：cargo 把 build.rs 的 panic
//! 转成"构建失败 + 完整错误输出"，这正是构建期的正确错误机制（返回 `Result`
//! 只会得到一句 `build script failed`，反而更难排查）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 资源与头文件的文件名常量（两处引用，避免拼写漂移）。
const RC_FILE_NAME: &str = "app.rc";
const RES_FILE_NAME: &str = "app.res";
const VERSION_HEADER_NAME: &str = "sb_version.h";

fn main() {
    let manifest_dir = PathBuf::from(env_or_panic("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env_or_panic("OUT_DIR"));
    let version = env_or_panic("CARGO_PKG_VERSION");

    // crates/shadebar-app → crates → workspace 根
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| {
            panic!(
                "无法从 CARGO_MANIFEST_DIR 推导 workspace 根：{}",
                manifest_dir.display()
            )
        });
    let resources_dir = workspace_root.join("resources");
    let rc_path = resources_dir.join(RC_FILE_NAME);
    if !rc_path.is_file() {
        panic!("找不到资源脚本：{}（workspace 结构见 docs/02-architecture.md §4）", rc_path.display());
    }

    // 1) 生成版本头文件（纯 ASCII + 中文串，UTF-8 + code_page 65001）
    let header = render_version_header(&version);
    let header_path = out_dir.join(VERSION_HEADER_NAME);
    std::fs::write(&header_path, header.as_bytes()).unwrap_or_else(|e| {
        panic!("写出 {} 失败：{e}", header_path.display());
    });

    // 2) 定位 rc.exe
    let rc_exe = find_rc_exe().unwrap_or_else(|| {
        panic!(
            "找不到 rc.exe。请任选一种方式修复：\n\
             1) 安装 \"Windows 11 SDK\"（通过 Visual Studio Installer 勾选）；\n\
             2) 设置环境变量 SB_RC_EXE 指向 rc.exe 的绝对路径；\n\
             3) 在 \"x64 Native Tools Command Prompt\" 中构建（rc.exe 会在 PATH 上）。"
        )
    });

    // 3) 编译资源。工作目录设为 resources/，这样 app.rc 里的相对路径
    //    （app.ico / app.manifest）无需改写即可解析。
    let res_path = out_dir.join(RES_FILE_NAME);
    let output = Command::new(&rc_exe)
        .current_dir(&resources_dir)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg("/I")
        .arg(&out_dir)
        .arg(RC_FILE_NAME)
        .output()
        .unwrap_or_else(|e| panic!("启动 {} 失败：{e}", rc_exe.display()));

    if !output.status.success() {
        panic!(
            "rc.exe 编译 {} 失败（exit={:?}）\n--- stdout ---\n{}\n--- stderr ---\n{}",
            rc_path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    if !res_path.is_file() {
        panic!("rc.exe 报告成功但未产出 {}", res_path.display());
    }

    // 4) 把 .res 作为链接输入交给链接器（.res 是合法的 COFF 输入）
    println!("cargo:rustc-link-arg={}", res_path.display());

    // 5) 载荷指纹：内容一变，文件名就变
    //
    // explorer 里若已加载同名 DLL，`LoadLibraryW` 不会再执行 `DllMain`，
    // 于是"改了代码行为却没变"。把内容哈希放进文件名，升级与本地迭代都不必
    // 重启 explorer（v1.1 实测踩坑）。
    let payload = workspace_root.join("payload").join("build").join("ShadeBarTap.dll");
    if payload.is_file() {
        let bytes = std::fs::read(&payload).unwrap_or_else(|e| {
            panic!("读取载荷 {} 失败：{e}", payload.display());
        });
        // FNV-1a 64：够区分构建产物，且零依赖
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in &bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        println!("cargo:rustc-env=SB_PAYLOAD_TAG={:08x}", hash & 0xFFFF_FFFF);
        println!("cargo:rustc-env=SB_PAYLOAD_LEN={}", bytes.len());
        println!("cargo:rerun-if-changed={}", payload.display());
    } else {
        panic!("缺少载荷 DLL：{}（先运行 payload/build.sh）", payload.display());
    }

    // 6) 增量构建依赖
    println!("cargo:rerun-if-changed={}", rc_path.display());
    for name in ["app.ico", "app.manifest"] {
        println!("cargo:rerun-if-changed={}", resources_dir.join(name).display());
    }
    println!("cargo:rerun-if-env-changed=SB_RC_EXE");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
}

/// 读取必需的环境变量，缺失时给出可执行的提示。
fn env_or_panic(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("构建期缺少环境变量 {key}（应由 cargo 注入）"))
}

/// 把 `1.0.0` 渲染成 `sb_version.h`。
///
/// 生成两份形态：`FILEVERSION` 需要的逗号数字序列，与字符串资源需要的四点版本号。
fn render_version_header(version: &str) -> String {
    let csv = version_csv(version);
    let quad = version_quad(version);
    let original_filename = format!("随影_ShadeBar_v{version}.exe");

    // 说明：VALUE 宏的值必须是带引号的窄字符串；中文在此处 UTF-8 +
    // code_page(65001) 下由 rc.exe 正确转成 UTF-16 写入资源。
    let mut s = String::new();
    s.push_str("// 由 crates/shadebar-app/build.rs 自动生成，请勿手工编辑。\n");
    s.push_str("// 版本号来源：Cargo.toml 的 [workspace.package] version\n");
    s.push_str("#pragma code_page(65001)\n\n");
    s.push_str("#ifndef SB_VERSION_H\n#define SB_VERSION_H\n\n");
    s.push_str(&format!("#define SB_VERSION_CSV {csv}\n"));
    s.push_str(&format!("#define SB_VERSION_STR \"{quad}\"\n"));
    s.push_str("#define SB_COMPANY_NAME \"Yi-One1\"\n");
    s.push_str("#define SB_FILE_DESCRIPTION \"随影 ShadeBar - 任务栏文字自适应着色\"\n");
    s.push_str("#define SB_INTERNAL_NAME \"shadebar-app\"\n");
    s.push_str("#define SB_LEGAL_COPYRIGHT \"MIT License\"\n");
    s.push_str("#define SB_PRODUCT_NAME \"随影 ShadeBar\"\n");
    s.push_str(&format!(
        "#define SB_ORIGINAL_FILENAME \"{original_filename}\"\n"
    ));
    s.push_str("\n#endif // SB_VERSION_H\n");
    s
}

/// `1.0.0` → `1,0,0,0`（VERSIONINFO 的 `FILEVERSION` 固定四段）。
fn version_csv(version: &str) -> String {
    let parts = numeric_parts(version);
    format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3])
}

/// `1.0.0` → `1.0.0.0`（资源字符串里的 `FileVersion`）。
fn version_quad(version: &str) -> String {
    let p = numeric_parts(version);
    format!("{}.{}.{}.{}", p[0], p[1], p[2], p[3])
}

/// 解析出四段数字；预发布后缀（`-rc.1`）被忽略，缺失段补 0。
///
/// 这样 `1.0.0-rc.1` 的 exe 属性仍显示 `1.0.0.0`，与 SemVer 主三元组一致。
fn numeric_parts(version: &str) -> [u32; 4] {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut out = [0u32; 4];
    for (i, seg) in core.split('.').take(4).enumerate() {
        out[i] = seg.parse::<u32>().unwrap_or_else(|_| {
            panic!("Cargo.toml 的 version 段 `{seg}` 不是数字：{version}")
        });
    }
    out
}

/// 定位 `rc.exe`：环境变量 → PATH → Windows SDK → MSVC。
fn find_rc_exe() -> Option<PathBuf> {
    // 1) 显式覆盖，便于 CI 或特殊 SDK 布局
    if let Ok(explicit) = std::env::var("SB_RC_EXE") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2) PATH
    if let Some(found) = which("rc.exe") {
        return Some(found);
    }

    // 3) Windows SDK：%ProgramFiles(x86)%\Windows Kits\10\bin\<ver>\x64\rc.exe
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(pf) = std::env::var(key) {
            let base = PathBuf::from(pf).join("Windows Kits").join("10").join("bin");
            if let Some(dir) = newest_versioned_dir(&base) {
                let candidate = dir.join("x64").join("rc.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    // 4) MSVC：…\VC\Tools\MSVC\<ver>\bin\Hostx64\x64\rc.exe
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(pf) = std::env::var(key) {
            let vs_root = PathBuf::from(pf).join("Microsoft Visual Studio").join("2022");
            let Ok(editions) = std::fs::read_dir(&vs_root) else {
                continue;
            };
            for edition in editions.flatten() {
                let msvc = edition.path().join("VC").join("Tools").join("MSVC");
                if let Some(dir) = newest_versioned_dir(&msvc) {
                    let candidate = dir.join("bin").join("Hostx64").join("x64").join("rc.exe");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    None
}

/// 在 `base` 下挑出名字形如 `10.0.26100.0` 的最大版本目录。
fn newest_versioned_dir(base: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(key) = version_sort_key(&name) else {
            continue;
        };
        if best.as_ref().map(|(k, _)| key > *k).unwrap_or(true) {
            best = Some((key, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// `10.0.26100.0` → `[10,0,26100,0]`；非版本名返回 `None`。
fn version_sort_key(name: &str) -> Option<Vec<u32>> {
    let mut key = Vec::new();
    for seg in name.split('.') {
        key.push(seg.parse::<u32>().ok()?);
    }
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

/// 极简 `which`：按 PATH 逐段查找可执行文件。
fn which(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// 注：本文件不写 `#[cfg(test)]` 单测——cargo 不会为 build 脚本构建测试配置，
// 那些测试永远不会被执行（写了等于假测试）。纯函数 `numeric_parts` /
// `version_sort_key` 的正确性由"每次构建都必须成功产出 VERSIONINFO"来保证。

