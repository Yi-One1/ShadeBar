//! `symprobe` —— 构建期符号探针（SP-2 符号方案的工具部分）。
//!
//! # 它做什么
//!
//! 1. 解析目标 DLL 的 PE 头，取出**三重校验**所需的三项：
//!    `TimeDateStamp` + `SizeOfImage` + PE 里 CodeView 的 `RSDS`（PDB 签名 + Age）；
//! 2. 由 `RSDS` 算出 Microsoft 公开符号服务器上的 PDB URL 并下载
//!    （`https://msdl.microsoft.com/download/symbols/<Pdb>/<SIG><AGE>/<Pdb>`）；
//! 3. 用系统自带的 `dbghelp.dll` 加载该 PDB、枚举符号、按关键字过滤；
//! 4. 打印 `RVA  名称`，并可导出 JSON 偏移表（供运行期注入器使用）。
//!
//! # 为什么这样设计（团队已定稿的 3 条安全约束）
//!
//! * **PDB 只在构建期用**：本工具是独立进程、不在发布产物里；产物只带
//!   `data/symbols/<build>.json` 这份偏移表，运行期不联网、不依赖符号服务器。
//! * **三重校验**：偏移表里记录 `build` + `time_date_stamp` + `size_of_image`；
//!   运行期三者必须全部与目标模块一致才允许注入，任一不符即安全放弃并降级。
//!   *这三项都在本工具里从**真实文件**读出，不靠版本号字符串猜。*
//! * **按 build 分条入库**：输出文件名即 build 号，适配新 build 只追加一条、不改代码。
//!
//! # 为什么全部符号 API 都走 `GetProcAddress`
//!
//! `dbghelp.lib` 的导入名在不同 SDK 版本里并不一致（例如 `SymGetSymFromNameW`
//! 在部分 SDK 的导入库里查不到，链接期直接 `undefined symbol`）。
//! 把 `dbghelp.dll` 的所有入口改成运行期 `GetProcAddress` 有两个好处：
//! 链接期永不失败；且可以"先试宽字符版、再退回 ANSI 版"，兼容性最好。
//!
//! # `--scan`：不注入、纯读盘的"能不能定位"验证
//!
//! 目标字符串（如 `TimeInnerTextBlock`）以 UTF-16 存在 `.rdata` 里，
//! 引用它的代码一定是 `lea reg, [rip+disp32]` 这类 RIP 相对寻址。
//! 因此：定位字符串 RVA → 在可执行节里扫 `REX 8B/8D modrm=mod00/rm101` →
//! 反算目标 RVA，恰好等于字符串 RVA 的就是引用点 → 再用 `SymFromAddrW`
//! 把引用点 RVA 翻译成"所在函数名 + 偏移"。
//!
//! 这样在**注入一行代码之前**就能回答"这个模块里到底有没有代码按名字找这个元素"，
//! 并且给出待 hook 的函数名。扫描只做字节模式匹配，**不做反汇编推导**。
//!
//! # 用法
//!
//! ```text
//! symprobe --dll <dll> --work <符号缓存目录> [--filter <子串>]... [--scan <UTF16 字符串>]...
//!          [--probe <符号名>]... [--out <json>] [--build <版本号>]
//! ```
//!
//! `--probe`：把一个符号名（可带修饰）求址回 RVA，用来做**双向交叉验证**——
//! `--scan` 先给出"引用点所在函数"，再用 `--probe` 把该函数名求址回去，
//! 若求址 RVA 落在引用点所属函数区间内，即证明"PDB 名字与地址一致"。
//! PDB 名字 → 地址 → 代码引用字符串，三方对上才算定位成立。

#![allow(clippy::too_many_lines)]

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// 极简 PE 解析（只取我们需要的几样东西，不引第三方 crate）
// ---------------------------------------------------------------------------

/// 一个节表项里我们关心的三样东西。
#[derive(Clone, Copy)]
struct Section {
    virt_addr: u32,
    virt_size: u32,
    raw_ptr: u32,
    raw_size: u32,
    characteristics: u32,
}

impl Section {
    /// 该 RVA 是否落在本节覆盖的范围内。
    fn contains_rva(&self, rva: u32) -> bool {
        let span = self.virt_size.max(self.raw_size);
        rva >= self.virt_addr && rva < self.virt_addr.saturating_add(span)
    }

    /// 本节在文件里的字节范围（可能为空，例如 `.bss`）。
    fn raw_range(&self) -> Option<(usize, usize)> {
        if self.raw_size == 0 || self.raw_ptr == 0 {
            return None;
        }
        Some((self.raw_ptr as usize, self.raw_size as usize))
    }

    /// 该节是否含可执行代码（`IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_CNT_CODE`）。
    fn is_code(&self) -> bool {
        const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
        const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
        self.characteristics & (IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE) != 0
    }
}

/// 从 PE 里读出的、三重校验与下载 PDB 所需的全部信息。
struct PeInfo {
    time_date_stamp: u32,
    size_of_image: u32,
    preferred_base: u64,
    pdb_name: String,
    /// Microsoft 符号服务器使用的签名：GUID 去掉连字符 + Age（十六进制）。
    pdb_signature: String,
    sections: Vec<Section>,
}

fn read_u16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn read_u32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64(b: &[u8], off: usize) -> Option<u64> {
    b.get(off..off + 8).map(|s| {
        u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
    })
}

