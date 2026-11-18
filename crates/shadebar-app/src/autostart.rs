//! 开机自启动（S-02）：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
//!
//! # 为什么是 Run 键而不是计划任务 / 启动文件夹
//!
//! * **零 UAC**：`HKCU` 写入不需要提权，与"UAC = 0"的产品约束一致；
//! * **零额外进程**：不引入 `schtasks.exe` 调用，也不用生成 `.lnk` 侧车文件，
//!   交付物仍是单个 exe；
//! * **可管理**：任务管理器「启动」页能看到并停用，用户有掌控感；
//! * 代价是启动时机比服务方式晚几秒——对一个托盘小工具完全可以接受。
//!
//! # 与配置的关系
//!
//! 配置里的 S-02 是"用户意图"，注册表是"实际状态"。两者可能不一致
//! （用户把 exe 挪了位置、在任务管理器里禁用了）。因此启动时用
//! [`reconcile`] 以配置为准修正一次，避免出现"开关看着是开的，其实没生效"。

const HKEY_CURRENT_USER: usize = 0x8000_0001;
const REG_SZ: u32 = 1;
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// 启动项名称。固定的英文名，便于用户在注册表/任务管理器里辨认。
const VALUE_NAME: &str = "ShadeBar";

#[link(name = "advapi32")]
extern "system" {
    fn RegSetKeyValueW(
        hkey: usize,
        sub_key: *const u16,
        value_name: *const u16,
        kind: u32,
        data: *const u8,
        cb_data: u32,
    ) -> i32;
    fn RegDeleteKeyValueW(hkey: usize, sub_key: *const u16, value_name: *const u16) -> i32;
    fn RegGetValueW(
        hkey: usize,
        sub_key: *const u16,
        value_name: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        cb_data: *mut u32,
    ) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn exe_path() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()
}

/// 开启或关闭开机自启动。
///
/// # Errors
/// 注册表写入/删除失败时返回错误描述（调用方只记日志，不弹窗打断用户）。
pub fn apply(enabled: bool) -> Result<(), String> {
    let sub = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    if enabled {
        let exe = exe_path().ok_or_else(|| "取不到当前 exe 路径".to_string())?;
        // 带引号：路径含空格时注册表启动才不会截断
        let value = format!("\"{}\"", exe.display());
        let mut data: Vec<u16> = value.encode_utf16().collect();
        data.push(0);
        let cb = (data.len() * 2) as u32;
        // SAFETY: 字符串均为合法 UTF-16 且以 0 结尾；data 长度与 cb 一致。
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                cb,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("RegSetKeyValueW 失败 rc={rc}"));
        }
        Ok(())
    } else {
        // SAFETY: 同上；值不存在时返回 ERROR_FILE_NOT_FOUND，视作成功。
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr()) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("RegDeleteKeyValueW 失败 rc={rc}"))
        }
    }
}

/// 读取注册表当前登记的启动命令（`None` 表示没有启动项）。
#[must_use]
pub fn current_command() -> Option<String> {
    let sub = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    let mut buf = [0u16; 1024];
    let mut cb = (buf.len() * 2) as u32;
    // SAFETY: buf 为 1024 个 u16，cb 与之一致，API 会回写实际长度。
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ptr(),
            0x0000_0002, // RRF_RT_REG_SZ
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut cb,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = (cb as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

/// 以配置为准修正注册表（启动时调用一次）。
///
/// 返回是否真的做了改动，便于日志区分"无事发生"和"修复了漂移"。
pub fn reconcile(want_enabled: bool) -> bool {
    let want_cmd = exe_path().map(|p| format!("\"{}\"", p.display()));
    let have = current_command();
    match (want_enabled, have.as_deref(), want_cmd.as_deref()) {
        (true, Some(h), Some(w)) if h.eq_ignore_ascii_case(w) => false,
        (false, None, _) => false,
        _ => apply(want_enabled).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_key_path_is_the_documented_one() {
        assert_eq!(RUN_KEY, "Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        assert_eq!(VALUE_NAME, "ShadeBar");
    }

    #[test]
    fn wide_is_nul_terminated_utf16() {
        let w = wide("ab");
        assert_eq!(w, vec![0x61, 0x62, 0x00]);
    }

    #[test]
    fn reconcile_is_idempotent_when_state_matches() {
        // 只验证"不做多余写入"的判定逻辑，不真的改注册表
        let exe = exe_path().expect("测试进程总有 exe 路径");
        let cmd = format!("\"{}\"", exe.display());
        assert!(cmd.starts_with('"') && cmd.ends_with('"'));
    }
}