/// 解析 PE。只支持 PE32/PE32+（本工具只处理 x64 DLL）。
fn parse_pe(bytes: &[u8]) -> Result<PeInfo, String> {
    if bytes.get(0..2) != Some(b"MZ") {
        return Err("不是 PE 文件（缺 MZ）".into());
    }
    let e_lfanew = read_u32(bytes, 0x3C).ok_or("DOS 头被截断")? as usize;
    if bytes.get(e_lfanew..e_lfanew + 4) != Some(b"PE\0\0") {
        return Err("缺 PE 签名".into());
    }
    let file_header = e_lfanew + 4;
    // FileHeader: Machine(0) NumberOfSections(2) TimeDateStamp(4) PtrSymTab(8)
    //             NumberOfSymbols(12) SizeOfOptionalHeader(16) Characteristics(18)
    let section_count = read_u16(bytes, file_header + 2).ok_or("FileHeader 被截断")? as usize;
    let time_date_stamp = read_u32(bytes, file_header + 4).ok_or("FileHeader 被截断")?;
    let size_of_optional = read_u16(bytes, file_header + 16).ok_or("FileHeader 被截断")? as usize;
    let optional = file_header + 20;

    let magic = read_u16(bytes, optional).ok_or("OptionalHeader 被截断")?;
    if magic != 0x20B {
        return Err(format!("只支持 PE32+（x64），实际 magic=0x{magic:04X}"));
    }
    // PE32+：ImageBase(24) SizeOfImage(56) DataDirectory(112)
    let preferred_base = read_u64(bytes, optional + 24).ok_or("ImageBase 缺失")?;
    let size_of_image = read_u32(bytes, optional + 56).ok_or("SizeOfImage 缺失")?;

    let sections_base = optional + size_of_optional;
    let mut sections = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let s = sections_base + i * 40;
        let Some(virt_size) = read_u32(bytes, s + 8) else {
            return Err("节表被截断".into());
        };
        sections.push(Section {
            virt_size,
            virt_addr: read_u32(bytes, s + 12).ok_or("节表被截断")?,
            raw_size: read_u32(bytes, s + 16).ok_or("节表被截断")?,
            raw_ptr: read_u32(bytes, s + 20).ok_or("节表被截断")?,
            characteristics: read_u32(bytes, s + 36).ok_or("节表被截断")?,
        });
    }

    let data_dir = optional + 112;
    let debug_rva = read_u32(bytes, data_dir + 6 * 8).ok_or("DataDirectory 被截断")?;
    let debug_size = read_u32(bytes, data_dir + 6 * 8 + 4).ok_or("DataDirectory 被截断")?;
    if debug_rva == 0 || debug_size == 0 {
        return Err("没有 Debug 目录（无 PDB 签名，无法走符号方案）".into());
    }

    let rva_to_offset = |rva: u32| -> Option<usize> {
        sections
            .iter()
            .find(|s| s.contains_rva(rva))
            .map(|s| (rva - s.virt_addr) as usize + s.raw_ptr as usize)
    };

    let debug_off = rva_to_offset(debug_rva).ok_or("Debug 目录 RVA 不在任何节内")?;
    // IMAGE_DEBUG_DIRECTORY 每项 28 字节；Type 在 +12，Type==2 即 CODEVIEW。
    let mut cursor = debug_off;
    let end = debug_off + debug_size as usize;
    while cursor + 28 <= end {
        let d_type = read_u32(bytes, cursor + 12).ok_or("Debug 目录项被截断")?;
        let size_of_data = read_u32(bytes, cursor + 16).ok_or("Debug 目录项被截断")? as usize;
        let ptr_raw = read_u32(bytes, cursor + 24).ok_or("Debug 目录项被截断")? as usize;
        if d_type == 2 && size_of_data >= 24 {
            let data = bytes
                .get(ptr_raw..ptr_raw + size_of_data)
                .ok_or("CodeView 数据越界")?;
            if data.get(0..4) != Some(b"RSDS") {
                return Err("CodeView 记录不是 RSDS（可能是 PDB 2.0 或已剥离）".into());
            }
            let guid = data.get(4..20).ok_or("RSDS GUID 被截断")?;
            // GUID 前三个字段小端存储，后 8 字节按原字节序。
            let d1 = u32::from_le_bytes([guid[0], guid[1], guid[2], guid[3]]);
            let d2 = u16::from_le_bytes([guid[4], guid[5]]);
            let d3 = u16::from_le_bytes([guid[6], guid[7]]);
            let mut sig = format!("{d1:08X}{d2:04X}{d3:04X}");
            for b in &guid[8..16] {
                sig.push_str(&format!("{b:02X}"));
            }
            let age = read_u32(data, 20).ok_or("RSDS Age 被截断")?;
            sig.push_str(&format!("{age:X}"));

            let name_bytes = data.get(24..).ok_or("RSDS PDB 名被截断")?;
            let name_end = name_bytes
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(name_bytes.len());
            let pdb_name = String::from_utf8_lossy(&name_bytes[..name_end]).into_owned();
            let pdb_name = Path::new(&pdb_name)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(pdb_name);

            return Ok(PeInfo {
                time_date_stamp,
                size_of_image,
                preferred_base,
                pdb_name,
                pdb_signature: sig,
                sections,
            });
        }
        cursor += 28;
    }
    Err("Debug 目录里没有 CodeView(RSDS) 记录".into())
}

// ---------------------------------------------------------------------------
// dbghelp 结构体（与 Windows SDK 的 IMAGEHLP_MODULEW64 / SYMBOL_INFOW 逐字段对齐）
// ---------------------------------------------------------------------------

const MAX_SYM_NAME: usize = 2000;

/// `IMAGEHLP_MODULEW64`。字段顺序与宽度严格照 SDK 定义写，
/// 由 `repr(C)` 自动插入与 C 编译器一致的对齐填充。
#[repr(C)]
#[derive(Clone, Copy)]
struct ImagehlpModuleW64 {
    size_of_struct: u32,
    base_of_image: u64,
    image_size: u32,
    time_date_stamp: u32,
    check_sum: u32,
    num_syms: u32,
    sym_type: u32,
    module_name: [u16; 32],
    image_name: [u16; 256],
    loaded_image_name: [u16; 256],
    loaded_pdb_name: [u16; 256],
    cv_sig: u32,
    cv_data: [u16; 780], // WCHAR CVData[MAX_PATH*3]
    pdb_sig: u32,
    pdb_sig70: [u8; 16],
    pdb_age: u32,
    pdb_unmatched: i32,
    dbg_unmatched: i32,
    line_numbers: i32,
    global_symbols: i32,
    type_info: i32,
    source_indexed: i32,
    publics: i32,
    machine_type: u32,
    reserved: u32,
}

impl ImagehlpModuleW64 {
    fn zeroed() -> Self {
        // 全零初始化即可，随后只填 size_of_struct。
        // SAFETY: 所有字段都是整数/整数数组，全零是合法值。
        let mut v: Self = unsafe { std::mem::zeroed() };
        v.size_of_struct = std::mem::size_of::<Self>() as u32;
        v
    }

    fn utf16_field(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    fn sym_type_name(&self) -> &'static str {
        match self.sym_type {
            0 => "SymNone(未加载)",
            1 => "SymCoff",
            2 => "SymCv",
            3 => "SymPdb(已加载)",
            4 => "SymExport",
            5 => "SymDeferred(延迟)",
            6 => "SymSym",
            7 => "SymDia",
            8 => "SymVirtual",
            _ => "未知",
        }
    }
}

/// `SYMBOL_INFOW`。`Name` 是柔性数组，这里定长 `MAX_SYM_NAME` 并把
/// `MaxNameLen` 设为可用容量。
#[repr(C)]
#[derive(Clone, Copy)]
struct SymbolInfoW {
    size_of_struct: u32,
    type_index: u32,
    reserved: [u64; 2],
    index: u32,
    size: u32,
    mod_base: u64,
    flags: u32,
    value: u64,
    address: u64,
    register: u32,
    scope: u32,
    tag: u32,
    name_len: u32,
    max_name_len: u32,
    name: [u16; MAX_SYM_NAME],
}

impl SymbolInfoW {
    fn zeroed() -> Self {
        // SAFETY: 全零对整数/整数数组字段是合法值。
        let mut v: Self = unsafe { std::mem::zeroed() };
        v.size_of_struct = std::mem::size_of::<Self>() as u32;
        v.max_name_len = MAX_SYM_NAME as u32;
        v
    }

    fn name_lossy(&self) -> String {
        let end = self
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.name.len());
        String::from_utf16_lossy(&self.name[..end])
    }
}

// ---------------------------------------------------------------------------
// dbghelp 动态绑定
// ---------------------------------------------------------------------------

type Handle = *mut c_void;
/// `PSYM_ENUMERATESYMBOLS_CALLBACKW` —— **三参数原型**（照 SDK DbgHelp.h）：
/// `(PSYMBOL_INFOW, ULONG SymbolSize, PVOID UserContext)`。
/// 注意不是网上常见的老式 `(PCWSTR, ULONG64, ULONG, PVOID)` 四参数原型；
/// 符号名与地址都从 `SYMBOL_INFOW` 里读。
type EnumCallback =
    Option<unsafe extern "system" fn(*const SymbolInfoW, u32, *mut c_void) -> i32>;

type FnSymInitializeW = unsafe extern "system" fn(Handle, *const u16, i32) -> i32;
type FnSymSetOptions = unsafe extern "system" fn(u32) -> u32;
type FnSymGetOptions = unsafe extern "system" fn() -> u32;
type FnSymLoadModuleExW =
    unsafe extern "system" fn(Handle, Handle, *const u16, *const u16, u64, u32, *mut c_void, u32) -> u64;
type FnSymFromAddrW = unsafe extern "system" fn(Handle, u64, *mut u64, *mut SymbolInfoW) -> i32;
type FnSymFromNameW = unsafe extern "system" fn(Handle, *const u16, *mut SymbolInfoW) -> i32;
type FnSymEnumSymbolsW =
    unsafe extern "system" fn(Handle, u64, *const u16, EnumCallback, *mut c_void) -> i32;
type FnSymGetModuleInfoW64 = unsafe extern "system" fn(Handle, u64, *mut ImagehlpModuleW64) -> i32;
type FnSymSearchW = unsafe extern "system" fn(
    Handle,
    u64,
    u32,
    u32,
    *const u16,
    u64,
    EnumCallback,
    *mut c_void,
    u32,
) -> i32;
type FnSymCleanup = unsafe extern "system" fn(Handle) -> i32;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn GetLastError() -> u32;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
    fn FreeLibrary(module: Handle) -> i32;
}

/// 取当前线程的 `GetLastError`，用于把失败原因说清楚。
fn last_error() -> u32 {
    // SAFETY: 无参数、无前置条件。
    unsafe { GetLastError() }
}

// ---------------------------------------------------------------------------
// DBWIN 捕获：抓取 dbghelp 在 SYMOPT_DEBUG 下发的 OutputDebugString 诊断流
// ---------------------------------------------------------------------------

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(attrs: *mut c_void, manual: i32, init: i32, name: *const u16) -> Handle;
    fn CreateFileMappingW(
        file: Handle,
        attrs: *mut c_void,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> Handle;
    fn MapViewOfFile(mapping: Handle, access: u32, off_high: u32, off_low: u32, bytes: usize)
        -> *mut c_void;
    fn CloseHandle(h: Handle) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn SetEvent(h: Handle) -> i32;
}

const PAGE_READWRITE: u32 = 0x0000_0004;
const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
const WAIT_OBJECT_0: u32 = 0;

/// 开启 DBWIN 监听线程，把本进程的 OutputDebugString 输出打到 stderr。
/// 若机器上已有别的调试器在监听（对象已存在），返回 None 不干扰。
fn start_dbwin_capture() -> bool {
    let buf_name = to_wide_z(OsStr::new("DBWIN_BUFFER"));
    let ready_name = to_wide_z(OsStr::new("DBWIN_BUFFER_READY"));
    let data_name = to_wide_z(OsStr::new("DBWIN_DATA_READY"));
    // SAFETY: 全部为 NUL 结尾宽串/标量；句柄在线程里统一释放。
    let mapping = unsafe {
        CreateFileMappingW(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            PAGE_READWRITE,
            0,
            4096 + 4,
            buf_name.as_ptr(),
        )
    };
    if mapping.is_null() || last_error() == 183 {
        // ERROR_ALREADY_EXISTS = 183：已有调试器在监听。
        return false;
    }
    // SAFETY: mapping 是刚创建的有效句柄。
    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 4096 + 4) };
    if view.is_null() {
        unsafe { CloseHandle(mapping) };
        return false;
    }
    // SAFETY: 名称与初始态均为常量。
    let ready = unsafe { CreateEventW(std::ptr::null_mut(), 0, 1, ready_name.as_ptr()) };
    let data = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, data_name.as_ptr()) };
    if ready.is_null() || data.is_null() {
        unsafe { CloseHandle(mapping) };
        return false;
    }
    {
        // 监听线程：等 DATA_READY → 读出 → 打印 → 置 BUFFER_READY。
        // 初始 BUFFER_READY 已置为有信号。句柄以 usize 传线程（裸指针不 Send）。
        let (mapping_u, view_u, ready_u, data_u) = (
            mapping as usize,
            view as usize,
            ready as usize,
            data as usize,
        );
        // 注意：闭包在**另一个线程**上执行，外层 unsafe 块不覆盖其运行期语义，
        // 因此闭包体自带 unsafe 块。
        let thread = std::thread::spawn(move || unsafe {
            let data = data_u as Handle;
            let view = view_u as *mut c_void;
            let ready = ready_u as Handle;
            loop {
                // SAFETY: 句柄有效。
                let w = WaitForSingleObject(data, 500);
                if w != WAIT_OBJECT_0 {
                    // 超时继续等；线程随进程退出。
                    continue;
                }
                // SAFETY: DBWIN 缓冲区布局固定：[u32 pid][4096 字节文本]。
                let pid = *(view as *const u32);
                let text_ptr = (view as *const u8).add(4);
                let text = std::slice::from_raw_parts(text_ptr, 4096);
                let end = text.iter().position(|&c| c == 0).unwrap_or(4096);
                let msg = String::from_utf8_lossy(&text[..end]);
                eprintln!("[dbghelp|pid={pid}] {msg}");
                SetEvent(ready);
            }
        });
        // 线程随主进程退出（句柄由内核回收；detach 语义）。
        std::mem::forget(thread);
        let _ = mapping_u;
        true
    }
}

fn to_wide_z(s: &OsStr) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_wide().collect();
    v.push(0);
    v
}

/// 按名字查一个导出函数；`names` 按优先级依次尝试（宽字符版在前、ANSI 版兜底）。
///
/// # Safety
/// 返回的函数指针必须按 `T` 的真实签名调用，调用方负责保证签名一致。
unsafe fn resolve<T: Copy>(module: Handle, names: &[&str]) -> Option<T> {
    for name in names {
        let mut c: Vec<u8> = name.as_bytes().to_vec();
        c.push(0);
        // SAFETY: GetProcAddress 接受 NUL 结尾的 ASCII 名称。
        let p = unsafe { GetProcAddress(module, c.as_ptr()) };
        if !p.is_null() {
            // SAFETY: T 是 extern "system" 函数指针；调用方保证与导出签名一致。
            return Some(unsafe { std::mem::transmute_copy::<*const c_void, T>(&p) });
        }
    }
    None
}

/// dbghelp 的全部入口。任何一个缺失都可以据此精确报错。
struct DbgHelp {
    module: Handle,
    sym_initialize_w: FnSymInitializeW,
    sym_set_options: FnSymSetOptions,
    sym_get_options: FnSymGetOptions,
    sym_load_module_ex_w: FnSymLoadModuleExW,
    sym_from_addr_w: FnSymFromAddrW,
    sym_from_name_w: FnSymFromNameW,
    sym_enum_symbols_w: FnSymEnumSymbolsW,
    sym_search_w: FnSymSearchW,
    sym_get_module_info_w64: FnSymGetModuleInfoW64,
    sym_cleanup: FnSymCleanup,
}

impl DbgHelp {
    fn load() -> Result<Self, String> {
        let name = to_wide_z(OsStr::new("dbghelp.dll"));
        // SAFETY: name 是 NUL 结尾宽串。
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return Err(format!("LoadLibraryW(dbghelp.dll) 失败（{}）", last_error()));
        }
        // SAFETY: 下面的 resolve 都以正确的导出签名取址。
        unsafe {
            let missing = |n: &str| format!("dbghelp.dll 缺少导出 {n}");
            Ok(Self {
                module,
                sym_initialize_w: resolve(module, &["SymInitializeW", "SymInitialize"])
                    .ok_or_else(|| missing("SymInitializeW"))?,
                sym_set_options: resolve(module, &["SymSetOptions"])
                    .ok_or_else(|| missing("SymSetOptions"))?,
                sym_get_options: resolve(module, &["SymGetOptions"])
                    .ok_or_else(|| missing("SymGetOptions"))?,
                sym_load_module_ex_w: resolve(
                    module,
                    &["SymLoadModuleExW", "SymLoadModuleEx"],
                )
                .ok_or_else(|| missing("SymLoadModuleExW"))?,
                sym_from_addr_w: resolve(module, &["SymFromAddrW", "SymFromAddr"])
                    .ok_or_else(|| missing("SymFromAddrW"))?,
                sym_from_name_w: resolve(module, &["SymFromNameW", "SymFromName"])
                    .ok_or_else(|| missing("SymFromNameW"))?,
                sym_enum_symbols_w: resolve(
                    module,
                    &["SymEnumSymbolsW", "SymEnumSymbols"],
                )
                .ok_or_else(|| missing("SymEnumSymbolsW"))?,
                sym_search_w: resolve(module, &["SymSearchW", "SymSearch"])
                    .ok_or_else(|| missing("SymSearchW"))?,
                sym_get_module_info_w64: resolve(module, &["SymGetModuleInfoW64"])
                    .ok_or_else(|| missing("SymGetModuleInfoW64"))?,
                sym_cleanup: resolve(module, &["SymCleanup"])
                    .ok_or_else(|| missing("SymCleanup"))?,
            })
        }
    }
}

impl Drop for DbgHelp {
    fn drop(&mut self) {
        if !self.module.is_null() {
            // SAFETY: module 由本结构体的 LoadLibraryW 获得，只释放一次。
            unsafe {
                FreeLibrary(self.module);
            }
            self.module = std::ptr::null_mut();
        }
    }
}

const SYMOPT_UNDNAME: u32 = 0x0000_0002;
const SYMOPT_LOAD_LINES: u32 = 0x0000_0010;
const SYMOPT_DEBUG: u32 = 0x8000_0000;

/// 一次符号会话：`SymbolSession::open` 负责 `SymInitializeW` + `SymLoadModuleExW`，
/// `Drop` 负责 `SymCleanup` + 释放以资源方式映射的映像。
/// 这样任何提前 return 都不会漏清理。
struct SymbolSession<'a> {
    dbg: &'a DbgHelp,
    process: Handle,
    base: u64,
    /// 以 LOAD_LIBRARY_AS_IMAGE_RESOURCE 映射的映像句柄（需要 FreeLibrary）。
    mapped: Handle,
}

impl<'a> SymbolSession<'a> {
    fn open(
        dbg: &'a DbgHelp,
        dll: &Path,
        search_dir: &Path,
        size_of_image: u32,
    ) -> Result<Self, String> {
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: 无参数。
        let opts = unsafe { (dbg.sym_get_options)() };
        // 注意：**不要**加 `SYMOPT_DEFERRED_LOADS`。
        // 延迟加载会让 dbghelp 在首次 `SymEnumSymbolsW` / `SymFromAddrW` 时才真正
        // 去读映像与 PDB，而此时目标模块并不在本进程的地址空间里 ——
        // 实测表现为 dbghelp 内部 0xC0000005。改成立即加载后就一切正常。
        // `SYMOPT_EXACT_SYMBOLS` 也不要加：实测它会让"非进程内模块"的
        // PDB 符号查找直接报 87（ERROR_INVALID_PARAMETER）。
        let mut opt_extra = 0u32;
        if std::env::var("SYMPROBE_DEBUG").map(|v| v == "1").unwrap_or(false) {
            // 调试模式：dbghelp 会经 OutputDebugString 吐出完整诊断流，
            // 由 main 里启动的 DBWIN 监听线程打到 stderr。
            opt_extra |= SYMOPT_DEBUG;
        }
        // SAFETY: 参数都是标量。
        unsafe {
            (dbg.sym_set_options)(opts | SYMOPT_UNDNAME | SYMOPT_LOAD_LINES | opt_extra);
        }

        // 搜索路径：PDB 缓存目录 + DLL 自身所在目录。
        //
        // 两个细节必须注意，否则 dbghelp 会静默退化到"只用 DLL 的导出表"：
        //  1. 必须是**绝对路径**（相对路径 dbghelp 不解析，直接当成无效项）；
        //  2. 多个目录用 `;` 分隔，必须用字符串拼（`PathBuf::push(";")` 会插入分隔符，
        //     拼出 `...\sym\;\C:\Windows\...` 这种垃圾路径）；
        //  3. **不要** `\\?\` 前缀 —— dbghelp 内部对该前缀的路径拼接
        //     （拼 PDB 全名）会产出非法路径，表现为 NumSyms=0。
        let work_abs = std::fs::canonicalize(search_dir)
            .map(|p| {
                let s = p.display().to_string();
                s.trim_start_matches(r"\\?\").to_string()
            })
            .unwrap_or_else(|_| search_dir.display().to_string());
        let dll_dir = dll
            .parent()
            .map(|p| {
                std::fs::canonicalize(p)
                    .map(|c| {
                        c.display()
                            .to_string()
                            .trim_start_matches(r"\\?\")
                            .to_string()
                    })
                    .unwrap_or_else(|_| p.display().to_string())
            });
        let mut search = work_abs;
        if let Some(dir) = dll_dir.as_ref() {
            search.push(';');
            search.push_str(dir);
        }
        println!("[symprobe] 符号搜索路径     = {search}");
        // 同时写进 `_NT_SYMBOL_PATH`：dbghelp 在 `SymInitializeW` 给的路径里
        // 找不到时会回落到这个环境变量，双保险。
        std::env::set_var("_NT_SYMBOL_PATH", &search);

        let search_w = to_wide_z(OsStr::new(&search));
        // 第三个参数 fInvadeProcess 必须是 FALSE：
        // invade=1 会枚举本进程已加载模块并与我们给的"文件式模块"互相干扰，
        // 实测 SymEnumSymbolsW 在该组合下 dbghelp 内部 0xC0000005。
        // SAFETY: process 是伪句柄；search_w 是 NUL 结尾宽串。
        if unsafe { (dbg.sym_initialize_w)(process, search_w.as_ptr(), 0) } == 0 {
            return Err(format!("SymInitializeW 失败（GetLastError={}）", last_error()));
        }

        let image = to_wide_z(dll.as_os_str());

        // 第 0 策略（实测唯一能让枚举/查址完全正常的一条）：
        // 先用 LoadLibraryExW 把 DLL 以"资源映像"方式映射进本进程地址空间
        // （LOAD_LIBRARY_AS_IMAGE_RESOURCE：按节映射、不执行、不应用重定位；
        //   LOAD_LIBRARY_AS_DATAFILE：不做任何初始化），再把映射基址交给
        // SymLoadModuleExW。这样 dbghelp 的枚举/查址路径就能直接读映像内存，
        // 不会在内部因为"映像不可读"而 0xC0000005。
        // 映射句柄低位带标记位（实测 LOAD_LIBRARY_AS_IMAGE_RESOURCE 时
        // 最低两位为 0b10，即 bit1 置 1），按页对齐掩码清掉低 12 位再当基址用。
        const LOAD_LIBRARY_AS_DATAFILE: u32 = 0x0000_0002;
        const LOAD_LIBRARY_AS_IMAGE_RESOURCE: u32 = 0x0000_0020;
        // SAFETY: image 为 NUL 结尾宽串；flags 只映射不执行。
        let mapped = unsafe {
            LoadLibraryExW(
                image.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            )
        };
        if mapped.is_null() {
            return Err(format!(
                "LoadLibraryExW(IMAGE_RESOURCE) 失败（GetLastError={}）",
                last_error()
            ));
        }
        let mapped_base = mapped as u64 & !0xFFF;
        println!("[symprobe] 映像已映射：句柄=0x{:016X} 基址=0x{mapped_base:016X}", mapped as u64);

        // SAFETY: mapped_base 是真实映射基址，dbghelp 据此读映像头。
        let mut base = unsafe {
            (dbg.sym_load_module_ex_w)(
                process,
                std::ptr::null_mut(),
                image.as_ptr(),
                std::ptr::null(),
                mapped_base,
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        let mut strategy = "base=映射基址,size=0,flags=0";
        if base == 0 {
            let e1 = last_error();
            println!("[symprobe] SymLoadModuleExW 策略0 失败（GetLastError={e1}），改用策略1");
            // 第 1 策略：base=0 / size=0 / flags=0。
            // dbghelp 自己读映像头拿 ImageBase 与 SizeOfImage。
            // SAFETY: image 为 NUL 结尾宽串，其余为空/0。
            base = unsafe {
                (dbg.sym_load_module_ex_w)(
                    process,
                    std::ptr::null_mut(),
                    image.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    std::ptr::null_mut(),
                    0,
                )
            };
            strategy = "base=0,size=0,flags=0";
            if base == 0 {
                let e2 = last_error();
                println!("[symprobe] SymLoadModuleExW 策略1 失败（GetLastError={e2}），改用策略2");
                // 第 2 策略：显式给一个不冲突的基址 + 真实大小 + SLMFLAG_VIRTUAL。
                const FAKE_BASE: u64 = 0x0000_0000_1000_0000;
                const SLMFLAG_VIRTUAL: u32 = 0x0000_0001;
                // SAFETY: 同上。
                base = unsafe {
                    (dbg.sym_load_module_ex_w)(
                        process,
                        std::ptr::null_mut(),
                        image.as_ptr(),
                        std::ptr::null(),
                        FAKE_BASE,
                        size_of_image,
                        std::ptr::null_mut(),
                        SLMFLAG_VIRTUAL,
                    )
                };
                strategy = "base=FAKE,size=real,SLMFLAG_VIRTUAL";
                if base == 0 {
                    let e3 = last_error();
                    // SAFETY: 与 SymInitializeW 配对。
                    unsafe {
                        (dbg.sym_cleanup)(process);
                    }
                    // SAFETY: 释放本函数映射的映像。
                    unsafe {
                        FreeLibrary(mapped);
                    }
                    return Err(format!(
                        "SymLoadModuleExW 三种策略都失败（err={e1}/{e2}/{e3}）"
                    ));
                }
            }
        }

        let session = Self {
            dbg,
            process,
            base,
            mapped,
        };
        println!(
            "[symprobe] SymLoadModuleExW 成功：策略={strategy} 返回基址=0x{base:016X} 映像大小=0x{size_of_image:08X}"
        );
        session.print_module_info();
        Ok(session)
    }

    /// 打印 `SymGetModuleInfoW64` 的关键字段，用来判别"符号到底绑上没有"。
    fn print_module_info(&self) {
        let mut info = ImagehlpModuleW64::zeroed();
        // SAFETY: info 是完整结构体，size_of_struct 已填为本结构体大小。
        let ok = unsafe { (self.dbg.sym_get_module_info_w64)(self.process, self.base, &mut info) };
        if ok == 0 {
            println!(
                "[symprobe] 诊断：SymGetModuleInfoW64 失败（GetLastError={}）",
                last_error()
            );
            return;
        }
        println!(
            "[symprobe] 诊断：SymType={}  NumSyms={}  ImageSize=0x{:08X}  TimeDateStamp=0x{:08X}",
            info.sym_type_name(),
            info.num_syms,
            info.image_size,
            info.time_date_stamp
        );
        println!(
            "[symprobe] 诊断：LoadedPdbName='{}'  LineNumbers={}  Publics={}  GlobalSymbols={}",
            ImagehlpModuleW64::utf16_field(&info.loaded_pdb_name),
            info.line_numbers,
            info.publics,
            info.global_symbols
        );
        println!(
            "[symprobe] 诊断：PdbUnmatched={}  DbgUnmatched={}  PdbAge={}  MachineType=0x{:X}",
            info.pdb_unmatched, info.dbg_unmatched, info.pdb_age, info.machine_type
        );
        // 结构体自身大小也打印出来，便于与 C 端 sizeof 对照排查偏移错误。
        println!(
            "[symprobe] 诊断：sizeof(IMAGEHLP_MODULEW64)={}  sizeof(SYMBOL_INFOW)={}",
            std::mem::size_of::<ImagehlpModuleW64>(),
            std::mem::size_of::<SymbolInfoW>()
        );
    }

    /// PDB 是否真的绑上了（`SymType == SymPdb`，即 3）。
    fn pdb_bound(&self) -> bool {
        let mut info = ImagehlpModuleW64::zeroed();
        // SAFETY: info 是完整结构体，size_of_struct 已填为本结构体大小。
        let ok = unsafe { (self.dbg.sym_get_module_info_w64)(self.process, self.base, &mut info) };
        ok != 0 && info.sym_type == 3
    }

    /// RVA → (函数名, 函数内偏移)。先试 `SymFromAddrW`（精确），
    /// 失败则退化为"全量枚举 + 最近邻"（取地址不大于目标的最近符号，
    /// 并给出上界符号用于估计函数范围）——publics-only PDB 上
    /// `SymFromAddrW` 实测报 87，但全量枚举可用，最近邻照样能定位到
    /// 包含函数。
    fn resolve_rva(&self, rva: u32) -> Option<(String, u64)> {
        if let Some(addr) = self.base.checked_add(u64::from(rva)) {
            let mut info = SymbolInfoW::zeroed();
            let mut displacement: u64 = 0;
            // SAFETY: info 是完整结构体且 max_name_len 已设置为数组容量。
            let ok = unsafe {
                (self.dbg.sym_from_addr_w)(
                    self.process,
                    addr,
                    &mut displacement,
                    &mut info,
                )
            };
            if ok != 0 {
                return Some((info.name_lossy(), displacement));
            }
        }
        // 最近邻退化：枚举全部符号找 floor。
        let (all, _) = self.enum_symbols(&[]).ok()?;
        let mut best: Option<(u64, String)> = None;
        for (sym_rva, name) in all {
            if sym_rva <= u64::from(rva)
                && best.as_ref().map_or(true, |(b, _)| sym_rva > *b)
            {
                best = Some((sym_rva, name));
            }
        }
        best.map(|(sym_rva, name)| (name, u64::from(rva) - sym_rva))
    }

    /// 符号名（可带修饰名）→ RVA。用来反过来验证"PDB 里的名字能不能求址"。
    fn resolve_name(&self, name: &str) -> Result<u64, String> {
        let wide = to_wide_z(OsStr::new(name));
        let mut info = SymbolInfoW::zeroed();
        // SAFETY: info 是完整结构体，max_name_len 已设置为数组容量。
        let ok = unsafe { (self.dbg.sym_from_name_w)(self.process, wide.as_ptr(), &mut info) };
        if ok == 0 {
            return Err(format!("GetLastError={}", last_error()));
        }
        info.address
            .checked_sub(self.base)
            .ok_or_else(|| format!("求得的地址 0x{:X} 小于模块基址", info.address))
    }

    /// 枚举全部符号，交给回调过滤。
    ///
    /// 两条路径：
    /// 1. `SymEnumSymbolsW`（符号表遍历）——首选；
    /// 2. `SymSearchW`（DIA 查询，`SymTagFunction`）——路径 1 失败时兜底。
    ///    两者走 dbghelp 内部不同的实现，路径 1 在"文件式加载模块"上
    ///    实测会 0xC0000005，路径 2 可以正常工作。
    fn enum_symbols(&self, filters: &[String]) -> Result<(Vec<(u64, String)>, usize), String> {
        // 只有 SymPdb 才值得枚举。`SymExport`（dbghelp 没用 PDB、只读了导出表）
        // 和 `SymNone` 都说明 PDB 没绑上，此时强行枚举只会拿到垃圾或崩溃。
        if !self.pdb_bound() {
            return Err(
                "PDB 未绑定（模块 SymType 不是 SymPdb）——跳过枚举以避免 dbghelp 内部 AV".into(),
            );
        }
        let mask = to_wide_z(OsStr::new("*"));
        let mut ctx = CollectCtx {
            base: self.base,
            filters: filters.to_vec(),
            matches: Vec::new(),
            total: 0,
        };
        let ctx_ptr = &mut ctx as *mut _ as *mut c_void;
        EXPECTED_CTX.store(ctx_ptr as usize, std::sync::atomic::Ordering::SeqCst);
        // SAFETY: 回调在调用期间同步执行，ctx 指针在本栈帧内一直有效。
        let ok = unsafe {
            (self.dbg.sym_enum_symbols_w)(
                self.process,
                self.base,
                mask.as_ptr(),
                Some(enum_cb),
                ctx_ptr,
            )
        };
        if ok == 0 {
            let e1 = last_error();
            println!(
                "[symprobe] SymEnumSymbolsW 失败（GetLastError={e1}），改走 SymSearchW 兜底"
            );
            // SymTagFunction = 5；SYMSEARCH_RECURSE(0x20)？不需要，直接按 tag 查。
            const SYM_TAG_FUNCTION: u32 = 5;
            // SAFETY: 同上，回调同步执行。
            let ok2 = unsafe {
                (self.dbg.sym_search_w)(
                    self.process,
                    self.base,
                    0,
                    SYM_TAG_FUNCTION,
                    mask.as_ptr(),
                    0,
                    Some(enum_cb),
                    ctx_ptr,
                    0,
                )
            };
            if ok2 == 0 {
                return Err(format!(
                    "SymEnumSymbolsW 与 SymSearchW 都失败（前者 err={e1}，后者 err={}）",
                    last_error()
                ));
            }
            println!("[symprobe] SymSearchW 兜底成功");
        }
        Ok((ctx.matches, ctx.total))
    }
}

impl Drop for SymbolSession<'_> {
    fn drop(&mut self) {
        // SAFETY: 与 Self::open 里的 SymInitializeW 严格配对，只调用一次。
        unsafe {
            (self.dbg.sym_cleanup)(self.process);
        }
        // SAFETY: mapped 由 Self::open 的 LoadLibraryExW 获得，只释放一次。
        unsafe {
            if !self.mapped.is_null() {
                FreeLibrary(self.mapped);
            }
        }
    }
}

/// 枚举回调的上下文：收集 (RVA, 名称)。
struct CollectCtx {
    base: u64,
    filters: Vec<String>,
    matches: Vec<(u64, String)>,
    total: usize,
}

/// 本次枚举调用期望的 ctx 指针（enum_symbols 调用前写入）。
/// 回调里先核对第 3 参是否等于它，不等则说明 dbghelp 的调用约定与
/// 我们的假设不符——打印诊断并跳过，而不是 deref 野指针直接崩。
static EXPECTED_CTX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn enum_cb(
    sym_info: *const SymbolInfoW,
    _size: u32,
    ctx: *mut c_void,
) -> i32 {
    use std::sync::atomic::Ordering;
    if sym_info.is_null() || ctx.is_null() {
        return 1;
    }
    let expected = EXPECTED_CTX.load(Ordering::Relaxed);
    if (ctx as usize) != expected {
        eprintln!(
            "[symprobe] 回调第3参异常：ctx=0x{:X} 期望=0x{:X}——跳过该次回调",
            ctx as usize, expected
        );
        return 1;
    }
    // SAFETY: ctx 已核对为 enum_symbols 栈上的 CollectCtx。
    let ctx = unsafe { &mut *(ctx as *mut CollectCtx) };
    ctx.total += 1;

    // SAFETY: dbghelp 保证 sym_info 指向合法的 SYMBOL_INFOW。
    let info = unsafe { &*sym_info };
    let name = info.name_lossy();
    if name.is_empty() {
        return 1;
    }

    let address = info.address;
    let lowered = name.to_lowercase();
    let hit = ctx.filters.is_empty()
        || ctx
            .filters
            .iter()
            .any(|f| lowered.contains(&f.to_lowercase()));
    if hit && address >= ctx.base {
        ctx.matches.push((address - ctx.base, name));
    }
    1
}

// ---------------------------------------------------------------------------
// 纯读盘定位：UTF-16 字符串 → 引用它的 `lea rip-rel` → 所在函数
// ---------------------------------------------------------------------------

/// 在文件字节里找 UTF-16LE 编码的 `needle`，返回全部命中的 RVA。
fn find_utf16_rvas(bytes: &[u8], sections: &[Section], needle: &str) -> Vec<u32> {
    let pattern: Vec<u8> = needle
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    if pattern.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + pattern.len() <= bytes.len() {
        if bytes[i] == pattern[0] && bytes[i..i + pattern.len()] == pattern[..] {
            // 文件偏移 → RVA
            let off = i as u32;
            if let Some(sec) = sections
                .iter()
                .find(|s| off >= s.raw_ptr && off < s.raw_ptr.saturating_add(s.raw_size))
            {
                out.push(off - sec.raw_ptr + sec.virt_addr);
            }
            i += pattern.len();
        } else {
            i += 1;
        }
    }
    out
}

/// 一个 RIP 相对引用点。
struct RefSite {
    /// `lea`/`mov` 指令自身的 RVA。
    insn_rva: u32,
    /// 反算出的被引用地址 RVA。
    target_rva: u32,
}

/// 在可执行节里扫 `REX.W 8B/8D modrm(mod=00, rm=101) disp32`，
/// 即"目标地址写死在指令里的 RIP 相对寻址"，只保留目标等于 `targets` 之一的。
///
/// 这是字节模式匹配，不是反汇编：只认这一种固定的 7 字节编码，
/// 误报通过 `target_rva` 精确相等来排除。
fn scan_rip_refs(bytes: &[u8], sections: &[Section], targets: &[u32]) -> Vec<RefSite> {
    let mut out = Vec::new();
    if targets.is_empty() {
        return out;
    }
    for sec in sections.iter().filter(|s| s.is_code()) {
        let Some((raw_ptr, raw_size)) = sec.raw_range() else {
            continue;
        };
        let Some(body) = bytes.get(raw_ptr..raw_ptr + raw_size) else {
            continue;
        };
        let mut i = 0usize;
        while i + 7 <= body.len() {
            let rex = body[i];
            let opcode = body[i + 1];
            let modrm = body[i + 2];
            // REX 前缀 0x48..=0x4F（W=1），opcode 0x8B/0x8D，mod=00 且 rm=101 → RIP 相对。
            let is_rip_rel = (0x48..=0x4F).contains(&rex)
                && (opcode == 0x8B || opcode == 0x8D)
                && (modrm & 0xC7) == 0x05;
            if is_rip_rel {
                let disp = i32::from_le_bytes([
                    body[i + 3],
                    body[i + 4],
                    body[i + 5],
                    body[i + 6],
                ]);
                let insn_rva = sec.virt_addr + (i as u32);
                // RIP 在指令结束时指向下一条指令。
                let next_rva = (insn_rva as i64) + 7;
                let target = next_rva + i64::from(disp);
                if target >= 0 {
                    let target_rva = target as u32;
                    if targets.contains(&target_rva) {
                        out.push(RefSite {
                            insn_rva,
                            target_rva,
                        });
                    }
                }
            }
            i += 1;
        }
    }
    out
}

/// 一次 `--scan` 的完整结果。
struct ScanResult {
    needle: String,
    string_rvas: Vec<u32>,
    sites: Vec<(u32, u32, Option<(String, u64)>)>,
}

fn run_scan(
    bytes: &[u8],
    info: &PeInfo,
    session: Option<&SymbolSession<'_>>,
    needle: &str,
) -> ScanResult {
    let string_rvas = find_utf16_rvas(bytes, &info.sections, needle);
    let sites = scan_rip_refs(bytes, &info.sections, &string_rvas);
    let mut rows: Vec<(u32, u32, Option<(String, u64)>)> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for site in sites {
        if seen.contains(&site.insn_rva) {
            continue;
        }
        seen.push(site.insn_rva);
        let resolved = session.and_then(|s| s.resolve_rva(site.insn_rva));
        rows.push((site.insn_rva, site.target_rva, resolved));
    }
    rows.sort_by_key(|r| r.0);
    ScanResult {
        needle: needle.to_string(),
        string_rvas,
        sites: rows,
    }
}

// ---------------------------------------------------------------------------
// 下载 PDB
// ---------------------------------------------------------------------------

fn download_pdb(info: &PeInfo, work: &Path) -> Result<PathBuf, String> {
    let target = work.join(&info.pdb_name);
    if target.is_file() && std::fs::metadata(&target).map(|m| m.len() > 1024).unwrap_or(false) {
        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        println!("[symprobe] PDB 已缓存（{size} 字节），跳过下载");
        return Ok(target);
    }
    let url = format!(
        "https://msdl.microsoft.com/download/symbols/{name}/{sig}/{name}",
        name = info.pdb_name,
        sig = info.pdb_signature
    );
    println!("[symprobe] 下载 PDB: {url}");
    // 用系统自带的 curl.exe，避免为一次构建期下载引入 HTTP 依赖。
    let output = Command::new("curl")
        .args([
            "-sS",
            "-L",
            "--fail",
            "--max-time",
            "180",
            "-w",
            "http=%{http_code}",
            "-o",
            &target.to_string_lossy(),
            &url,
        ])
        .output()
        .map_err(|e| format!("启动 curl 失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "curl 失败（退出码 {:?}，{stdout}）—— PDB 可能未公开或网络不可达",
            output.status.code()
        ));
    }
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if size < 1024 {
        return Err(format!("下载到的 PDB 过小（{size} 字节）"));
    }
    // 校验 MSF 魔数，防止把 HTML 错误页当成 PDB。
    let head = std::fs::read(&target)
        .map_err(|e| format!("读回 PDB 失败：{e}"))?
        .into_iter()
        .take(32)
        .collect::<Vec<u8>>();
    if !head.starts_with(b"Microsoft C/C++ MSF 7.00") {
        return Err(format!(
            "下载内容不是 PDB（首 32 字节：{}）",
            String::from_utf8_lossy(&head)
        ));
    }
    println!("[symprobe] PDB 下载完成（{size} 字节，{stdout}）");
    Ok(target)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn usage() {
    println!(
        "usage: symprobe --dll <path> --work <dir> [--filter <substr>]... \
         [--scan <utf16-string>]... [--probe <symbol-name>]... [--resolve <rva>]... \
         [--out <json>] [--build <ver>]"
    );
}

fn main() -> std::process::ExitCode {
    // SYMPROBE_DEBUG=1：抓取 dbghelp 的内部诊断流（SYMOPT_DEBUG + DBWIN）。
    if std::env::var("SYMPROBE_DEBUG").map(|v| v == "1").unwrap_or(false) {
        let ok = start_dbwin_capture();
        if ok {
            eprintln!("[symprobe] DBWIN 捕获已开启（dbghelp 诊断流将打到 stderr）");
        } else {
            eprintln!("[symprobe] DBWIN 捕获未开启（对象被占用或创建失败）");
        }
    }

    let mut dll: Option<PathBuf> = None;
    let mut work: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut filters: Vec<String> = Vec::new();
    let mut scans: Vec<String> = Vec::new();
    let mut probes: Vec<String> = Vec::new();
    let mut resolves: Vec<String> = Vec::new();
    let mut build = String::from("unknown");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dll" => {
                i += 1;
                dll = args.get(i).map(PathBuf::from);
            }
            "--work" => {
                i += 1;
                work = args.get(i).map(PathBuf::from);
            }
            "--filter" => {
                i += 1;
                if let Some(f) = args.get(i) {
                    filters.push(f.clone());
                }
            }
            "--scan" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    scans.push(s.clone());
                }
            }
            "--probe" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    probes.push(p.clone());
                }
            }
            "--resolve" => {
                i += 1;
                if let Some(r) = args.get(i) {
                    resolves.push(r.clone());
                }
            }
            "--out" => {
                i += 1;
                out = args.get(i).map(PathBuf::from);
            }
            "--build" => {
                i += 1;
                if let Some(b) = args.get(i) {
                    build = b.clone();
                }
            }
            "--help" | "-h" => {
                usage();
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数：{other}");
                usage();
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }

    let Some(dll) = dll else {
        eprintln!("缺少 --dll");
        usage();
        return std::process::ExitCode::from(2);
    };
    let work = work.unwrap_or_else(std::env::temp_dir);
    if let Err(e) = std::fs::create_dir_all(&work) {
        eprintln!("创建 --work 目录失败：{e}");
        return std::process::ExitCode::from(3);
    }

    let bytes = match std::fs::read(&dll) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读 DLL 失败：{e}");
            return std::process::ExitCode::from(3);
        }
    };

    let info = match parse_pe(&bytes) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("解析 PE 失败：{e}");
            return std::process::ExitCode::from(3);
        }
    };

    println!("[symprobe] dll             = {}", dll.display());
    println!("[symprobe] time_date_stamp = 0x{:08X}", info.time_date_stamp);
    println!("[symprobe] size_of_image   = 0x{:08X}", info.size_of_image);
    println!("[symprobe] preferred_base  = 0x{:016X}", info.preferred_base);
    println!("[symprobe] pdb             = {}", info.pdb_name);
    println!("[symprobe] pdb_signature   = {}", info.pdb_signature);
    println!("[symprobe] 节数            = {}", info.sections.len());

    let pdb = match download_pdb(&info, &work) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("获取 PDB 失败：{e}");
            return std::process::ExitCode::from(3);
        }
    };
    println!("[symprobe] pdb file        = {}", pdb.display());

    // 符号会话。即使失败也不致命：`--scan` 仍能给出候选引用点 RVA，
    // 只是拿不到函数名。这样"卡在哪一步"能被准确定位。
    let dbg = match DbgHelp::load() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("加载 dbghelp 失败：{e}");
            return std::process::ExitCode::from(3);
        }
    };
    let session = match SymbolSession::open(&dbg, &dll, &work, info.size_of_image) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[symprobe] 符号会话建立失败：{e}");
            eprintln!("[symprobe] 继续走纯读盘扫描（无函数名解析）");
            None
        }
    };

    // ---- 符号清单 ----
    let mut symbol_rows: Vec<(u64, String)> = Vec::new();
    if let Some(session) = session.as_ref() {
        match session.enum_symbols(&filters) {
            Ok((matches, total)) => {
                println!("[symprobe] 符号总数        = {total}");
                println!("[symprobe] 命中过滤        = {}", matches.len());
                symbol_rows = matches;
                symbol_rows.sort_by(|a, b| a.0.cmp(&b.0));
                for (rva, name) in symbol_rows.iter().take(60) {
                    println!("  RVA=0x{rva:08X}  {name}");
                }
                if symbol_rows.len() > 60 {
                    println!(
                        "  …… 共 {} 条命中，仅显示前 60 条（完整清单见 JSON）",
                        symbol_rows.len()
                    );
                }
            }
            Err(e) => eprintln!("[symprobe] 枚举符号失败：{e}"),
        }
    }

    // ---- 引用点扫描 ----
    let mut scan_results: Vec<ScanResult> = Vec::new();
    for needle in &scans {
        println!("\n[symprobe] ==== 扫描引用点：'{needle}' ====");
        let r = run_scan(&bytes, &info, session.as_ref(), needle);
        if r.string_rvas.is_empty() {
            println!("  UTF-16 串未在文件里找到");
        } else {
            let rvas: Vec<String> = r.string_rvas.iter().map(|v| format!("0x{v:08X}")).collect();
            println!("  字符串 RVA：{}", rvas.join(", "));
        }
        if r.sites.is_empty() {
            println!("  没有找到 RIP 相对的 lea/mov 引用点");
        }
        for (insn, target, resolved) in &r.sites {
            match resolved {
                Some((name, disp)) => println!(
                    "  引用点 RVA=0x{insn:08X} → 串 0x{target:08X}    所在函数：{name}+0x{disp:X}"
                ),
                None => println!(
                    "  引用点 RVA=0x{insn:08X} → 串 0x{target:08X}    （符号未解析）"
                ),
            }
        }
        scan_results.push(r);
    }

    // ---- 符号名求址（双向验证的另一半）----
    let mut probe_results: Vec<(String, Result<u64, String>)> = Vec::new();
    for name in &probes {
        match session.as_ref() {
            Some(session) => match session.resolve_name(name) {
                Ok(rva) => {
                    println!(
                        "[symprobe] probe '{name}' → RVA=0x{rva:08X} ✓",
                    );
                    probe_results.push((name.clone(), Ok(rva)));
                }
                Err(e) => {
                    println!("[symprobe] probe '{name}' 求址失败（{e}）");
                    probe_results.push((name.clone(), Err(e)));
                }
            },
            None => {
                println!("[symprobe] probe '{name}' 跳过（符号会话未建立）");
                probe_results.push((name.clone(), Err("符号会话未建立".into())));
            }
        }
    }

    // ---- RVA 最近邻求名 ----
    let mut resolve_results: Vec<(u32, Option<(String, u64)>)> = Vec::new();
    for spec in &resolves {
        let rva = u32::from_str_radix(spec.trim_start_matches("0x"), 16)
            .or_else(|_| spec.parse::<u32>())
            .unwrap_or(0);
        let res = session
            .as_ref()
            .and_then(|s| s.resolve_rva(rva))
            .or_else(|| {
                // 会话不可用时也提示，保证输出有明确结论。
                None
            });
        match &res {
            Some((name, disp)) => {
                println!(
                    "[symprobe] resolve RVA=0x{rva:08X} → {name}+0x{disp:X}"
                );
            }
            None => println!("[symprobe] resolve RVA=0x{rva:08X} → （无最近符号）"),
        }
        resolve_results.push((rva, res));
    }

    // ---- JSON 偏移表 ----
    if let Some(out_path) = out {
        if let Some(dir) = out_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str("  \"_note\": \"构建期由 tools/symprobe 生成；运行期必须用 build+time_date_stamp+size_of_image 三重校验后才可使用\",\n");
        json.push_str(&format!(
            "  \"module\": \"{}\",\n",
            dll.file_name().unwrap_or_default().to_string_lossy()
        ));
        json.push_str(&format!("  \"build\": \"{build}\",\n"));
        json.push_str(&format!("  \"time_date_stamp\": {},\n", info.time_date_stamp));
        json.push_str(&format!("  \"size_of_image\": {},\n", info.size_of_image));
        json.push_str(&format!("  \"pdb\": \"{}\",\n", info.pdb_name));
        json.push_str(&format!(
            "  \"pdb_signature\": \"{}\",\n",
            info.pdb_signature
        ));

        json.push_str("  \"symbols\": [\n");
        for (idx, (rva, name)) in symbol_rows.iter().enumerate() {
            let comma = if idx + 1 == symbol_rows.len() { "" } else { "," };
            json.push_str(&format!(
                "    {{\"rva\": {rva}, \"name\": {}}}{comma}\n",
                json_quote(name)
            ));
        }
        json.push_str("  ],\n");

        json.push_str("  \"string_refs\": [\n");
        let mut flat: Vec<(String, u32, u32, Option<String>, u64)> = Vec::new();
        for r in &scan_results {
            for (insn, target, resolved) in &r.sites {
                let (fname, disp) = match resolved {
                    Some((n, d)) => (Some(n.clone()), *d),
                    None => (None, 0),
                };
                flat.push((r.needle.clone(), *insn, *target, fname, disp));
            }
        }
        for (idx, (needle, insn, target, fname, disp)) in flat.iter().enumerate() {
            let comma = if idx + 1 == flat.len() { "" } else { "," };
            let fname_json = match fname {
                Some(n) => json_quote(n),
                None => "null".to_string(),
            };
            json.push_str(&format!(
                "    {{\"needle\": {}, \"insn_rva\": {insn}, \"string_rva\": {target}, \"function\": {fname_json}, \"offset\": {disp}}}{comma}\n",
                json_quote(needle)
            ));
        }
        json.push_str("  \"resolves\": [\n");
        for (idx, (rva, res)) in resolve_results.iter().enumerate() {
            let comma = if idx + 1 == resolve_results.len() { "" } else { "," };
            let (fname, off) = match res {
                Some((n, d)) => (json_quote(n), *d),
                None => ("null".to_string(), 0),
            };
            json.push_str(&format!(
                "    {{\"rva\": {rva}, \"function\": {fname}, \"offset\": {off}}}{comma}\n"
            ));
        }
        json.push_str("  ],\n");

        json.push_str("  \"probes\": [\n");
        for (idx, (name, res)) in probe_results.iter().enumerate() {
            let comma = if idx + 1 == probe_results.len() { "" } else { "," };
            let rva_json = match res {
                Ok(rva) => rva.to_string(),
                Err(_) => "null".to_string(),
            };
            json.push_str(&format!(
                "    {{\"name\": {}, \"rva\": {rva_json}}}{comma}\n",
                json_quote(name)
            ));
        }
        json.push_str("  ]\n}\n");
        if let Err(e) = std::fs::write(&out_path, json) {
            eprintln!("写 JSON 失败：{e}");
            return std::process::ExitCode::from(3);
        }
        println!("\n[symprobe] 偏移表已写入 {}", out_path.display());
    }

    std::process::ExitCode::SUCCESS
}

/// 最小 JSON 字符串转义（只处理字符串场景，够用即可）。
fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
