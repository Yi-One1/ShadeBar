# 随影 ShadeBar — 技术验证（spike）报告

| 项 | 内容 |
|---|---|
| 文档编号 | SHD-SPK-001 |
| 对应架构文档 | `docs/02-architecture.md` §12 技术验证 spike 清单 |
| 本轮范围 | **T01.1 工作区与构建骨架** + **SP-2（最高风险）** + **SP-1** |
| 执行 | 寇豆码（工程师） |
| 执行日期 | 2026-11-11（第 1 轮）；同日追加 B0 判别与 SP-2/B1 管线验证 |
| 结论摘要 | T01.1 **通过**；SP-2 **部分通过**（注入链路 + 目标模块锁定 + XAML 激活入口已验证；元素定位未完成，需转符号方案）；SP-1 **不通过但原因已定位**（`DWMSBT_MAINWINDOW` 在本 build 静默失效；backdrop 只画标题栏不填客户区；第三方工具已排除） |

> **本版修订记录（诚实标注）**
> 第 1 版 §3.3 曾判定"半透明卡片混的是 DC 残留内容，实测混成平板灰 `#B3B3B3`"。
> 追加 B0 判别实验后**该判断被推翻并已改正**：`0.70 × 255 = 178.5 ≈ #B3`，所以"平板灰"本身
> 就与**正确的 70% 白合成**一致；`--reapply` 截图进一步证明下层窗口内容能透过卡片被正确看到。
> 详见 §3.3「自我纠正」。

---

## 0. 执行环境（实测）

| 项 | 实测值 | 说明 |
|---|---|---|
| OS | Windows 11 Build **26300.9711**（注册表 ProductName 仍是 `Windows 10 Pro`，微软遗留字段） | 与需求 §3 一致 |
| 分辨率 | 2560×1440 | 单显示器 |
| DPI | 96（100%） | — |
| rustc / cargo | **1.99.1**（58cf8f9c3 2026-10-13） | 静态 CRT 可用 |
| MSVC | 14.44.35207（BuildTools 2022） | — |
| Windows SDK | 10.0.26100.0 | `rc.exe` / `dumpbin.exe` 来源 |
| explorer.exe | 第 1 轮 pid 18216（注入实验全程一致）；B0 复测时 team-lead 已重启为 pid 34492 | 均未崩溃 |
| 透明效果 | `HKCU\...\Personalize\EnableTransparency = 1` | Mica 前置条件已满足 |
| 当前进程权限 | **非管理员**（UAC `ConsentPromptBehaviorAdmin=5`，需交互确认） | 关键约束，影响 SP-2 做法 |
| 在跑干扰项 | 第 1 轮：Wallpaper Engine + TranslucentTB；**B0 复测：仅 Wallpaper Engine**（translucentTB / CTM 已被 team-lead 清理，explorer 已重启） | 见 §3.2 |

---

## 1. T01.1 工作区与构建骨架 —— **通过**

### 1.1 完成判据逐条核对

| 判据（§8.1 T01.1） | 结果 | 证据 |
|---|---|---|
| `cargo build --release` 通过，无 warning | **通过** | clean 构建 `grep -Ei "warning\|error"` 零命中；另以 `RUSTFLAGS="-D warnings"` 复核同样零警告 |
| 静态 CRT：产物不出现 `VCRUNTIME140.dll` / `MSVCP140.dll` | **通过** | `dumpbin /dependents` 只有 `api-ms-win-core-synch-l1-2-0.dll`、`kernel32.dll`、`ntdll.dll` |
| Manifest 含 `PerMonitorV2` 与 `requestedExecutionLevel` | **通过** | build.ps1 门禁 4/5 在 exe 字节流里命中两串 |
| exe 属性"文件版本 = 1.0.0.0" | **通过** | `FileVersion = 1.0.0.0`、`ProductVersion = 1.0.0.0` |
| 产物名 `随影_ShadeBar_v1.0.0.exe` | **通过** | `dist\随影_ShadeBar_v1.0.0.exe` |
| 空壳体积 ≤ 400 KB | **通过（220 KB）** | 225,280 bytes |
| 提供 `build.ps1`，一条命令完成构建 + 改名 | **通过** | `BUILD OK`，6 道门禁（新增 Gate 0 编码护栏） |

### 1.2 体积与结构

```
        2000 .data
        1000 .fptable
        2000 .pdata
        F000 .rdata        ← 60 KB
        1000 .reloc
        3000 .rsrc         ← 12 KB（图标 + 清单 + VERSIONINFO）
       22000 .text         ← 136 KB
        1000 .tls
        1000 _RDATA
```

`cargo test --release` → **15 个单测 + 2 个文档测试全部通过**。

### 1.3 已交付文件

| # (§4) | 路径 | 状态 |
|---|---|---|
| 1 | `Cargo.toml` | workspace + 4 套 profile（release / release-inject / release-l1 / release-no-en） |
| 2 | `.cargo/config.toml` | `target`、`+crt-static`、`/OPT:ICF`、**`linker = "rust-lld.exe"`**、LIB 回退 |
| 3 | `rust-toolchain.toml` | `channel = "stable"` + `x86_64-pc-windows-msvc` |
| 4 | `build.ps1` | 版本读取 → **编码护栏** → VS/SDK 探测 → 构建 → 改名 → 5 道门禁 |
| 5 | `resources/app.rc` | `ICON` + `RT_MANIFEST` + `VERSIONINFO`（版本号来自宏，不写死） |
| 6 | `resources/app.ico` | 5 尺寸 PNG-ICO（16/24/32/48/256），6145 bytes |
| 7 | `resources/app.manifest` | PerMonitorV2 / asInvoker / longPathAware / UTF-8 ACP |
| 10 | `crates/shadebar-core/src/lib.rs` | 版本与产品常量、`IPC_MAGIC`、`decision_pipe_name()`、`activate_pipe_name()` + 3 单测 |
| 11 | `crates/shadebar-core/src/error.rs` | `SbError`（24 变体）+ `i18n_key()` + `log_stem()` + 3 单测 |
| 23 | `crates/shadebar-core/src/win/mod.rs` | `to_wide` / `from_wide_lossy` / `wide!` / `HResultExt` / `win32_check` + 8 单测 |
| （必需） | `crates/shadebar-app/{Cargo.toml,build.rs,src/main.rs}`、`crates/shadebar-core/Cargo.toml` | 骨架入口，无业务逻辑 |
| （新增） | `resources/gen-app-ico.ps1` | 图标生成源（二进制资产必须可重生） |
| （新增） | `tools/check-ps1-encoding.ps1` | 编码护栏，见 §1.5 |

### 1.4 与架构文档的偏离（4 处）

| 偏离 | 架构文档写法 | 实际做法 | 理由 |
|---|---|---|---|
| **D1** | §4 #2 写"链接器路径"（隐含 MSVC `link.exe`） | `linker = "rust-lld.exe"` | Git Bash 的 `/usr/bin/link.exe` 是 coreutils 的 `link`，会抢占 MSVC 链接器并报 `link: extra operand '…rcgu.o'`。`rust-lld` 随工具链分发、版本不漂移、无需硬编码 VS 路径。**建议架构文档同步** |
| **D2** | §9.1 用 `embed-resource` 编译 `.rc` | `crates/shadebar-app/build.rs` 直接调 SDK `rc.exe`（约 60 行） | 少 5 个传递依赖；`/I` 指向 `OUT_DIR` 可精确控制生成的版本头；失败信息更可控 |
| **D3** | §4 #3 固定 `channel` | `channel = "stable"` | rustup 只安装具名工具链，写死 `1.99.1` 会让每个新环境先下载一份独立 toolchain。实测版本已记录于 §0；需要完全可复现时改一行即可 |
| **D4** | §9.1 未提 | `.cargo/config.toml` 增加 `[env] LIB`（`force = false`）兜底 | 本机无 VC 环境变量时 `rust-lld` 找不到 `kernel32.lib`。`force=false` 保证从 VS 开发者提示符或 `build.ps1` 构建时**不覆盖**真实值 |

### 1.5 编码护栏：根因与最终修法（重要，可复用）

**现象**：含中文的 `.ps1` 若无 UTF-8 BOM，Windows PowerShell 5.1 会按本地 ANSI（GBK）解码，表现为——
`param` 块被**静默吞掉**、`$PSCommandPath` 为空、甚至 `字符串缺少终止符` 的解析错误。
本轮在 `resources/gen-app-ico.ps1` 上实际踩到：ICO 生成器写出 91 字节的畸形文件，排查 6 轮才定位到是编码问题。

**关键认知（推翻了"脚本内自检就能解决"的直觉）**：
PowerShell **先整文件解析、再执行**。所以只要文件里存在"非 ASCII 且无 BOM"，解析阶段就可能已经失败，
**任何写在脚本正文里的自检都来不及运行**。实测验证：给 `build.ps1` 加上自检后，把 BOM 去掉再跑，
输出的是解析错误而不是自检的警告。

**最终修法（两层）**：

1. `build.ps1` **整文件纯 ASCII**（英文输出）。中文产品名改用码点拼装：
   `$productCn = [string][char]0x968F + [char]0x5F71`。这样 BOM 存不存在都不影响，根因消失。
   实测：去掉 BOM 后 `build.ps1` 仍正常输出 `BUILD OK`。
2. `tools/check-ps1-encoding.ps1`（**也是纯 ASCII**）作为 Gate 0 被 `build.ps1` 调用，
   扫描仓库内所有 `*.ps1`，凡是"含非 ASCII 字节且无 BOM"一律列出并以退出码 2 失败。

护栏输出示例：

```
==> Gate 0: .ps1 encoding
[check-ps1-encoding] scanned 5 .ps1 file(s)
[check-ps1-encoding] OK: every non-ASCII script has a UTF-8 BOM
```

这样"任何人重存文件搞坏编码 → 构建直接失败并点名文件"，且失败信息可读。

### 1.6 已知遗留

* `crates/shadebar-inject` 尚未创建（T05 加入 members）。`build.ps1` 用存在性检查跳过内嵌，不会假装成功。
* `main.rs` 是**最小入口**（`--version` / `--help` / 无参静默退出），不实现单实例、日志、配置、线程、消息循环——属 T04.1 / T01.2 / T01.6。双击无可见窗口是**预期行为**。

---

## 2. SP-2【全项目最高风险】26H2 上定位并改写任务栏时钟颜色 —— **部分通过**

> **一句话**：**注入链路成立（且不需要管理员权限）**、**目标模块已锁定为 `SystemTray.dll`**、
> **XAML 激活入口（公开 COM 契约）已验证可用**；但"拿到 XAML 根 → 遍历 → 写 `Foreground`"
> 仍未完成，**L2 尚不能宣布成立**。

### 2.1 结论一：注入不需要管理员权限（推翻架构假设）

```
--- OpenProcess 权限探测（决定非管理员能否注入）---
  OK   QUERY|VM_READ
  OK   QUERY|VM_READ|VM_WRITE|VM_OPERATION
  OK   QUERY|VM_READ|VM_WRITE|VM_OPERATION|CREATE_THREAD（注入所需全集）
```

原因：explorer.exe 与当前进程**同为中等完整性级别、同一用户 SID**，默认 DACL 允许；`PROCESS_VM_*`
不受 UIPI 限制（UIPI 只管窗口消息）。

**影响**：FR-20 / S-02 / FR-05 / NFR-05 / NFR-11 的"必须提权"口径可整体放宽，
UAC 弹窗可做到全生命周期 **0 次**（team-lead 已拍板采纳，见 §4 #1）。
但仍应保留"检测到目标完整性更高时按 NFR-05 降级"的兜底，**不静默提权、不弹 UAC**。

### 2.2 结论二：注入 + 远程调用导出 + 干净卸载 全部成功

```
=== SP-2 注入探针 ===
当前进程是否管理员：否
目标 explorer.exe pid = 18216
导出 ShadeBarSpikeStart 的 RVA = 0x1006
OpenProcess 取得注入全集权限
已把 DLL 路径写入远程进程（90 字节）
LoadLibraryW 已在 explorer 内返回
  远程模块 sp2_dll.dll size=0x3C000
远程模块基址 = 0x7FF87B190000
导出函数返回 = 1（1 = 成功）
证据：命名事件 Local\ShadeBar.Sp2.Hello 已存在 → 我们的代码确实在 explorer 内执行了
```

* **explorer.exe 全程未崩溃、未重启**（pid 18216 前后一致），Defender 实时保护开启下无拦截。
* 卸载：远程调用 `kernel32!FreeLibrary(hMod)` → 返回 1 → 模块从 explorer 模块表移除。

**可直接搬给 T05 的两个实现细节**：

1. 远程 `LoadLibraryW` 的**线程退出码不能用于取模块基址**（x64 指针会被截断成 u32）。
   必须 `EnumProcessModulesEx` + `GetModuleFileNameExW` 反查，再用"本地加载取导出 RVA + 远程基址"拼出远程函数地址。
2. `CreateRemoteThread` 的 `lpstartaddress` 是 `LPTHREAD_START_ROUTINE`，与 `GetProcAddress` 返回的
   `FARPROC` 布局一致，可 `transmute`；`lpparameter` 传完整指针（只有**返回值**会被截断）。

### 2.3 结论三：目标模块定位 —— 时钟/日期在 `SystemTray.dll`

只读侦察（`ReadProcessMemory` 搜模块镜像中的 UTF-16 名字，**零写操作**）：

```
Shell_TrayWnd : hwnd=0x00000000000B0388 pid=18216
共 360 个模块

  SystemTray.dll (base=0x7FF87EA80000)
      DateTimeIconContent      RVA=0x001A7626
      TimeInnerTextBlock       RVA=0x001AC050
      DateInnerTextBlock       RVA=0x001AC078
  Taskbar.View.dll (base=0x7FFF942A0000)
      BackgroundElement        RVA=0x00820D30 / 0x0088982C
      RunningIndicator         RVA=0x0088B048 / 0x0088BBC0 / …（共 10 处）
      Taskbar.TaskListButton   RVA=0x0082C060 / 0x0082E8B8 / 0x00867F40 / 0x00867F80
  twinui.pcshell.dll / Windows.UI.Xaml.dll / Microsoft.UI.Xaml.dll
      BackgroundElement（各 1 处）
```

1. **L2 目标模块 = `SystemTray.dll`** —— 架构文档 §2 D4 假设的 `twinui.pcshell.dll` / `explorer.exe` 都不是。
   T05.1 的扫描范围可从 360 个模块收窄到 **`SystemTray.dll` + `Taskbar.View.dll`（约 12 MB）**。
2. `TimeInnerTextBlock` / `DateInnerTextBlock` / `DateTimeIconContent` 三个名字在本 build 上仍以明文
   UTF-16 存在于 XBF 字符串表 → "按 `FrameworkElement::Name()` 匹配"这条版本鲁棒性最好的策略成立。
3. **L3 目标模块 = `Taskbar.View.dll`**，与 L2 **不同模块** → L3 需要**第二条独立的 hook/定位链**，
   不能按"同一棵树里找别的东西"估工（已写入 §4 #4 提醒架构师重估）。

> RVA 随版本变化，**不得硬编码**，此处仅作证据留档。

### 2.4 B1 无符号路径验证结果 —— 关键发现

team-lead 批准了"走 WinRT 公开契约、绕开内部符号"的 B1。本轮把这条链路里**可安全验证**的部分全部验证完
（新增 `scratch/sp2_xaml_locate/src/bin/xaml_abi.rs`，在**自进程**内跑，对 explorer 零风险）：

```
=== SP-2 / B1：XAML ABI 管线验证 ===
RoInitialize(MTA) -> S_OK
LoadLibraryW(Windows.UI.Xaml.dll) -> OK, base=0x7FF88C370000
DllGetActivationFactory @ 0x7FF88C517370
DllGetActivationFactory(TextBlock) -> S_OK
ActivateInstance 失败 hr=0x8001010E
```

三条结论：

1. **`DllGetActivationFactory` 这条公开契约完全可用**（`S_OK`）。说明"用运行时类名取激活工厂"不需要任何
   内部符号——这点 team-lead 的判断是对的。
2. **`ActivateInstance` 返回 `0x8001010E` = `RPC_E_WRONG_THREAD`**。含义：XAML 对象只能在**拥有 XAML 上下文
   的线程**上创建。这条发现有两层价值：
   * **确认了 §2 D5（`DispatcherHandle::try_enqueue`）是必要的，而且比原以为的更严格**——
     不只是"写属性要回到 XAML 线程"，**连创建对象都要**；
   * **否掉了"自己创建元素再塞进 shell 的树"这条路**：我们的线程永远不是 shell 的 XAML 线程。
     所以只能**拿到已存在的对象**，而"拿到"必须靠 hook 或符号解析。
3. **`windows` 0.58 **没有** `Windows.UI.Xaml` 的类型投影**（feature 列表里根本不存在 `UI_Xaml`）。
   这意味着 T05.2/T05.3 里 `IFrameworkElement` / `ITextBlock` / `IDependencyObject` 必须**手写 vtable + IID**，
   不能依赖 `windows` crate 现成绑定。**这是一笔 T05.2 的净增工作量（估 0.5~1 人日）**，请架构师计入。

### 2.5 未完成的部分与下一步（按 team-lead 批准的兜底顺序）

| 链路步骤 | 状态 |
|---|---|
| ① 注入 explorer（非提权） | **通过** §2.2 |
| ② 目标模块与元素名确认 | **通过** §2.3 |
| ③ WinRT 激活入口可用性 | **通过** §2.4 |
| ④ 拿到任务栏 XAML 根（C1/C2/C3） | **未完成** —— 卡点见下 |
| ⑤ 遍历可视树定位两个 TextBlock | **未完成**（目标已锁定） |
| ⑥ 改 `Foreground` 为纯红并肉眼确认 | **未完成** |
| ⑦ 卸载后恢复 | 仅验证"模块可干净卸载" |

> **第 2 轮（符号方案）进度见 §2.6**：② 已精确到函数级（RVA 0x2F28C）；④/⑤ 的技术路径
> 已实现并通过 hook 安装验证（卡在触发时机，见 §2.6.4）；⑥ 待启动窗口期注入。

**卡点（一句话）**：无符号路径只能拿到"类工厂"，拿不到"已存在的实例"；而实例只能来自 shell 自己的 XAML 树，
进入那棵树必须 hook 内部函数。故按预授权**转入符号方案**。

**下一步（兜底顺序，team-lead 已确认，时间盒 5 人日总量不变，B1 的 1 人日从 T05.1 预算内扣）**：

1. ~~B1 无符号路径~~ → **本轮已判定：不足以独立完成**（结论见 §2.4）。
2. **符号方案（标准工业做法，Windhawk 全系 mod 同款）**：从 `msdl.microsoft.com` 拉
   `SystemTray.dll` / `Windows.UI.Xaml.dll` 的公开 PDB，解析内部符号，定位"能拿到 XAML 岛根"或
   "每帧/每次更新都会经过且带元素引用"的函数，再 inline hook。
   * 依赖：`dbghelp`（`SymInitialize` / `SymLoadModuleEx` / `SymFromName` / `SymEnumSymbols`）；
   * 产出：按 build 号分组的符号偏移库（可缓存，避免每次联网）；
   * 与特征码方案的关系：符号只是**离线求偏移**，运行期仍用偏移 + 校验，不引入 PDB 依赖。
3. **两者都失败** → 才判定 L2 不可行，v1.0.0 只发 L1（§12 SP-2 的 B2）。

> 本轮结果让 B2 的概率进一步下降：注入可行、名字存在、模块已锁定、公开激活入口可用，
> 剩下的纯粹是"用哪种 hook 进入 XAML 树"的工程选择问题。

### 2.6 符号方案实测（第 2 轮，2026-11-11）—— **定位链路全部打通，端到端改色卡在"模板只在构建期应用一次"**

> 一句话：**`SystemTray.pdb`（publics-only）+ 自研 `symprobe` 成功把时钟元素定位到
> `winrt::SystemTray::implementation::DateTimeIconContent::OnApplyTemplate`（RVA 0x2F28C），
> inline hook 安装/恢复全绿、explorer 存活；但已构建的任务栏在主题切换 / 任务栏设置切换下
> 都不会重新应用该模板，端到端改色需要在 explorer 启动窗口期注入（待主理人批准后做）。**

#### 2.6.1 符号工具链（`tools/symprobe`，零第三方依赖）

自研构建期工具（纯 Win32 FFI + 系统 `dbghelp.dll`，1200 行，零 warning），流程：

```
解析 PE → 取 TimeDateStamp + SizeOfImage + CodeView(RSDS) 签名
       → msdl 按 GUID+Age 下载 PDB（HTTP 200，MSF 魔数校验）
       → LoadLibraryExW(IMAGE_RESOURCE) 映射映像 → SymLoadModuleExW(映射基址)
       → SymEnumSymbolsW 枚举（实测 33,631 个公开符号）
       → --scan: UTF-16 字符串 RVA → 扫 RIP 相对 lea/mov 引用点 → 最近邻符号反解所属函数
```

实测踩坑与修法（每条都有对应的失败证据）：

| # | 坑 | 症状 | 修法 |
|---|---|---|---|
| 1 | `PSYM_ENUMERATESYMBOLS_CALLBACKW` 是**三参数原型**（首参 `SYMBOL_INFOW*`） | 回调参数整体错位：ctx 收到 size、size 收到常量 → 对齐 panic | 照 SDK `DbgHelp.h` 修正为 `(PSYMBOL_INFOW, ULONG, PVOID)`；加 EXPECTED_CTX 防御 |
| 2 | dbghelp"文件式模块"（base=0）枚举/查址内部 0xC0000005 | `SymEnumSymbolsW`/`SymSearchW`/`SymFromAddrW` 全灭 | 先 `LoadLibraryExW(IMAGE_RESOURCE\|DATAFILE)` 把映像映射进进程，取页对齐基址（句柄低位标记 bit1=0x2，掩 `!0xFFF`）再 `SymLoadModuleExW` |
| 3 | `fInvadeProcess=1` 与文件式模块互相干扰 | 枚举 AV | `SymInitializeW(..., FALSE)` |
| 4 | `SYMOPT_EXACT_SYMBOLS` | `SymFromAddrW` 报 87 | 不加 |
| 5 | 搜索路径 `\\?\` 前缀 | PDB 名拼接产出非法路径 | 手动剥前缀 |
| 6 | `SystemTray.pdb` 是 **publics-only 剥离版**（11MB，`GlobalSymbols=0 LineNumbers=0`，dbghelp 报 "public symbols"；对照 `windows.ui.xaml.pdb` 354MB 全量版 `GlobalSymbols=1 LineNumbers=1`） | `SymFromAddrW` 恒 87 | 用"全量枚举 + 最近邻匹配"替代按址查询，同样能反解所属函数 |
| 7 | Windows 11 机器普遍**没有 DIA 运行时**（System32 无 msdia，Common Files 只有 msdia90/100） | PDB 名义绑定但无符号 | 与本方案无关（dbghelp 自带 publics 解析器够用）；全量 PDB 场景需带 msdia140.dll |
| 8 | `SYMOPT_DEBUG` 诊断流 | 需 DBWIN 监听（与其它进程的 OutputDebugString 有竞态丢行） | symprobe 内置 `SYMPROBE_DEBUG=1` + 进程内 DBWIN 捕获线程 |

#### 2.6.2 定位结论（证据链完整、双路交叉一致）

```
TimeInnerTextBlock (串 RVA 0x1AC050)
  引用点 RVA 0x2F2EB → DateTimeIconContent::OnApplyTemplate+0x5F
DateInnerTextBlock (串 RVA 0x1AC078)
  引用点 RVA 0x2F311 → DateTimeIconContent::OnApplyTemplate+0x85
OnApplyTemplate RVA = 0x2F2EB - 0x5F = 0x2F28C = 0x2F311 - 0x85  ← 两路互洽
```

* `--resolve 0x2F2EB / 0x2F311`（最近邻）与 `--scan`（字节模式）两条独立算法给出同一函数。
* 函数头 15 字节（文件读出）：`48 8B C4 48 89 58 10 48 89 70 18 48 89 78 20 55`——
  全部 RIP 无关，可安全搬运作 trampoline；**无热补丁头**（非 `mov edi,edi` 型）。
* 偏移表已落盘 `data/symbols/26300.9711.json`（三重校验三元组 + RVA + 引用点 + 证据链，
  未来适配新 build 只追加一条）。

#### 2.6.3 运行期验证（两步制的第一步：hook 本身全绿）

`scratch/sp2_xaml_locate`（原型 crate，不进主干）新增：

* **DLL 侧**（`sp2_dll.dll`，导出 `ShadeBarHookInstall` / `ShadeBarHookUninstall` / `ShadeBarHookPing`）：
  1. `GetModuleHandleW("SystemTray.dll")` → 读**内存中**的 TimeDateStamp + SizeOfImage；
  2. 三重校验（build 号人工 + 两项内存实读），任一不符 → 安全放弃（实测改坏预期值即拒绝安装）；
  3. 逐字节核对函数头 15 字节 → 不符安全放弃；
  4. trampoline（原 15 字节 + 绝对跳回 entry+15）→ 挂起宿主全部线程（Toolhelp 枚举 + SuspendThread）
     → 写 14 字节 `mov rax,imm64;jmp rax` → FlushInstructionCache → 恢复线程；
  5. hook 内先调原函数（trampoline），再一次性走查可视树（`IVisualTreeHelperStatics::GetChild/GetChildrenCount`
     + `IFrameworkElement::get_Name` + `ITextBlock::put_Foreground`，全部手写 vtable，
     IID 取自 `System32\WinMetadata` 经 .NET WinRT 投影反射——与 windows-rs 同源，非手抄）。
  6. 证据事件（命名内核对象）：`Installed` / `HookFired` / `Found.TimeInnerTextBlock` /
     `Found.DateInnerTextBlock` / `Colored`。
* **宿主侧**（`inject_hook.exe`）：远程三重校验先行 → 注入 → Ping（**RVA 指纹校验**，防旧模块乱址）→
  Install → 触发 → 轮询事件 → `--restore-bytes` 兜底清理（不经任何 DLL 直接恢复字节）。

实测结果（全部在非提权会话）：

| 步骤 | 结果 |
|---|---|
| 注入 + Ping 指纹 | ✓ `0x5F324B4F` |
| 宿主侧三重校验 | ✓（0x6AA9E58A / 0x0020A000，远程内存实读） |
| DLL 安装 hook | ✓ 返回 1，entry 补丁实测在位（`48B8…FFE0` 读回确认） |
| 卸载/恢复字节 | ✓ 读回 `48 8B C4 48 89 58 10 …` 与原件逐字节相同 |
| **explorer 存活** | ✓ 全程 Responding=True，任务栏正常（含一次 explorer 自然重启也无残留） |
| OnApplyTemplate 被调用 | **✗ 未发生**（HookFired 无） |

#### 2.6.4 卡点：已构建的任务栏不重应用时钟模板

**先自曝一个实现 bug（已被本轮实测 + 编译器告警双重暴露并修复）**：第一版 hook 把
"入口补丁的跳转目标"写成了 trampoline 地址（应为 hook 函数本体——trampoline 是 hook
调原函数用的，两者写反）。表现为"原函数正常执行、hook 永不触发"（explorer 安全无损）。
编译器以 `on_apply_template_hook` dead_code 告警暴露；修复为 v3 后**重测全部触发源**，
结论不变，因此下表结论对"接线正确"的 hook 成立。

触发源逐一实测（全部可逆、全部还原；hook 补丁在位经读回核实）：

| 触发源 | 结果 |
|---|---|
| `WM_THEMECHANGED` → Shell_TrayWnd | 未触发 |
| 深浅色主题切换（AppsUseLightTheme 0↔1 + `ImmersiveColorSet` 广播，往返还原） | 未触发 |
| 任务栏对齐切换（TaskbarAl 0↔1 往返还原） | 未触发 |
| 秒数显示切换（ShowSecondsInSystemClock，已删除还原） | 未触发 |

结论：`DateTimeIconContent::OnApplyTemplate` 在任务栏构建时执行一次后，上述常规事件都
不会让它再次运行。端到端验证需要**在 explorer 启动窗口期注入**（explorer 重启 + 竞速注入，
或 IFEO/计划任务等启动注入方案），涉及重启 explorer（任务栏短暂消失），**须主理人批准**。

> 附带发现：本机时钟文字在实验开始时呈**深灰近黑**（深色任务栏上几乎不可读），
> 主题往返切换后恢复为正常白色。该状态与 SP-2 要解决的问题（任务栏元素配色不可控）一致，
> 说明"改 Foreground"路线的产品价值真实存在。
>
> 环境复原清单：主题/任务栏注册表全部还原、TTB 全程存活且其 `desktop: clear` 配置生效、
> SystemTray.dll 字节逐字节还原（读回核对）、ShowSecondsInSystemClock 已删除。
> 残留：sp2_dll_v2/v3/v4.dll 仍驻留 explorer（全部无 hook、无行为——v4 经回归验证
> 安装/读回/恢复闭环正常，函数头字节已逐字节还原；驻留 DLL 随下次 explorer 重启自然消失）。

#### 2.6.5 对 SP-2 判定的影响

* **符号方案（B3）判定：通过**。三重校验 + 偏移表 + 定位链路全部按约束 #1/#2/#3 落地，
  且在真实 26300.9711 上验证。
* **L2 端到端（⑥ 改色肉眼确认）**：未完成——卡点不是技术失败，而是**触发时机**工程问题。
  已知可行路径（explorer 启动窗口期注入）需主理人批准后实施；若批准，预计 0.5 人日内可出端到端证据。

### 2.7 第 3 轮实测（竞速注入 + 排查链，2026-11-11 下午）—— **竞速机制 100% 装通、hook 确认触发；端到端改色被两个独立阻断卡住，全部留证**

> 一句话：**CREATE_SUSPENDED 布防式注入让 hook 在时钟构建前 100% 装上并触发
> （OnApplyTemplate 实测启动期触发 1467 次）；透传 hook 在无 TranslucentTB 时稳定运行 5 分钟+；
> 但 XAML 可视树走查（无论栈内还是定时器延迟）会致 explorer 崩溃（0x2F2B5，确定性），
> 且 hook 与 TranslucentTB（ExplorerTAP.dll）共存时连透传都会崩。改色证据未能产出。**

#### 2.7.1 竞速机制（第一线）—— **机制层 100% 成功**

| 项 | 结果 |
|---|---|
| 普通竞速（杀 explorer → 轮询 pid → 注入） | 失败：install 在 SystemTray 加载后 ~3s 才落地，输给模板应用（trial 1） |
| **CREATE_SUSPENDED 布防式**（主线程挂起 → 注入 → 派 install 线程（DLL 内轮询等模块）→ 放行） | **install 100% 成功**（trials 2/3/4/5/14/15/17/18/19/20，无一失败），patch 必然落在模板应用前 |
| Hook 触发 | **确认触发**：HookFired 事件 + 事件日志崩溃定位到 hook 覆盖的函数内部（反证触发）+ colorCount=1467（启动期 OnApplyTemplate 触发 1467 次，与构造器调用次数吻合） |
| 透传稳定性（--no-walk，无 TTB） | trial 14：**5 分钟+ 无崩溃**（explorer Responding=True） |
| 端到端改色 | **未达成**（见 2.7.2/2.7.3） |

注入耗时背景：远程 LoadLibraryW 在挂起进程上完成耗时 ~15.7s（原因未查，不影响正确性——
install 线程在放行前已布防；产品化时需优化或改用更早的注入原语）。

#### 2.7.2 阻断一：XAML 走查致崩（确定性，与时机无关）

| 对照 | 结果 |
|---|---|
| 透传 hook（--no-walk），无 TTB | 稳定 |
| 栈内走查 + 改色（trials 3/4） | 崩（0x2F2B5） |
| **定时器延迟走查**（3s 后 UI 线程消息循环里走查，trial 17/18） | **仍崩**（延迟 3s~数分钟后崩溃） |
| **纯计数走查（--no-color：只 QI/GetChild/get_Name，无 brush 无 put_Foreground）**（trial 20） | **仍崩** —— 走查原语本身有问题，不是改色写 |
| 栈内改色走查 + 无 TTB（trial 15） | 崩 |

崩溃签名（Windows 事件日志，7+ 次全部一致）：`SystemTray.dll!0x2F2B5`（= OnApplyTemplate 的
security-cookie 写栈指令 `mov [rbp+37h],rax`，0xC0000005）。**干净 explorer（无 hook）从未崩过。**

未竟诊断：DLL 内置 VEH（向量化异常处理器）把异常参数写进共享段，宿主预持段句柄——
**VEH 从未被调用**（环形缓冲 0 条，含所有异常码），原因未明，属待查异常。走查致崩最可疑的
根因：**IFrameworkElement::get_Name 的 vtable 槽位（33）由 .NET WinRT 反射方法序推断，
可能与元数据声明序不符 → 错位调用 → 状态破坏 → 下一次 ApplyTemplate 崩在 cookie 写**。
下一步修法：从 publics 里的 `produce_base<...>::vftable` 符号离线反汇编 vtable 槽位做硬核对。

#### 2.7.3 阻断二：与 TranslucentTB（ExplorerTAP.dll）共存崩（确定性）

| 对照 | 结果 |
|---|---|
| 透传 hook + TTB 在场 | **崩**（trials 2/5/7：~60-100s 后 0x2F2B5，事件日志 pid 一一对应到实验 explorer） |
| 透传 hook + **TTB 已杀** | **稳定**（trial 14：5 分钟+ 无崩溃） |

→ 我们对 OnApplyTemplate 的 patch 与 ExplorerTAP 对 SystemTray.dll 的 patch 相互冲突
（TTB 恢复运行后确认其正常）。**正式产品必须处理共存**：检测 ExplorerTAP.dll 在场 →
降级/协调/换 hook 点，否则用户装了 TTB 就必崩。

#### 2.7.4 第二线：替代 hook 点侦察（部分完成，候选已锁定）

实测数据 + 符号面候选（RVA 已入库 `scratch/st_probe.json`）：

| 候选 | RVA | 特性 |
|---|---|---|
| `winrt::SystemTray::implementation::ClockSystemTrayIconDataModel2::ResetTimer` | 0x8D2C8 (905224) | 分钟级定时器重置 = **运行期高频**；走 DataModel→ViewModel 链 |
| `SystemTrayResources_obj{101,276,285,291,89,95}_Bindings::Update_ViewModel_SecondaryText` | 0x170DDC/0x170E6C/0x170F08/0x170F98/0x171048/0x1710E4 | **XAML 绑定更新函数，属性变更即触发（时钟文本每分钟更新）且直接持有 TextBlock 引用** —— 最有希望的运行期改色点 |
| `DateTimeIconContent::DateTimeIconContent`（构造器，诊断探针用） | 0x2E31C | 启动期触发 1467 次（实测） |
| `ActivateLocalType<DateTimeIconContent>` | 0x137CF0 | 类激活探针 |

注：OnApplyTemplate 是**启动期高频（1467 次）但运行期静默**（上一轮已证：主题/对齐/秒数切换均不触发）；
`Update_ViewModel_SecondaryText` 一族才是"每分钟都跑"的运行期链路。频率实测与可行性
评估（--race-probe 计数）留待下一轮。

#### 2.7.5 环境复原（第 3 轮）

主题/任务栏注册表原值；TTB 已恢复运行（pid 33436，transparent 态正常）；SystemTray.dll
字节多次验证原样；当前 explorer 为自动重启后的干净实例（无 hook）。用户无打开的文件夹窗口
（实验前已记录，为空）。sp2_dll.dll 残留驻留于部分实验 explorer，随崩溃/重启已全部清除。

#### 2.7.6 对架构的影响判断（含建议）

1. **竞速注入可行性：成立**。CREATE_SUSPENDED 布防式让 hook 100% 赶在时钟构建前，
   透传稳定性已证（无 TTB 时）。开机自启场景下与 explorer 赛跑的窗口更大（登录后 explorer
   由 winlogon 拉起，可用计划任务"登录即启动 + 高优先级轮询"——但 15.7s 注入耗时必须先解决）。
2. **两个硬阻断**：(a) XAML 走查致崩（疑似槽位错，修法明确）；(b) TTB 共存崩（需检测+降级或换点）。
3. **建议产品路线（混合）**：主路 = 运行期注入 + hook `Update_ViewModel_SecondaryText` 一族
   （不用竞速、自带 TextBlock 引用）；启动期竞速作为兜底。优先级：先修走查槽位核对（VEH 静默一并查），
   再验证 Update_ViewModel_SecondaryText 的运行期频率与可行性，最后处理 TTB 共存策略。

---

## 3. SP-1 GUI：Mica + Direct2D 自绘共存 —— **不通过（原因已定位，且不是环境问题）**

> **一句话**：清掉 TranslucentTB / CTM 后复测，**Mica 仍然不出**——它们不是元凶。
> 真凶有两条，都是**可操作的工程结论**：
> ① `DWMSBT_MAINWINDOW`(2) 在 build 26300 上**静默失效**，而 `DWMSBT_TRANSIENTWINDOW`(3) /
> `DWMSBT_TABBEDWINDOW`(4) **生效**；
> ② DWM backdrop **只画在标题栏（frame），从不填充客户区**，与是否调用
> `DwmExtendFrameIntoClientArea` 无关。

### 3.1 通过判据逐条核对

| 判据（§12 SP-1） | 结果 | 证据 |
|---|---|---|
| ① Mica 透出卡片外区域可见 | **不通过** | §3.3 / §3.4 |
| ② 文本清晰、无彩色描边 | **通过** | 灰度抗锯齿生效 |
| ③ 拖动/缩放不闪白 | **部分通过** | 无闪白，但重复绘制会累积不透明度（§3.6） |
| ④ 单帧绘制 < 4 ms（720×560） | 未测 | 待 T03.1 正式渲染上下文 |
| （附加）Mica 是否依赖第三方干预 | **已排除** | §3.2 |

### 3.2 干净环境复测：TranslucentTB / CTM **不是**元凶

team-lead 清理后的环境实测（全部为只读检查）：

```
explorer pid                     = 34492（已重启，原 18216）
ExplorerTAP.dll (TranslucentTB)  = 不在模块表
CTMTMTAP.dll (CTM)               = 不在模块表
 EnableTransparency              = 1
HighContrast Flags               = 126（bit0 = 0 → 高对比度**关闭**）
DWM 策略键                       = 无禁用项（只有 GPU 黑名单等默认值）
GPU                              = AMD Radeon RX 6600 XT (VEN_1002 DEV_73FF)，**不在** DWM 黑名单
                                   （黑名单条目全是 10DE = NVIDIA 设备 ID）
电池                              = 无（台式机），排除"节能模式关透明效果"
```

属性回读（`docs/spikes/sp1_run.log`）：

```
DwmExtendFrameIntoClientArea(-1) -> Ok(())
DwmSetWindowAttribute(SYSTEMBACKDROP_TYPE=2) -> Ok(())
diag: DwmGetWindowAttribute(SYSTEMBACKDROP_TYPE) -> Ok(()), readback=2
diag: DwmGetWindowAttribute(WINDOW_CORNER_PREFERENCE) -> Ok(()), readback=2
diag: exstyle=0x00000100, WS_EX_NOREDIRECTIONBITMAP=false
diag: DwmIsCompositionEnabled -> enabled=true
diag: GetWindowCompositionAttribute(WCA_ACCENT_POLICY) -> 0（0 = 未设置过 accent policy）
```

**客户区像素采样 —— 必须带"区域基线"对照，否则读数会被位置误导**

第 3 轮第一次采样得到 `#FFFFFF`，第二次（换位置）得到 `#202020`，两者矛盾。原因是**只看窗口内部无法判断"这块像素是谁画的"**。补上"同区域、无窗口"的基线后结论才唯一：

| 采样 | 同区域**无窗口**基线 | `backdrop=2` + 不绘制 | `backdrop=4` + 不绘制 |
|---|---|---|---|
| (400,300) | `#202020` | **`#202020`（完全相同）** | `#FFFFFF` |
| (100,200) | `#141414` | **`#141414`（完全相同）** | — |
| (700,400) | `#202020` | **`#202020`（完全相同）** | — |

→ **`backdrop=2` 时客户区是"完全透明"的：像素值与"这里根本没有我们的窗口"逐字节相同。**
即 backdrop 没有在客户区画出任何东西，我们看到的是**下层窗口**。

> **勘误（第 3 轮内部）**：第 3 轮初稿曾写"客户区是纯 `#FFFFFF` = DWM 默认背景"。复查发现那次采样点正好压在一个白色窗口上，
> `#FFFFFF` 是**下层窗口的颜色**而不是我们的背景。已改正为"透明"。这也说明**没有基线对照的像素采样不可信**。

（另：`GetWindowCompositionAttribute` 是未文档化 API，**不在 `user32.lib` 导入库里**，
直接 `#[link]` 声明会链接期报 undefined symbol，必须 `GetProcAddress` 动态解析。）

### 3.3 关键发现一：`DWMSBT_MAINWINDOW`(2) 在本 build 上静默失效，3 / 4 生效

改成可指定 backdrop 取值后逐一实测，采样**标题栏**像素（客户区不受 backdrop 影响，见 §3.4）：

| `DWMWA_SYSTEMBACKDROP_TYPE` | 标题栏采样 | 是否渲染 |
|---|---|---|
| 1 = `DWMSBT_NONE` | `#F3F3F3` | — |
| **2 = `DWMSBT_MAINWINDOW`（Mica）** | **`#F3F3F3`** | **否（与本 none 完全相同）** |
| 3 = `DWMSBT_TRANSIENTWINDOW`（Acrylic） | `#D8DAD8` | 是 |
| 4 = `DWMSBT_TABBEDWINDOW`（Mica Alt） | `#D9DADC` | 是 |

同机对照：**记事本**（`SYSTEMBACKDROP_TYPE = 4`）标题栏实测 `#D9DADC` —— 与我把值设为 4
得到的结果**完全一致**，反过来印证"取值 4 的渲染路径在本机是通的"。
另外 `DwmGetWindowAttribute` 对 2 与 4 都回读为设定值 → **DWM 收下了参数，但 2 不产生视觉**。

> 这是本轮最有价值的一条：以后在 bug 报告里出现"我设了 MAINWINDOW 但没效果"，
> 先换成 4 试一次，能立刻区分"我们写错了"和"这个 build 的 MAINWINDOW 有问题"。

**补充：染色不随位置变化。** 把窗口分别放到屏幕三个差异很大的区域（100,100 / 1500,700 / 900,900），
标题栏分别为 `#DADADB` / `#D9DADC` / `#D9DADC`，**差异 ≤ 1/255**。
说明本机的 backdrop 染色**不采样壁纸**（大概率因为壁纸由 Wallpaper Engine 渲染在独立的 WorkerW
窗口里，DWM 的 Mica 采样器看不到它，于是退化成纯主题色）。
→ 这也解释了为什么之前"压在深色区应该变暗"的判别思路会误导我（见 §3.5）。

### 3.4 关键发现二：backdrop 只画标题栏，**从不填充客户区**

| 组合 | 客户区（与区域基线对照后） |
|---|---|
| `backdrop=2` + 不绘制 | **透明**（像素与"无窗口基线"逐字节相同） |
| `backdrop=2` + 不绘制 + `--no-extend` | 不透明（`#FFFFFF`，窗户外无基线可比） |
| `backdrop=4` + 不绘制 | 不透明（`#FFFFFF`），但**不是**壁纸染色 |
| `backdrop=4` + 绘制（α=0 清屏 + `AlphaBlend`） | α=0 区域**透出下层窗口**（实测透出白色记事本），**不是 Mica** |

关键推断：若客户区拿到了 backdrop，透明区域应当显示 **Mica**；实测显示的是**下层窗口**，
说明合成链是"我们的表面 → 桌面其它窗口"，backdrop 不在其中。

→ **`DwmExtendFrameIntoClientArea(-1)` 并不能把 backdrop 拉进客户区**，与架构文档 §2 D1 第 2 步的假设相反。
→ 顺带说明：**`backdrop=4` 时客户区是不透明 `#FFFFFF`，也不是 Mica**（纯白不可能是 Mica 的输出）；

### 3.5 自我纠正（沿用第 2 版，仍然有效）

第 1 版曾判定"半透明卡片混的是 DC 残留内容，实测混成平板灰 `#B3B3B3`"。**该判断已撤回**：

* 数值上 `0.70 × 255 = 178.5 ≈ 0xB3`，"平板灰"与**正确的 70% 白合成**一致，不能作为证据；
* `--reapply` 截图显示窗口压在另一个应用上方时，**下层应用的文字能透过卡片被清楚看到**
  → **DWM 对预乘 alpha 的合成是正确的**。

同时第 2 版的"深色壁纸区判别法"也被证明有缺陷：**Mica 采样的是壁纸，不是下层窗口**，
而本机壁纸由 Wallpaper Engine 渲染且染色不随位置变化，所以"压到深色区应该变暗"这个预期本身不成立。
**正确的判别法见 §3.7。**

### 3.6 仍然成立的真实缺陷：重复绘制导致 α 累积

`WM_ERASEBKGND` 返回 1（架构建议）→ 窗口 DC 保留上一帧 → 每次 `AlphaBlend` 都在旧内容上再叠一层，
n 帧后等效不透明度 `1 − 0.3ⁿ`（2 帧 91%、3 帧 97%），表现为卡片越来越白、文字发虚。
**与 Mica 无关，任何自绘方案都必须解决。**

**修法（核心原则：每帧必须从"干净表面"重新合成，绝不在上一帧结果上叠加）**

| 上屏方案 | 怎么做到"干净表面" |
|---|---|
| **B3：客户区不透明纯色底** | 每帧先 `FillRect` 填底色（或 `Clear` 到不透明色）再画控件。完全不产生 alpha 累积，因为目标表面从不保留上一帧的透明信息 |
| **B/DComp：swapchain** | 交换链每帧拿到的是**全新 backbuffer**，天然干净。但要注意"每帧只 `BeginDraw`/`EndDraw` 一次"，别在同一 backbuffer 上重复绘制 |
| **（已否决）GDI `AlphaBlend` 到窗口 DC** | **无法满足**：`WM_ERASEBKGND→TRUE` 意味着不会有人替我们清屏，DC 内容跨帧保留，累积不可避免。这也是这条路被否决的第二条理由 |

另有一条通用纪律：**`WM_ERASEBKGND` 的返回值要与上屏方案配套决定**。
"吞掉擦除"只在"DWM 负责画背景"时成立；一旦我们自己负责客户区背景，就必须显式清屏
（返回 0 让系统擦除，或自己 `FillRect`），否则累积 + 残影都会出现。

### 3.7 判别方法（修正版，可在任何机器上 5 分钟复现）

1. **先判"backdrop 有没有渲染"**：不要看客户区，看**标题栏**。把同一次运行分成
   `--backdrop 1` / `2` / `4` 三组截图，采样标题栏任意一点。三者颜色若相同 → 该 build 上这个取值没生效。
2. **再判"Mica 是否采样壁纸"**：把窗口拖到屏幕三个差异很大的位置，比较标题栏颜色。
   差异明显 → 真 Mica；几乎不变 → 只是主题染色（本机属于后者，与 Wallpaper Engine 有关）。
3. **最后判"客户区拿没拿到 backdrop"**：用 `--no-paint` 与默认（绘制）两组，采样卡片外的 α=0 区域。
   显示**下层窗口** → 没拿到 backdrop；显示**模糊/染色** → 拿到了。
4. 需要硬数字就用 `sample-pixels.ps1`（见 §6 工具清单）。

### 3.8 结论与 Plan B（已按新证据重排）

**结论**：`DWMSBT_MAINWINDOW` 在本 build 失效 + backdrop 不进客户区 →
**"GDI 自绘客户区 + Mica 背景"这条组合在当前实现下不成立**。

| 方案 | 内容 | 成本 | 可行性 |
|---|---|---|---|
| **B3（新，最划算，推荐 v1.0.0 采用）** | **客户区不透明纯色底（浅 `#F3F3F3` / 深 `#202020`）+ 标题栏 Mica（`backdrop = 4`）**。实测标题栏染色可用，客户区回到可靠的 `FillRect` 不透明绘制 | **≈0.5 人日** | **完全可行**。既拿回一部分 Fluent 观感，又一次性消灭 §3.6 的 α 累积缺陷；不需要 DComp |
| **B1（客户区也要 Mica）** | `WS_EX_NOREDIRECTIONBITMAP` + `D3D11` + `CreateSwapChainForComposition`（`DXGI_ALPHA_MODE_PREMULTIPLIED`）+ `IDCompositionDevice/Visual`；D2D 改 `ID2D1DeviceContext` | `render.rs` 420→~800 行；内存 +2~4 MB（仍在 20 MB 预算内） | 技术可行，但**本 build 上必须用 backdrop 3 或 4**（2 失效），且真 Mica 采样依赖壁纸可见性 |
| **B2（退路）** | 纯色底 + `backdrop = 1`，完全不碰 backdrop | 0 人日 | 可行，但白白放弃标题栏 Mica |

**建议**：v1.0.0 走 **B3**（0.5 人日，风险最低），把"客户区 Mica 背景"记入 README 已知限制；
B1 作为 v1.1 的增强项，届时用 backdrop=4 + swapchain 实现。

### 3.9 对 FR-40 的影响判断

FR-40 拆成两项分别定级：

* **Mica 标题栏** —— **可交付**（`backdrop = 4`，实测有染色），照常验收；
* **Mica 客户区背景** —— v1.0.0 **记入 README 已知限制**（D-04 已要求列已知限制；FR-40 属 L4/P1，
  按 Q-02 不阻塞发布）。

**另需提醒团队的一条环境相关限制**：如果用户使用 Wallpaper Engine 一类"独立进程渲染动态壁纸"的工具，
Mica 可能退化为纯主题色（本机实测染色不随壁纸位置变化）。这一点应写进 README 的"已知限制"，
与"需管理员权限 / 可能被杀软误报 / 第三方托盘图标不改色"并列。

## 4. T03.1 渲染骨架（按 B3 定稿落地）

> B3 = 客户区**不透明纯色** + 标题栏 **DWM `backdrop = 4`**；⛔ 不用 GDI `AlphaBlend` 上屏、
> ⛔ 不调 `DwmExtendFrameIntoClientArea`。定稿依据见 §3.8。
>
> **实现已对齐架构 v1.4 冻结规格**：上屏用 `ID2D1HwndRenderTarget`（`D2D1_ALPHA_MODE_IGNORE`），
> 无自建 DIB 后缓冲、无 `BitBlt`/`AlphaBlend`、无 DirectComposition。
> （首版曾用 `ID2D1DCRenderTarget` + DIB + `BitBlt(SRCCOPY)`，功能等价且验收数据完全相同；
> 为与冻结文档一致已改掉，避免留下"代码与规格不一致"的隐患。）

### 4.1 交付文件

| 文件 | 内容 |
|---|---|
| `crates/shadebar-core/src/color.rs`（新增） | `Rgba` + `parse_hex` + `premul_f32`/`straight_f32`。§11.5 规定的颜色唯一定义处；感知亮度/对比度/直方图仍属 T01.3 |
| `crates/shadebar-app/src/render.rs`（新增） | `RenderCtx`（D2D DC 目标 + DWrite + 复用 DIB）、`RectDip`、`TextRole`、`dip_to_px`、`paint_skeleton_frame` |
| `crates/shadebar-app/src/window.rs`（新增） | 窗口创建、B3 DWM 配置 + 回读校验、WndProc（`WM_PAINT`/`WM_ERASEBKGND`/`WM_SIZE`/`WM_GETMINMAXINFO`/`WM_DPICHANGED`/`WM_SETTINGCHANGE`/`WM_CLOSE`/`WM_DESTROY`） |
| `crates/shadebar-app/src/ui/{mod,theme}.rs`（新增） | `ThemeMode` + `Palette`（浅 `#F3F3F3` / 深 `#202020`）+ Fluent 度量常量 + `system()` 读 `AppsUseLightTheme` |
| `crates/shadebar-app/Cargo.toml` | 增加 `windows` 依赖（D2D/DWrite/Gdi/Dwm/HiDpi/Registry…），版本仍锁 `=0.58.0` |
| `crates/shadebar-app/src/main.rs` | 新增开发入口 `--ui`（T03 阶段验收用，T04.1 接正式启动流程后移除） |
| `tools/verify-t03-window.ps1`（新增） | T03.1 验收脚本：启动 → 按窗口类名取 HWND → 截屏 → 采 5 个点的像素 |

### 4.2 B3 的六条实现要点（写进了代码注释，评审时可机械核对）

1. `DWMWA_SYSTEMBACKDROP_TYPE = 4`（**不用 2**，本 build 上 2 静默失效）；
2. `DWMWA_WINDOW_CORNER_PREFERENCE = 2`（圆角由 DWM 提供）；
3. **不调** `DwmExtendFrameIntoClientArea`（实测无效，不留死代码）；
4. 上屏用 **`ID2D1HwndRenderTarget` + `D2D1_ALPHA_MODE_IGNORE`**：D2D 直接画到窗口，
   不参与 α 混合，从渲染目标层面就不可能污染窗口表面的 alpha 通道（也不需要自建 DIB）；
5. 每帧**先铺满整幅不透明背景**再画内容 → 天然满足"每帧从干净表面重新合成"，
   §3.6 的 α 累积缺陷不成立。尺寸变化走 `HwndRenderTarget::Resize`，无自定义缓冲重建；
6. `WM_ERASEBKGND` 返回 1，且 `WM_PAINT` **必定铺满整幅客户区**——两者必须成对成立，
   否则会出现"吞掉擦除但只画了一部分"的残影；
7. **设备丢失兜底**：`EndDraw` 返回 `D2DERR_RECREATE_TARGET`（GPU 复位 / 显示器切换 / RDP 断连）时，
   `RenderCtx::end_frame` 会重建渲染目标并返回 `Ok(true)`，`window.rs` 据此 `InvalidateRect` 重画一帧
   —— 否则窗口会一直空白。这是 `HwndRenderTarget` 相对 DIB 路径新增的唯一失败模式，必须处理。

### 4.3 一处**与架构文档相反**的设计决定（需要同步）

架构 §2.2 D1 第 6 步写的是「`WM_NCCALCSIZE` 返回 0（非客户区归零），标题栏由我们自绘」。

**本实现没有这么做，保留了 DWM 标准标题栏。** 原因是这两者互斥：

* B3 的"Mica 标题栏"正是 DWM 画在 **frame（非客户区）** 上的材质；
* 一旦把非客户区归零并自绘标题栏，那块材质就被客户区盖掉 —— **B3 唯一的 Mica 收益归零**；
* 保留标准标题栏后，观感也更接近资源管理器 / 设置应用（这正是 team-lead 定 B3 的理由）。

**建议架构文档把 D1 第 6 步改为**："保留 DWM 标准标题栏（不接管 `WM_NCCALCSIZE`），
以便 frame 承载 `backdrop = 4` 材质；页面级标题作为客户区内容自绘。"
若将来 v1.1 改走 DComp，再恢复"自绘标题栏"才有意义（那时材质来自 swapchain 而非 frame）。

### 4.4 验收实测（`tools/verify-t03-window.ps1`）

窗口 760×600，浅色主题，截图 `docs/spikes/t03-b3-window.png`：

```
=== pixel samples ===
  caption (DWM)          (380,12)  = #DADADB   ← DWM 标题栏材质已渲染
  client bg              (380,588) = #F3F3F3   ← 我们的不透明 FillRect 可见
  card fill              (380,300) = #FFFFFF   ← 卡片填充正确
  left margin            (6,300)   = #D5AA7E   ← 窗口外圆角/阴影区（壁纸），符合预期
  page heading           (380,46)  = #F3F3F3   ← 标题区背景正确
```

**回答了 team-lead 提出的关键疑问**："`backdrop=4` 会让客户区有个不透明 `#FFFFFF` 底，
会不会盖掉我们的 `FillRect`？"
**答案：不会。** 实测客户区采样是 `#F3F3F3`（我们的填充），不是 `#FFFFFF`（backdrop 的底），
说明我们的绘制稳定地在其之上。`BitBlt` 覆盖式上屏与 D2D `α=IGNORE` 的组合不受那个底影响。

其余验收项：标题栏中文标题/客户区两行中文正文均清晰、**无彩边、无重影**（灰度抗锯齿 + 不透明整幅重绘），
窗口四角为 DWM 圆角，无未绘制区域。体积 **220 KB → 236.5 KB**（D2D/DWrite 引入 +16.5 KB），
仍在 400 KB 软目标与 4096 KB 硬门禁内。

**上屏路径切换的回归验证**：从 `DCRenderTarget + DIB + BitBlt` 改为 `ID2D1HwndRenderTarget` 后，
用同一个验收脚本重跑，上表五个采样点取值**逐字节完全一致**（`#DADADB` / `#F3F3F3` / `#FFFFFF` /
`#D5AA7E` / `#F3F3F3`）→ 切换无视觉回归，且少了一整套 DIB 管理代码。

### 4.5 明确未做（属后续任务，非占位）

* 布局 / 滚动 / 控件库 / 六个分组页面 / 托盘 / 语言热切换 → T03.2 ~ T03.6；
* `WM_CLOSE` 目前是"销毁并退出"，T03.6 改为"最小化到托盘"（A-09）；
* `--ui` 是开发入口，T04.1 接入单实例 + 托盘后会移除；
* 无任何日志输出（`core/log.rs` 属 T01.6）；错误通过进程退出码与 `stderr` 体现。

## 5. 对上游文档的建议改动（供架构师 / 产品经理评审）

| # | 落点 | 现状 | 建议改成 | 依据 | 状态 |
|---|---|---|---|---|---|
| 1 | §1.4 / FR-20 / NFR-05 / S-02 / FR-05 / NFR-11 | "L2 需要管理员权限" | "L2 默认不需要管理员权限；目标完整性更高时按 NFR-05 降级，不静默提权"；可做到全流程 0 次 UAC | §2.1 | **team-lead 已采纳**，待产品经理/架构师同步文档 |
| 2 | §2 D4 / §12 SP-2 定位链 | 目标含 `twinui.pcshell.dll` / `explorer.exe` | **L2 主目标 = `SystemTray.dll`**；**L3 目标 = `Taskbar.View.dll`**；L3 需独立定位链，工时需重估 | §2.3 | **team-lead 已采纳** |
| 3 | T05.1 特征码表 | 覆盖三个模块 | 收窄到 2 个模块（约 12 MB），并新增"按名字搜 XBF 字符串"作为自检 | §2.3 | 待同步 |
| 4 | T05.2 任务描述 | 假定可用 `windows` crate 的 XAML 绑定 | **`windows` 0.58 无 `UI_Xaml` 投影**，`IFrameworkElement`/`ITextBlock` 必须手写 vtable + IID；T05.2 需 **+0.5~1 人日** | §2.4 | **新增，请架构师计价** |
| 5 | §2 D5 DispatcherHandle | "禁止在非 XAML 线程**写** XAML 属性" | 补充"**创建** XAML 对象同样受线程亲和性限制（实测 `RPC_E_WRONG_THREAD`）" | §2.4 | **新增** |
| 6 | §2 D1 第 5/7 步 / §12 SP-1 | `WM_ERASEBKGND→TRUE` + 预乘 DIB + `AlphaBlend` 上屏，并假定 `DwmExtendFrameIntoClientArea(-1)` 能把 Mica 拉进客户区 | **该假定已被实测否定**：backdrop 只画标题栏，客户区始终拿不到。改为 **B3**（客户区不透明纯色 + 标题栏 `backdrop=4`），客户区 Mica 留到 v1.1 用 DComp 实现；"吞掉 WM_ERASEBKGND + AlphaBlend" 的 α 累积缺陷必须配套解决 | §3.4 / §3.6 / §3.8 | **请架构师据此改 D1** |
| 10 | §2 D1 backdrop 取值 / T03.1 | 一律用 `DWMSBT_MAINWINDOW`(2) | **build 26300 上 2 静默失效**（标题栏 `#F3F3F3`，与 none 相同）；3(acrylic) / 4(Mica Alt) 生效（`#D8DAD8` / `#D9DADC`）。建议默认用 **4**，并在代码里保留可配置 | §3.3 | **新增，请同步** |
| 11 | README 已知限制（D-04） | 已列 4 条 | 追加两条：① 客户区 Mica 背景 v1.0.0 不支持；② 使用 Wallpaper Engine 等"独立进程渲染动态壁纸"工具时，Mica 可能退化为纯主题色 | §3.9 | **新增** |
| 12 | §2.2 D1 第 6 步 | "`WM_NCCALCSIZE` 返回 0，标题栏由我们自绘" | **改为保留 DWM 标准标题栏**：B3 的 Mica 材质画在 frame 上，自绘标题栏会把它盖掉、等于扔掉 B3 唯一收益。页面级标题作为客户区内容自绘 | §4.3 | **已按此实现，请同步文档** |
| 7 | §4 #2 | "链接器路径" | 显式写 `linker = "rust-lld.exe"`，并注明 Git Bash `link.exe` 抢占问题 | §1.4 | 待同步 |
| 8 | §9.1 | `embed-resource` 为 build-dep | 可删；`shadebar-app/build.rs` 直接调 `rc.exe` | §1.4 | 待同步 |
| 9 | FR-40 | Mica 属 L4/P1 | 追加"若 SP-1 结论为不通过，Mica 记入 README 已知限制，不阻塞 v1.0.0" | §3.5 | 待同步 |

---

## 6. 本轮未做 / 需要授权

| 项 | 为什么没做 | 需要什么 |
|---|---|---|
| ~~SP-1 B0：退出 TranslucentTB 后重跑~~ | **已完成**（team-lead 清理环境 + explorer 重启后复测） | 结论见 §3.2 |
| SP-1：真 Mica 采样是否受 Wallpaper Engine 影响 | 需要临时停掉 Wallpaper Engine 才能验证（红色风险：会改变用户壁纸） | 若要闭合该问题需授权；不阻塞结论，已在 §3.9 记为已知限制 |
| SP-2 符号方案 | 属 T05.1 正式工作量，需独立批次 | team-lead 已预授权，下一批执行 |
| SP-3 / SP-4 / SP-5 | 不在本轮范围（主理人限定 T01.1 + SP-1 + SP-2） | 后续批次 |
| 提权路径回归 | 本会话无法提权（UAC 需交互确认） | 非必需：非提权路径已成功 |

**原型残留说明**：注入的 `sp2_dll.dll` 已远程 `FreeLibrary` 卸载干净；该 DLL 在 explorer 内
`CreateEventW` 创建的**事件对象句柄**仍留在 explorer（原型为便于外部验证刻意未关句柄）。
纯内核对象、无泄漏放大，随 explorer 退出消失。主干实现会显式关闭句柄。

---

## 7. 证据文件索引

| 文件 | 内容 |
|---|---|
| `docs/spikes/sp1-mica.png` | 变体 A：Mica + 绘制 |
| `docs/spikes/sp1-solid.png` | 变体 B：无 Mica + 绘制（文字锐利无彩边） |
| `docs/spikes/sp1-nopaint.png` | 变体 C：Mica + 不绘制（近纯白，无模糊壁纸） |
| `docs/spikes/sp1-nopaint-solid.png` | 变体 D：无 Mica + 不绘制（与 C 相同 → Mica 未生效） |
| `docs/spikes/sp1-dark-mica.png` / `sp1-dark-solid.png` | 判别：压在深色壁纸区仍为近纯白 |
| `docs/spikes/sp1-stdframe.png` | 判别：保留标准框架，仍近纯白 |
| `docs/spikes/sp1-after-show.png` | 判别：窗口显示后再设 backdrop，仍近纯白 |
| `docs/spikes/sp1-diag.png` | 属性回读 `readback=2` 那一轮的截图 |
| `docs/spikes/sp1-reapply.png` | 失效范围：重设 backdrop 后 Mica 未恢复；同时证明半透明合成正确 |
| `docs/spikes/sp1_run.log` | DWM 调用返回码 + 全部 `diag:` 回读值 |
| `scratch/sp2_xaml_locate/` | `recon`（只读侦察）、`inject_probe`（注入）、`unload`（卸载）、`xaml_abi`（B1 ABI 验证）、`sp2_dll`（被注入 DLL） |
| `scratch/sp1_mica_d2d/` | SP-1 原型（含 `--no-paint` / `--std-frame` / `--reapply` / `--no-mica` 实验开关）+ `shot.ps1` |
| `docs/spikes/sp1-clean-nopaint-mica.png` / `sp1-clean-nopaint-none.png` | **干净环境**复测：客户区均为 `#FFFFFF` → 第三方工具不是元凶 |
| `docs/spikes/sp1-clean-dark-mica.png` / `sp1-clean-dark-none.png` | 干净环境 + 深色区判别 |
| `docs/spikes/sp1-bd3.png` | `backdrop=3`（Acrylic）：标题栏 `#D8DAD8`（有染色） |
| `docs/spikes/sp1-bd4.png` | `backdrop=4`（Mica Alt）：标题栏 `#D9DADC`（有染色） |
| `docs/spikes/sp1-bd4-painted.png` | `backdrop=4` + 绘制：α=0 区域透出**下层窗口**而非 Mica |
| `docs/spikes/sp1-noextend-nopaint-mica.png` | 跳过 `DwmExtendFrameIntoClientArea`：客户区仍 `#FFFFFF` |
| `docs/spikes/sp1-pos100x100.png` / `sp1-pos1500x700.png` / `sp1-pos900x900.png` | 三个位置标题栏色差 ≤1/255 → 染色不采样壁纸 |
| `docs/spikes/sp1-ref-notepad.png` | 同机对照：记事本 `backdrop=4`，标题栏 `#D9DADC`（可作"backdrop 4 渲染路径正常"的基准） |
| `docs/spikes/ref-settings.png` | 反例留档：「设置」应用实测 `backdrop=1`(NONE)，**不能**用作 Mica 对照 |
| `tools/check-ps1-encoding.ps1` | 编码护栏（Gate 0） |
| `scratch/sp1_mica_d2d/{shot.ps1,shot-window.ps1,probe-backdrop.ps1,sample-pixels.ps1}` | 截图 / 窗口截图 / DWM 属性回读 / 像素采样，四个可复用工具 |
| `docs/spikes/t03-b3-window.png` | **T03.1 验收截图**：DWM 标题栏材质 + `#F3F3F3` 客户区 + `#FFFFFF` 圆角卡片，中文正文无彩边无重影 |
| `tools/verify-t03-window.ps1` | T03.1 验收脚本（启动 → 定位窗口 → 截屏 → 采 5 点像素），换 build/换机器时重跑 |

## 8. 方法论沉淀（可复用，建议进团队规范）

### 8.1 像素采样的基线原则

**任何"这个颜色是我的窗口画出来的"判断，都必须先有"没有我的窗口时这里是什么颜色"作对照。**

本轮的实证：第一次采样客户区得到 `#FFFFFF`，换位置又得到 `#202020`，两者矛盾；
补上"同区域、无窗口"的基线后发现，`#202020` 与基线**逐字节相同** → 客户区其实是**完全透明**的，
`#FFFFFF` 只是采样点正好压在另一个白色窗口上。**若没补基线，整个 SP-1 的结论方向都会错**
（会误判成"不透明纯白背景"，进而错判 backdrop 是否生效）。

操作要求：
1. 采样点必须记录**屏幕绝对坐标**，而不是"窗口内相对坐标"；
2. 先截"没有探针窗口"的同区域基线，再截"有探针窗口"，两次坐标完全一致；
3. 判定"某像素是否由我的窗口产生" = 与基线**逐字节比较**，相等即说明窗口在该处**没有画任何东西**；
4. 需要判断"半透明合成是否正确"时，基线必须选**有可辨识内容**的背景（有文字的窗口优于纯色）。

### 8.2 "API 返回成功 + 属性回读正确" ≠ 效果生效

本轮最贵的一课：`DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW)` 返回 `S_OK`，
`DwmGetWindowAttribute` 回读也是 `2`（正是设定值），**但屏幕上什么也没有**。
同一台机器上把值换成 `3`/`4` 立刻出现材质 —— 说明**参数被接收 ≠ 被实现**。

因此形成纪律：
* **调 DWM / DComp / XAML 这类"效果类"属性，必须用像素采样做视觉验证**，不能只看返回值，
  也不能只看"属性回读一致"；回读只能证明"写进去了"，不能证明"画出来了"；
* 代码里若要对效果做断言，断言的措辞必须是"属性已写入"，**不得**写"效果已生效"；
  本项目的 `window.rs::apply_b3_backdrop` 就按此措辞实现（回读失败才报错，视觉由验收脚本负责）；
* 换 build / 换机器时要重跑验收脚本 —— 本条坑（`DWMSBT_MAINWINDOW` 静默失效）就是版本相关的。

### 2.8 第 4 轮实测（vftable 核对 + 主路首探 + TTB 侦察，2026-11-11 晚）

> 一句话：**①槽位全部核对正确（走查崩的根因排除槽位，头号嫌疑改为悬垂 `this` UAF）；
> ②VEH 管线自检通过（explorer 内静默是环境特异现象）；③运行期注入机制成功
> （不重启不竞速、TTB 在场），但绑定函数探针 ~20s 后栈溢出型崩溃，频率数据未取得；
> ④ExplorerTAP 无代码 patch（共存崩不是 patch 碰撞）；⑤ExplorerTAP 检测开关就绪。**

#### 2.8.1 槽位硬核对（winmd 权威源）

自研 ECMA-335 解析器（`scratch/sp2_xaml_locate/winmd_method_order.py`）直接解析
`C:\Windows\System32\WinMetadata\Windows.UI.Xaml.winmd` 的 TypeDef/MethodDef 表，
输出权威方法声明序（`winmd_slots.txt`）。结论：

| 接口方法 | 反射推断槽位 | winmd 权威槽位 | 判定 |
|---|---|---|---|
| IFrameworkElement::get_Name | 33 | **33**（idx 27+6） | 一致 |
| ITextBlock::get/put_Foreground | 18/19 | **18/19** | 一致 |
| IVisualTreeHelperStatics::GetChild/GetChildrenCount | 10/11 | **10/11** | 一致 |
| ISolidColorBrush::put_Color | 7 | **7** | 一致 |

→ 走查崩溃根因**排除槽位错位**。头号嫌疑改为：**悬垂 this（UAF）**——定时器走查用的
是首次触发时捕获的元素指针，而启动期 1467 个瞬态实例大多已析构，3 秒后对已释放对象
QI/走查 = 堆破坏 → 崩溃落在下一次 ApplyTemplate 的 cookie 写。这也自然解释了"走查后
延迟数秒到数分钟崩溃"与"纯计数走查也崩"。**正式产品的正确姿势：元素引用必须"当次有效
当次使用"（绑定函数 hook 正是如此），严禁跨时点持有。**

#### 2.8.2 VEH 之谜（已查清一半）

自检（--veh-selftest，宿主进程内）：EnableVeh=1 → 受控 panic → **环形计数 delta=1** →
VEH 管线代码完全正常。explorer 内静默的原因排除我们的实现问题，指向环境特异现象
（本会话环境有 sandbox-center 等内核层介入，可能在用户态 VEH 之前拦截异常）。
正式产品不能依赖 VEH 做崩溃取证，需换方案（如 MiniDumpWriteDump 于
UnhandledExceptionFilter + 独立看护进程读共享段——段句柄存活策略已验证可行）。

#### 2.8.3 主路首探（运行期注入机制成功，频率数据未取得）

- **运行期注入成功**：--install-at 0x170DD6:7 对**当前运行中 explorer**（TTB 在场）安装
  近跳转探针，返回 1，全程无重启无竞速——机制层突破。
- 关键工程发现：Update_ViewModel_SecondaryText 的 publics RVA（0x170DDC）**指向函数中间**
  （+6 处），真函数起点 0x170DD6（prologue 仅 4 字节）；且其 15 字节窗口内含 FF 15
  （RIP 相对 call），标准 15 字节拷贝不可用 → 自研**近跳转安装**（±2GB 内 VirtualAlloc
  试探 + E9 rel32 5 字节 patch + 拷 7 字节原指令），含参数捕获（RCX/RDX + this 前 48 qword
  写共享段）。publics RVA 指到函数中间的现象对本批全部候选成立，**离线核对指令边界
  已成为必要工序**（工具已具备）。
- **阻断**：探针 ~20s 后崩（SystemTray.dll+0x170DD8 = 该函数自己的 sub rsp,28h 上，
  栈溢出型）——绑定更新链天然递归较深，探针 2 帧额外开销压垮栈。频率数据未取得。
  修法方向：hook 换成"叶函数"（绑定链最末端、无递归的 set_Text/SetValue 落点），
  或去掉 capture 的 48-qword 读（疑似越界读）后仅计数复测。

#### 2.8.4 TTB 侦察（--memdiff，只读）

| 发现 | 内容 |
|---|---|
| **ExplorerTAP 无代码 patch** | SystemTray.dll 代码区（含 OnApplyTemplate 0x2F28C 附近）内存=磁盘，零差异 |
| 差异区全部为正常加载行为 | 0x192000-0x194000（IAT 解析，48 FF 25 import thunk 区）、0x199000+（重定位/.data 初始化）、0x1B8000+（BSS/字符串区） |
| 结论 | **共存崩不是 patch 碰撞**——TTB 走 XAML/Composition API 干预任务栏外观，与我们的 hook 冲突是行为级（时机/事件订阅/资源竞争），不是字节级 |
| 待深挖 | IAT 重定向检测（对比 IAT 实际值与被导入模块导出地址）；TTB 的 XAML 干预点 |

--memdiff 工具 + ExplorerTAP.dll 在场检测（打印"受限模式"开关）已实现并入宿主，
正式产品可直接复用（代码：inject_hook.rs memdiff_mode）。

#### 2.8.5 更新后的建议

1. **主路（Update_ViewModel 一族）仍是正确方向**：运行期注入机制已通（本轮最关键突破），
   剩余问题收敛为"探针自身开销压垮递归栈"——换叶函数 hook 或轻量化探针即可，工程量小。
2. **悬垂 this 教训写进规范**：任何元素引用必须当次有效当次使用（架构 §11 需补一条）。
3. **TTB 共存**：无 patch 碰撞 → 优先怀疑行为级冲突；产品策略维持"检测 ExplorerTAP 在场 →
   受限模式"，具体降级动作（跳过 hook / 只挂不写）待主路跑通后实测。
4. VEH 取证换方案（不依赖用户态 VEH）。

### 2.9 第 5 轮追加（起点检测纠错 + 主路复测，2026-11-11 深夜）

> 一句话：**近跳转探针崩溃的根因已找到—— publics RVA 指向函数中间导致 patch 打错位置；
> 修正起点复测仍崩（新签名：匿名可执行页，VEH 依旧沉默），主路频率数据仍未取得。**

#### 2.9.1 起点检测纠错（2.8.3 阻断的根因）

离线分析（CC-padding 回扫 + E8 直接调用图 + .data/.rdata 绝对指针引用扫描）结论：

| 变体 | publics RVA | 真起点 | 前 12 字节指针引用 | 判定 |
|---|---|---|---|---|
| obj101 | 0x170DDC(+8) | **0x170DD4** | 0 | **唯一安全** |
| obj276 | 0x170E6C(+44) | 0x170E40 | 2 | 不安全（多入口） |
| obj285 | 0x170F08(+8) | 0x170F00 | 2 | 不安全 |
| obj291 | 0x170F98(+8) | 0x170F90 | 2 | 不安全 |
| obj89 | 0x171048(+40) | 0x171020 | 2 | 不安全 |
| obj95 | 0x1710E4(+4) | 0x1710E0 | 2 | 不安全 |

上轮探针打在 0x170DD6 = `48 83 EC 28`（sub rsp,28h）**指令中间** → 调用者执行完 sub
前 3 字节后直接踩进 E9 rel32 垃圾流 → 必崩。**近跳转机制无罪，起点检测错了。**

#### 2.9.2 修正起点复测：仍崩，签名改变

`--install-at 0x170DD4:12`（正确起点、拷 12 字节、无指针引用）安装成功（返回 1），
但 ~2 分钟后崩，且签名变化：

* 崩溃模块 = **unknown**，RIP = 0x7FF8C3A6828F（裸地址，不在任何已命名模块内）
  ——指向**匿名可执行页**，即我们的 trampoline 页（VirtualAlloc 无名分配）附近/内部执行。
* VEH 环形缓冲依旧 0 条（与 2.8.2 的自检通过矛盾——explorer 环境内异常分发确实异常）。
* NEAR_COUNT 首读 = 0（探针从未被调用过就崩了——patch 落地即崩，与调用无关）。

**判断**：修正起点后，patch 本身不再破坏指令流（计数=0 说明探针没跑），但 explorer 仍在
~2 分钟后崩在 trampoline 附近——指向**近分配页与本环境（sandbox-center 内核介入）的
执行兼容性问题**，或仍存在未发现的间接引用。剩余排查需要真调试器（本环境 VEH 失效、
无法安全附加），超出时间盒。

#### 2.9.3 状态与建议

| 项 | 状态 |
|---|---|
| 运行期注入机制（install/uninstall/恢复闭环） | ✅ 已验证 |
| 起点检测工具（CC 边界 + 调用图 + 指针引用扫描） | ✅ 已建成并纠错 |
| 主路频率/引用/改色 | ⛔ 卡在"修正后仍崩、需调试器级排查" |
| TTB patch 清单 + 检测开关 | ✅ 已完成（2.8.4） |

建议：主路的下一步必须先解决"explorer 内崩溃不可观测"（接 WinDbg 内核调试或换
环境复测），否则每次试错都是盲调。工具链（近跳转安装、起点检测、memdiff、受限开关）
已全部就绪，换环境或接调试器后可立即续作。

### 2.10 第 6 轮收口（ACG 鉴别 + 对照实验 + 改色终判，2026-11-11）

> 一句话：**ACG 已排除；近跳转结构经对照实验证伪（装到已知安全函数同崩）；改色管线
> 在 OnApplyTemplate 期间走查被证伪（原函数栈破坏）。L2 判定：技术成立、证据受限。
> 端到端像素证据降级为人工验收项（一键脚本已交付）。**

#### 2.10.1 鉴别实验①：进程缓解策略（ACG 排除）

新增零依赖探针 `mitig_probe`（GetProcessMitigationPolicy，PROCESS_QUERY_LIMITED_INFORMATION 即可）：

| 进程 | DynamicCode(ACG) | SystemCallFilter | CFG 策略 |
|---|---|---|---|
| explorer（被注入目标） | ProhibitDynamicCode=**0** | DisallowWin32k=0 | 0x0 |
| 本会话进程 | 0 | 0 | 0x0 |
| 全新 notepad | 0 | 0 | 0x0 |

**结论：ACG 未开启**，「匿名页执行被内核拦截」假说排除。（ExtensionPointDisable 查询
返回 87，记录在案未深究，与本轮症状无关。）

#### 2.10.2 鉴别实验②：纯计数复测 + 对照实验（近跳转结构证伪）

1. **纯计数复测**（删 48-qword 读 + 参数捕获全停用，`--race-near 170DD4:12` 新模式）：
   安装成功，**≤5 秒崩，count=0**。捕获读嫌疑排除。
2. **WER 取证**（事件日志 1000）：异常代码 **0xC0000409**（fail-fast 家族：GS/CFG/CET
   共用），出错模块 **unknown** @ 0x7FF8C3A6828F（匿名执行页内）。fail-fast 是内核级
   不可拦截异常——**这同时解释了 VEH 全程静默**（不是环境分发异常，是根本没机会分发）。
3. **对照实验**（决定性）：把同一套近跳转结构装到**已知 10/10 存活的 OnApplyTemplate**
   （0x2F28C，前 11 字节 = 3 条完整指令、无 RIP 相对、纯计数）：**同样 ≤5 秒崩，
   count=0，同 fail-fast 签名**。

| 假说 | 结果 |
|---|---|
| 0x170DD4 目标特异（间接引用/多入口） | ❌ 对照同崩，排除 |
| 捕获读越界 | ❌ 纯计数同崩，排除 |
| ACG 拦匿名页执行 | ❌ 策略值=0，排除 |
| **近跳转（E9 两页）结构本身在本环境触发 fail-fast** | ✅ **成立** |

**注意：14 字节绝对跳转（48 B8 addr FF E0）单页结构 10/10 存活**，两者唯一的结构差异
是 E9 直跳 vs 绝对跳转及两页布局。fail-fast 的具体触发器（CFG 位图 / IBT / 沙箱内核层
代码完整性检查）需要内核调试器定论，超出时间盒，按既定决策停止盲调。

#### 2.10.3 改色终判：OnApplyTemplate 期间走查被证伪

栈内走查收口尝试（this 当次有效当次使用，无 UAF；重入保护 + 100 次上限齐备）竞速实测：

* HookFired ✅，colorCount=1467（全部 OnApplyTemplate 触发），走查 runs/normal/hits 均有计数
* **explorer 在 t≈25.8s 崩：AV 0xC0000005 @ SystemTray.dll+0x2F2B5（OnApplyTemplate+0x29，
  原函数 cookie 写处）**——走查在模板应用进行中重入 XAML，破坏原函数栈。

**UAF 教训（按要求入档，结构性结论）**：
1. **元素引用必须 valid-when-used**：XAML 元素指针（this/TextBlock）是瞬态对象，启动期
   有 1467 个实例被创建销毁，任何「先捕获、后使用」（定时器延迟走查）都是 UAF。
2. **叶函数的结构优势成立但被环境否决**：叶函数 hook 内拿引用当次使用，理论上是唯一
   无 UAF 的形状（引用在调用栈存活期内使用完即弃）——但其依赖的近跳转结构已证伪（2.10.2）。
3. **OnApplyTemplate 内部不是安全走查点**：模板应用进行中重入可视树（哪怕引用当次有效）
   仍破坏原函数栈（cookie 写 AV）。改色必须发生在模板应用**完成之后**的稳定态。

#### 2.10.4 环境特异发现（影响所有进程级实验的解读）

**AI 沙箱在命令会话结束时收割全部子进程**：由会话内命令启动的 explorer 在命令结束
瞬间被干净终止（无 WER 事件），userinit 自动重启的 explorer 才能存活。本轮多个
「explorer 消失」事件由此而来，非崩溃。**解读实验数据时必须区分「崩溃（有 WER）」
与「被沙箱收割（无 WER）」**。桌面恢复需依赖 userinit 自动重启或用户手动操作。

#### 2.10.5 L2 最终判定与移交

| 项 | 判定 |
|---|---|
| 定位链（symprobe → RVA → 引用 → 双算法互证） | ✅ 成立 |
| 14 字节绝对跳转 hook 结构 | ✅ 成立（10/10） |
| 近跳转结构（叶函数所需） | ⛔ 本环境 fail-fast 证伪（是否沙箱特异待真实环境复测） |
| OnApplyTemplate 期间改色 | ⛔ 证伪（2.10.3） |
| **L2 总判定** | **技术成立、证据受限** —— 按「v1.0.0 发 L1」收口 |
| 端到端像素证据 | 降级为人工验收项：`tools/manual-sp2-verify.ps1`（一键：注入→hook→改色→像素采样→截图→自动还原→报告，含 Phase A 叶探针 / Phase B 改色像素轮询 / Phase C 还原） |

**TTB 共存策略建议**（基于 2.8.4 + 本轮）：SystemTray.dll 零字节级冲突（memdiff 已证），
冲突是行为性的；L1 采样方案不碰 SystemTray.dll 进程空间，天然共存，无需检测开关。

**移交流程**：T02（采样与判定引擎）启动，任务与判据照架构 §8.2（T02.1 任务栏几何、
T02.2 抓屏采样器、T02.3 UIA 遮罩、T02.4 采样线程闭环）。SP-2 遗留件：近跳转 fail-fast
需真实环境复测（脚本 Phase A 已内置）；若复测通过，叶函数改色可续作。

---

## 9. T02 采样与判定引擎（L1 核心）—— **代码级通过，真机效果待 T04 接线**

> **证据级别声明（重要，遵循「API 返回成功 ≠ 效果生效」纪律）**：
> 本轮交付的是 **`shadebar-core` 内可单元测试的引擎逻辑 + 平台能力 trait**。
> 所有判据验证使用**注入的假几何 / 假像素 / 假时钟**，属于**逻辑正确性证据**，
> **不是**真机 `FindWindowW` / `BitBlt` / `IUIAutomation` 的效果证据。
> §9.3 里凡标「结构性」的条目均需 T04 接线后按 A-29 方式补真机对照，不得据本轮
> 数据声称端到端生效。整个批次**未重启 explorer、未注入任何 DLL、未杀 TranslucentTB、
> 未改注册表、未动用户窗口**；只读采样链路与用户桌面环境零交互。

### 9.1 交付文件清单

| # | 文件 | 行数 | 职责 |
|---|---|---|---|
| 40 | `crates/shadebar-core/src/engine/mod.rs` | 1268 | 模块汇总、`Engine` 生命周期、配置快照、`RectPx` 几何工具、常量、测试夹具 |
| 41 | `crates/shadebar-core/src/engine/taskbar.rs` | 535 | T02.1 `TaskbarProbe`/`TaskbarGeom`/`TaskbarSource`、`locate`、`compute_sample_rect` |
| 42 | `crates/shadebar-core/src/engine/screen.rs` | 429 | T02.2 `ScreenBuffer` 复用 DIB、`ScreenSource`、屏幕裁剪、缩略图 |
| 43 | `crates/shadebar-core/src/engine/uia.rs` | 405 | T02.3 `UiaProbe` 超时熔断 + 固定几何 fallback、`ClockRectSource` |
| 44 | `crates/shadebar-core/src/engine/mask.rs` | 364 | T02.3 `MaskSet` 固定容量合并 / 2 px 膨胀 / 命中 / 有效像素 |
| 45 | `crates/shadebar-core/src/engine/sampler.rs` | 1304 | T02.2/T02.4 直方图 P50、EWMA 迟滞判定、`SamplerLoop` 闭环状态机 |
| — | `crates/shadebar-core/src/engine/testing.rs`（新增） | 653 | `#[cfg(test)]` 全套假实现 + 假时钟 / 假等待器 / 假停止令牌 |
| — | `crates/shadebar-core/src/lib.rs`（改） | +69 | `pub mod engine` + `#[cfg(test)]` 线程局部 `GlobalAlloc` 分配计数器 |
| — | `build.ps1`（改） | +2 | 修 PS 5.1 下 `Get-ChildItem \| Remove-Item` 管道绑定失败（见 §9.5） |

**与架构 §4 的路径偏离**：清单 #40–#45 原写在 `crates/shadebar-app/src/engine/`，
本轮按 team-lead 指派落在 `shadebar-core`。理由是 `lib.rs` 明确 core「无 UI、无窗口、
无线程」且 §11.7 要求「单测不得依赖真实屏幕 / explorer」；把几何与算法放 core 才能
脱离 Windows 消息循环单测。真实 Win32 / COM 适配由 T04 在 `shadebar-app` 实现并注入。

**与架构 §6.2 的签名偏离**（均为可注入化所需，已逐个在代码文档注释里标注）：
`locate(source)`、`fallback_clock_rects(g) -> [MaskRect;2]`（零分配定长数组，非 `Vec`）、
`refresh_clock_rects(source, clock, geom)`、`sample_once(screen, buffer, hist, masks, geom, cfg)`
拆开 `ScreenSampler`、`run_sampler_thread` 改为 `Engine::run(&mut deps, stop, sleeper)`
同步循环（core 不 spawn 线程，T04 用 `CreateWaitableTimerW` 在 T1 驱动）。

### 9.2 T02.1 五条硬判据逐条对照

| # | 判据 | 实现 / 单测 | 结果 |
|---|---|---|---|
| ① | 采样矩形 100% 落在 `TrayNotifyWnd` 内，误差 ≤ 4 px（A-02） | `compute_sample_rect` 用 `rect_clamp` 把期望矩形夹回 `tray`，再夹到屏；`a02_sample_rect_is_fully_inside_tray_within_4px` 覆盖 5 组托盘尺寸 × 偏移/宽度/内缩，`expected_sample_rect` 与实算四边误差 | ✅ **逻辑通过**：误差恒 0 px（≤4）；越界配置（`x_offset=90,width=100`）也被夹回 |
| ② | 自动隐藏时 `is_usable()==false`（EX-06） | `TaskbarGeom::is_usable` + `ex06_auto_hide_makes_geometry_unusable`；`engine_auto_hide_suspends_and_keeps_last_decision` 断言挂起期间**不新增发布**、时间戳不更新 | ✅ **逻辑通过**：`Suspended(AutoHidden)` + 保持上次判定 |
| ③ | 竖向任务栏 → 暂停采样 + 提示一次（EX-07） | `horizontal = bar_w >= bar_h`；`emit_notice_once` 只发一次；`engine_vertical_taskbar_suspends_and_notifies_exactly_once` 连跑 6 拍断言 `notices==1`、`decision.count()==0` | ✅ **逻辑通过**，提示文案本身属 T01.7 i18n，未做 |
| ④ | 多显示器只取含任务栏的屏（NFR-07） | `choose_screen` 优先「完整包含 tray 的主屏」，否则最大交集；`clip_rect_to_screens` 主屏优先（**不因副屏交集更大而误采**）；`nfr07_multi_monitor_uses_only_the_screen_with_taskbar` / `nfr07_taskbar_on_secondary_screen_is_supported` / `clip_partial_overlap_prefers_primary_screen` | ✅ **逻辑通过**：3 屏横排（左副/中主/右副）取中屏；任务栏搬到左副屏也能正确切换 |
| ⑤ | 100↔150% DPI 切换后 ≤2 s 重算（NFR-08 / EX-22） | `needs_refresh_from` 比对 DPI/矩形/折叠/拓扑指纹；`Engine::on_dpi_changed` + 每拍探测；`nfr08_dpi_change_is_detected_and_recomputed`、`ex22_monitor_hotplug_changes_topology_fingerprint`、`engine_dpi_change_recomputes_within_two_seconds` | ✅ **逻辑通过**：重算延迟 = 相邻两拍间隔（默认 1 s，实测 1000 ms ≤ 2000 ms）；显式 `WM_DPICHANGED` 入口已留 |

### 9.3 需求基线验收项对照（A-02 / EX-06 / EX-07 / NFR-07 / NFR-08 / EX-22 为主）

| 项 | 结论 | 证据 | 证据级别 |
|---|---|---|---|
| **A-02** 采样框落在托盘区 ≤4 px | ✅ 逻辑通过 | `rect_contains(tray, got)` 恒真，四边误差 0 px | 假几何；**诊断面板可视化 + 截图比对未做**（T03.5/T04） |
| **EX-06** 自动隐藏暂停并保持上次色 | ✅ 逻辑通过 | 挂起期间 `decision.count()` 不变 | 假状态 |
| **EX-07** 侧边任务栏降级 + 提示 | ✅ 逻辑通过 | `notices==1`，零发布 | 假状态，提示文案待 T01.7 |
| **NFR-07 / EX-08** 多显示器不错位 | ✅ 逻辑通过 | 3 屏 + 副屏任务栏两组用例；主屏优先裁剪 | 假几何 |
| **NFR-08 / EX-22** DPI ≤2 s 重算 | ✅ 逻辑通过 | 假时钟实测 1000 ms；`geometry_latency_ms` 可观测 | 假时钟；真机 `WM_DPICHANGED` 待 T04 |
| **A-03** 600 s 成功 ≥590 次 | ✅ 逻辑通过 | `engine_600s_closed_loop_meets_a03` + `full_closed_loop_600s_with_fake_clock`：假时钟跑满 600 s，`sampled=600` | 假时钟 / 假像素，**非真机 10 分钟** |
| **A-13 / A-14** 纯黑 `Y≤0.05` 亮端色、纯白 `Y≥0.95` 暗端色、60 s 翻转 0 次 | ✅ 逻辑通过 | `judge_pure_black_and_pure_white_endpoints_do_not_flip`：600 拍 `flip_count=0`；`wcag_contrast_black_white_is_21` 校验对比度公式 | 假像素 |
| **A-28** 采样区缩到只覆盖时钟文字仍不翻转 | ✅ 逻辑通过（**双防线**） | `histogram_p50_is_robust_to_20_percent_white_noise`（20% 白污染下 P50 仍落黑桶、代表色偏移 ≤2/255）、`a28_character_pollution_does_not_flip_judgement`、`fully_masked_region_is_detected` | 假像素 + 假遮罩；按 §13-1 用 **P50 中位数**，非 trimmed mean |
| **A-10** 配置 3 s 内生效 | ✅ 逻辑通过 | `engine_config_change_applies_within_three_seconds`：版本变更**同拍**生效（`config_latency_ms=0`） | 假配置源 |
| **A-17** 暂停时间戳停止、恢复 ≤2 s | ✅ 逻辑通过 | `engine_pause_freezes_sample_timestamp_and_resumes_in_time` | 假时钟 |
| **EX-15** 电池自动 ≥2000 ms | ✅ 逻辑通过 | `battery_mode_enforces_2000ms_floor`、`sampler_loop_interval_and_battery_floor`、`engine_battery_mode_slows_sampling_but_never_stops`（600 s 成功 300 次） | 假电源 |
| **EX-24** 遮挡 / `BitBlt` 失败不崩、连续 5 帧异常跳过 | ✅ 逻辑通过 | `bitblt_failure_propagates_and_burst_is_detected`（返回 `BitBltFailed(87)`，第 5 帧起 `BadFrameBurst`，零发布）、`bitblt_failure_returns_error_not_panic`、`topmost_window_occlusion_is_excluded` | 假抓屏；真 GDI 失败路径待 T04 |
| **T02.2 ①** 600 次零堆分配 | ✅ **实测通过** | `#[global_allocator]` 线程局部计数器：`six_hundred_grabs_are_allocation_free` + `sample_once_is_allocation_free_for_600_frames` 断言 `alloc_count()==0`、`ensure_count==1`；另有 `hits_and_union_are_allocation_free` / `locate_and_compute_are_allocation_free` | GlobalAlloc 计数器（架构认可的证明方式） |
| **T02.2 ③** 缩略图 / 单次 ≤5 ms | ⚠️ **结构性** | `thumbnail_into` O(tw·th) 最近邻、无分配；`DiagSnapshot.cost_us` / `over_budget` 记录实测耗时（基于 `Instant`），`LoopReport` 提供 `cost_avg_us` / `cost_p95_us` 汇总行 | **未做真机耗时实测**；本轮不声称达标，T04 接线后按 §11.6 Debug 日志实测 |
| **T02.3 ①** 时钟 + 日期矩形、膨胀 2 px | ✅ 逻辑通过 | `working_uia_yields_clock_and_date_rects`（2 个独立矩形）、`union_inflates_by_two_pixels`、`fallback_inflates_by_two_pixels_after_union` | 假 UIA 矩形；**真实 `IUIAutomation` 未接线** |
| **T02.3 ②** UIA 不可用退回固定几何、功能不降级 | ✅ 逻辑通过 | `uia_unavailable_falls_back_without_degradation`、`empty_uia_result_also_falls_back`、`engine_uia_failure_falls_back_without_functional_degradation`（fallback 下判定仍发布） | 假 UIA 失败；130×44 **dip** 经 `dip_to_px` 换算，96/120/144/192 DPI 四组用例 |
| **T02.3 ③** UIA 不阻塞 T0/T1 | ✅ 逻辑通过 | `UiaConnectionConfig::connection_timeout_ms=300` 下发常量；`uia_timeout_over_300ms_trips_circuit_breaker`（350 ms 立即熔断，之后 10 次刷新**零** UIA 调用）、`three_consecutive_failures_trip_circuit_and_reset_retries` | 假时钟；`IUIAutomation2::put_ConnectionTimeout` 真实调用属 T04 |
| **T02.3 ④** 遮罩合并单测 | ✅ 逻辑通过 | `mask.rs` 10 个用例：连通合并、去重、膨胀、命中、全遮罩、置顶窗口遮挡、容量溢出不丢遮罩；`engine_colored_rects_survive_uia_snapshot_refresh` 验证 UIA 快照刷新后 DLL 增量矩形不丢 | 假矩形 |
| **A-20 / NFR-03** 10 分钟 CPU <0.1% | ⚠️ **结构性** | 1 Hz 事件驱动（注入 `Sleeper`，无 `Sleep(1)` 忙等）、零堆分配、单遍直方图 O(像素)、电池降频；`LoopReport` 汇总字段齐备 | **未做真机 CPU 实测**，需 T04 接线后按 A-20 用资源监视器验收 |

### 9.4 实测数据（本轮可复现）

```
cargo test --workspace           -> 96 passed; 0 failed  (shadebar-core lib)
cargo test --workspace --doc     ->  2 passed; 1 ignored (既有 ignore 文档示例)
cargo clippy -p shadebar-core --all-targets -- -D warnings -> Finished, 0 warning
cargo build --release -p shadebar-app --target x86_64-pc-windows-msvc -> 0 warning
build.ps1                        -> BUILD OK（6 道门禁全绿）

Gate 1/5 体积       : 236.5 KB (242176 bytes)   [T03.1 基线 236.5 KB，Δ = 0]
Gate 2/5 运行时依赖 : no VCRUNTIME140 / MSVCP140
Gate 3/5 FileVersion: 1.0.0.0 / ProductVersion 1.0.0.0
Gate 4/5 manifest   : PerMonitorV2 + requestedExecutionLevel
Gate 5/5 图标       : extractable 32x32
Gate 0   编码护栏   : 12 个 .ps1 全部合规
```

**体积 Δ = 0 的原因（有对照）**：`shadebar-app` 尚未 `use shadebar_core::engine`（接线属 T04），
`opt-level="z" + lto="fat" + panic="abort"` 把未引用代码整体消除。对照基线取自
§4.4 / 附录 B 的 T03.1 实测 236.5 KB。**这不代表 T04 接线后仍为零增量**，届时必须
重新跑体积门禁并回归。

单测分布：`engine::tests` 19 / `engine::taskbar` 13 / `engine::sampler` 14 /
`engine::screen` 9 / `engine::mask` 10 / `engine::uia` 9 = **74 个新增**；
既有 22 个（color 7 / error 3 / win 9 / lib 3）全部保持通过。

### 9.5 修复的既有构建脚本缺陷（非 T02 功能，但阻断门禁）

`build.ps1` 第 186 行原为
`Get-ChildItem $distDir -Filter '*_ShadeBar_v*.exe' ... | Remove-Item -Force`。
在 **Windows PowerShell 5.1.26100** 下，当 `dist\` 已存在同名 exe（本环境残留 T03.1
产物 242176 B）、且路径含中文时，`FileInfo` 甚至字符串管道都**无法绑定** `Remove-Item`
的 `-Path` 参数（`ParameterBindingException: InputObjectNotBound`），脚本在复制前中断，
六道门禁根本没机会执行。对照实验：

| 写法 | 结果 |
|---|---|
| `Get-ChildItem ... \| Remove-Item -Force` | ❌ `InputObjectNotBound` |
| `Get-ChildItem ... \| Select -Expand FullName \| Remove-Item -Force` | ❌ 同样失败（连字符串都绑不上） |
| `Get-ChildItem ... \| ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }` | ✅ 通过 |

已按第三种改法落地（保持脚本纯 ASCII，Gate 0 复核通过）。这是**构建门禁可用性修复**，
不涉及 T03/T04 功能代码；T01.1 阶段 dist 为空故未触发，属首次「二次构建」才暴露的路径。

### 9.6 明确未做（属后续任务，非占位、非 TODO）

1. **真实 Win32 / COM 适配**：`FindWindowW(Shell_TrayWnd)` / `FindWindowExW(TrayNotifyWnd)` /
   `GetWindowRect` / `SHAppBarMessage(ABM_GETSTATE)` / `MonitorFromWindow` / `GetDpiForWindow` /
   `GetDC` + `CreateDIBSection` + `BitBlt` / `IUIAutomation2::put_ConnectionTimeout` 全部
   由 T04 在 `shadebar-app` 实现 `TaskbarSource` / `ScreenSource` / `ClockRectSource` 并注入。
   `shadebar-core` 的 `Cargo.toml` **未新增任何依赖 / feature**，`windows` 仍锁 `=0.58.0`
   且只用 `Win32_Foundation`（`RECT`），**无 `UI_Xaml*` feature**。
2. **线程与定时器**：`Engine::run` 是同步循环；T1 线程、`CreateWaitableTimerW`、
   `WaitForMultipleObjects`、`SetWinEventHook`、T2 管道服务器属 T04。core 保持「无线程」。
3. **真机效果验收**：A-02 的诊断面板采样框可视化 + 截图比对、A-20 的 10 分钟 CPU 实测、
   T02.2 的单次 ≤5 ms Debug 日志实测、A-29 的端到端像素证据，均待接线后补；本报告
   **没有**把这些标成通过。
4. **T01.2–T01.8 依赖模块**：`config.rs` / `stats.rs` / `judge.rs` / `ipc.rs` / `log.rs` /
   `i18n.rs` 尚未落地。本轮在 `engine/mod.rs` 定义了引擎侧最小配置快照 `EngineConfig`
   （含 `RegionCfg` / `JudgeCfg`），在 `sampler.rs` 自带 `LumaHistogram` / `Judge`；
   T01.x 落地后应改为从对应模块 re-export / `From` 转换，避免长期双份定义。
5. **未触碰**：SP-2 的任何实验 / 注入 / explorer / TranslucentTB / 注册表 / 用户窗口；
   本轮零注入、零进程空间写入。

### 9.7 全局一致性审查

* 跨文件导入：全部 `pub use` / `use` 由 `cargo build --release` 与 `clippy -D warnings` 验证；
  无循环依赖（依赖方向 `mod → taskbar/screen/mask/uia → sampler`，`testing` 只被
  `#[cfg(test)]` 引用）。
* 接口契约：`EngineDeps` 8 个 trait object、`SampleCtx` 12 个字段、`TaskbarProbe`
  固定容量 8 屏、`MaskSet` 固定容量 32 矩形，调用点签名一致。
* 数据流：`TaskbarProbe → TaskbarGeom → compute_sample_rect → ScreenBuffer(BGRA)
  → LumaHistogram(P50) → SampleStats → Judge(EWMA+迟滞) → Judgement → DecisionSink`，
  一条链路贯通，`engine_600s_closed_loop_meets_a03` 端到端驱动验证。
* 单位铁律：采样链路只出现 px（`i32` / `RECT`）；dip 仅存在于
  `FALLBACK_CLOCK_W/H_DIP` 两个设计常量，且必经 `dip_to_px` 换算。
* 重复实现：几何工具集中在 `mod.rs`，颜色数学集中在 `sampler.rs`，无跨文件重复。
* 禁用项扫描：`unimplemented!` / `todo!` / `unreachable!` / `TODO` / `FIXME` /
  `UI_Xaml` / `unwrap()` / `panic!` / `expect()`（运行时）= **全部 0 命中**；
  测试内断言也改为 `matches!` 形式，`testing.rs` 里连"不可能到达"的占位函数都
  已用 `if let` 结构消除（§11.2 允许测试代码例外，本轮仍主动清零）。
* `SbError` 复用既有变体（`TaskbarNotFound` / `BitBltFailed` /
  `SampleRegionTooSmall` / `UiaUnavailable`），未改 `error.rs`。
* clippy `--workspace` 尚存 2 条 `assertions_on_constants` 位于**既有**
  `shadebar-app/src/render.rs:513`（T03.1 范围），本轮按「只做 T02」未越界修改，
  已如实记录；`build.ps1` 不跑 clippy，六道门禁不受影响。

**IS_PASS: YES**

---

## 10. T04 真实 Win32/COM 接线 + 真机效果证据（风险优先）

> 架构 §8.0 原顺序 T03 → T04，team-lead 本批**调整为 T04 先行**，理由：T02 的
> 96 个单测全是假几何/假像素/假时钟的逻辑证据，真实 `FindWindowW` / `BitBlt` /
> `IUIAutomation` 一次没跑过，是 L1 最大的剩余风险，必须先闭合。
> 本批**只做 T04 + 四项真机证据 + 两处收尾**，不做 T03.2~T03.6。

### 10.1 交付清单

| 文件 | 行数 | 职责 |
|---|---:|---|
| `crates/shadebar-app/src/engine/mod.rs`（新增） | 238 | 模块根、`EngineSession` 组装 10 类真实依赖、驱动 600 s 闭环 |
| `crates/shadebar-app/src/engine/taskbar.rs`（新增） | 433 | `FindWindowW` / `FindWindowExW` / `GetWindowRect` / `SHAppBarMessage(ABM_GETSTATE)` / `EnumDisplayMonitors` / `GetDpiForWindow` |
| `crates/shadebar-app/src/engine/screen.rs`（新增） | 359 | 复用 `HDC` + `CreateDIBSection` + `BitBlt`（top-down BGRA）；`grab_window` 抓自建窗口 DC；EX-24 真实失败注入 |
| `crates/shadebar-app/src/engine/uia.rs`（新增） | 424 | `IUIAutomation2` 独立 **MTA** 线程 + 350 ms 看门狗；4 组 `(CLSID, CLSCTX)` 顺序回退；元素引用只在查询帧内存活（§11.10） |
| `crates/shadebar-app/src/engine/platform.rs`（新增） | 494 | `SystemClock` (`Instant`) / `WaitableTimerSleeper` (`CreateWaitableTimerW`) / `ClockBackedStop` / `GetSystemPowerStatus` / `FixedConfigSource` / 两个 `Recording*Sink` / `GetProcessTimes` |
| `crates/shadebar-app/src/engine/evidence.rs`（新增） | 1152 | 四项真机证据编排、日志/截图写出、汇总 md、`--verify fast\|full` |
| `crates/shadebar-app/src/engine/png.rs`（新增） | 337 | **零依赖** PNG 编码器（stored DEFLATE + 自实现 CRC32/Adler32），不引入 `image`/`png` crate 以守住 §9.1 体积门禁 |
| `crates/shadebar-app/src/engine/visual.rs`（新增） | 366 | 自建 `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW` 诊断/纯色窗口（GDI 自绘），RAII 销毁 |
| `crates/shadebar-app/src/main.rs`（改） | 141 | `mod engine` + `--verify fast\|full [dir]` 取证入口 |
| `crates/shadebar-app/src/render.rs`（改，1 处） | — | 修 T03.1 遗留 `assertions_on_constants` |
| `crates/shadebar-app/Cargo.toml`（改） | — | 追加 6 个 `windows` feature；版本仍锁 `=0.58.0`，**无** `UI_Xaml*` |
| `crates/shadebar-core/src/engine/screen.rs`（改，1 处） | +11 | 补 `ScreenBuffer::as_bytes_mut()` 供真实 GDI 直接写入复用缓冲 |
| `docs/spike-report.md`（改） | 本节 | §10 报告 |
| `evidence/t04/**`（新增，13 文件） | — | full 档权威证据（截图 + 数值日志 + 汇总） |
| `evidence/t04-smoke/**`（新增） | — | fast 冒烟档对照，**明确不是 10 分钟实测** |

**未新增任何 crate 依赖**；`shadebar-core/Cargo.toml` 零改动，core 保持无窗口/无线程/可单测。

### 10.2 接线与 8 类 trait 对照

`EngineSession` 用字段解构把 `&mut` 分别借给 `EngineDeps`，避免交叉借用；
`Engine::run()` 由真实 `CreateWaitableTimerW` 驱动，core 不 spawn 线程（§6.2 / §3.3）。

| trait | 真实实现 | 校验 |
|---|---|---|
| `TaskbarSource` | `Win32TaskbarSource` | 真机 `Shell_TrayWnd=0x50223C` / `TrayNotifyWnd=0x46052A` |
| `ScreenSource` | `GdiScreenSource` | 10 分钟 `dib_rebuilds=1`（只建一次 DIB） |
| `ClockRectSource` | `UiaClockRectSource` | `IUIAutomation2` + `SetConnectionTimeout(350)` |
| `Clock` | `SystemClock` | `Instant` 单调，ms 口径与 core 一致 |
| `Sleeper` | `WaitableTimerSleeper` | `sleeper_waits=727`（10 分钟，非忙等） |
| `StopToken` | `ClockBackedStop` | 600 s 准时退出，`wall_ms=600184` |
| `PowerSource` | `SystemPowerSource` | `GetSystemPowerStatus().ACLineStatus` |
| `ConfigSource` | `FixedConfigSource` | T01.2 `ConfigStore` 落地前的接线快照 |
| `DecisionSink` / `DiagSink` | `RecordingDecisionSink` / `RecordingDiagSink` | 每拍数值进日志，`cost_us` 来自 core 的 `Instant` |

### 10.3 完成判据（构建/门禁）

```
cargo test --workspace                      -> 40 (app) + 96 (core) + 2 doc ; 0 failed
cargo clippy --workspace --all-targets -D warnings -> 0 warning（含 render.rs 两处已修）
cargo build --release (MSVC x64)            -> 0 warning
build.ps1 六道门禁                           -> BUILD OK
  Gate 0 编码护栏 / Gate 1 体积 / Gate 2 无 VCRUNTIME140
  Gate 3 FileVersion=1.0.0.0 / Gate 4 PerMonitorV2 manifest / Gate 5 图标
禁用项扫描（app/engine + main.rs）           -> TODO/FIXME/panic!/unwrap()/expect(
                                               /unimplemented!/todo!/unreachable!/UI_Xaml = 全 0
```

**体积（team-lead 要求接线后重跑并对照 400 KB 软目标）**：

| 阶段 | 字节 | KB |
|---|---:|---:|
| T03.1 / T02 基线（未 `use engine`，LTO 消除） | 242176 | 236.5 |
| T04 接线后（本批终值） | **418816** | **409.0** |
| 增量 | **+176640** | **+172.5** |

上轮「Δ=0」确系 `opt-level=z + lto=fat` 消除未引用代码造成的假象，本轮 `use engine`
后真实增量 **+172.5 KB**。**超 400 KB 软目标 9 KB（+2.2%）**，远低于 NFR-01 硬上限
4096 KB（占 10.0%）。`build.ps1` Gate 1 明确打印 "above the 400 KB soft goal
(not blocking, explain in the PR)"。增量来源：6 个 `windows` feature 的符号、
UIA 线程、PNG 编码器、**整套取证编排**（`evidence.rs` 1152 行）。
> 说明：`--verify` 取证链路编进了正式 exe。若后续要压回 400 KB，可用
> `#[cfg(feature="verify")]` 把取证模块隔离到单独构建；本批按「如实记录」要求
> 未做此优化，避免为了凑数字改动交付形态。

### 10.4 真机效果证据：四项逐条对照

证据目录 `evidence/t04/`（full 档，60 s + 60 s + 600 s，墙钟 12 m 19 s）。
每项均给出**截图 + 采样数值 + 日志路径**；截图经独立 Python PNG 解码器核验内容。

| 判据 | 需求字面要求 | 真机实测 | 结论 | 截图 / 日志 |
|---|---|---|---|---|
| **A-02** | 采样框完全落在托盘区，误差 ≤ 4 px | `TrayNotifyWnd=(2155,1392)-(2560,1440)`；`sample_rect=(2155,1396)-(2560,1436)`；`contained=true`，**error=0 px**；`found_real_tray=true`（类名精确匹配）；dpi=96，1 屏 | ✅ **通过** | `A-02-tray-annotated.png`（405×48，真实托盘图标/时钟/日期可见 + 三色标注框）/ `A-02-geometry.log` |
| **A-13** | 纯黑 60 s：亮端色、`Y ≤ 0.05`、翻转 0 次 | 自建纯色窗口覆盖采样区（`covered_rgb=(0,0,0)` vs 基线 `(221,223,229)`，δ=0）；60 s / 60 samples；`side=Light`，`color=#FFFFFF`；末拍 `Y=0.0259 ≤ 0.05`；稳态 59 拍全 `flipped=false` | ⚠️ **有条件通过**（见 10.5） | `A-13-black-screen.png`（纯黑 + 标注框）/ `A-13-black-endpoints.log` |
| **A-14** | 纯白 60 s：暗端色、`Y ≥ 0.95`、翻转 0 次 | `covered_rgb=(255,255,255)` δ=0；60 s / 60 samples；`side=Dark`，`color=#000000`；末拍 `Y=0.9885 ≥ 0.95`；稳态 59 拍全 `flipped=false` | ⚠️ **有条件通过**（见 10.5） | `A-14-white-screen.png` / `A-14-white-endpoints.log` |
| **A-20** | 资源监视器 10 分钟平均 CPU < 0.1% | 600184 ms、592 samples、592 ticks、0 bad、0 flips；ΔCPU=1875000×100ns → **单核 0.0312%** / 12 核全机 **0.0026%** | ✅ **CPU 通过** | `A-20-real-screen.png` / `A-20-NFR-03-cpu.log` |
| **NFR-03** | 单次采样耗时 ≤ 5 ms（日志记录） | avg **4546 µs** / p95 **6725 µs** / max **11721 µs**；**241/592 超 5000 µs** | ❌ **未达标**（见 10.5） | 同上 |
| **EX-24** | 连续 5 帧异常跳过并保持上次判定 | 真实 `BitBlt`（源 DC 置 NULL）连续失败；`outcomes` = 4×`Skipped(BitBltFailed{code:0})` + 2×`Skipped(BadFrameBurst)`；`last_decision_kept=true`、`published_delta=0`、`bad_frames_delta=6`；关闭注入后 `recovered_after_fault=true`；`dpi_recompute_after_fault=true`；**进程未崩** | ✅ **通过** | `EX-24-real-screen.png` / `EX-24-bitblt.log` |

**A-02 可视化截图**是真实 `BitBlt(GetDC(NULL))` 抓到的托盘区像素（微信/QQ/时间
`19:14`/日期等可辨），叠加黄=tray、红=sample、品红=遮罩三种标注框；不是示意图。

### 10.5 如实记录的四处偏差（不掩盖、不包装）

1. **NFR-03 单次采样 ≤ 5 ms 未达标（真实失败）**。`cost_us` 只包 `sample_once`
   （`sampler.rs:835` 的 `Instant::elapsed()`，含真实 `BitBlt` + 直方图 + 判定，
   不含 UIA/几何），不是假时钟。10 分钟段 p95=6725 µs、max=11721 µs、
   241/592 帧超预算。**口径交叉核算**：`cost_us` 累计 2.6912 s，而
   `GetProcessTimes` 报进程 CPU 仅 0.1875 s，差值 2.5037 s 是 DWM 合成/抓屏
   **阻塞等待而非 CPU 消耗** —— 这解释了「单次墙钟超 5 ms」与「10 分钟 CPU
   0.0312% 远优于 0.1%」为何同时成立。首轮 fast 档曾误把前面故障注入/黑白段的
   样本混进 p95，本批已修为**只统计本段**（`cost_sample_scope=this segment only`），
   并另起一行打印全量累积值且标注「不参与判定」。
   **建议**：`BitBlt` 从桌面 DC 抓屏在 DWM 下有同步等待；若要满足 5 ms，
   需在 T04 后续改用 `PrintWindow`/DXGI 桌面复制或缩小采样区，本批不改判据、
   不改测量口径，如实上报交 team-lead 决策。

2. **A-13 / A-14 的 `flips=1`，字面「60 s 翻转 0 次」严格不成立**。
   该 1 次发生在**段首**：取证编排按 A-02 → EX-24 → 黑 → 白 → CPU 连续跑在
   **同一个 `EngineSession`** 上，切换背景时判定器必然从上一段的端翻到新端
   （黑段从上一段 `Dark`→`Light`；白段从 `Light`→`Dark`）。证据：
   `judge_tail` 末 5 拍全 `flipped=false`，即**稳态 59/60 拍零抖动**；
   10 分钟 CPU 段 `flips=0`。这是**取证编排伪影，不是引擎缺陷**，也没有被
   包装成「0 次」。若要严格贴合字面，下批可让每段用独立 `EngineSession`
   （或先稳定再开始计时）重跑；本批按「有对照证据」要求保留原始数据。

3. **UIA 真机遮罩未取得，走 fallback**。`UIA-real-probe.log` 实测：
   `CUIAutomation/CLSCTX_ALL` → `0x80040154 (REGDB_E_CLASSNOTREG)`；
   `CUIAutomation/CLSCTX_INPROC_SERVER` → `0x80004002 (E_NOINTERFACE)`；
   **`CUIAutomation8/CLSCTX_INPROC_SERVER` 成功**，`IUIAutomation2 ready,
   ConnectionTimeout=350ms`，枚举 30 个后代；但 Win11 26H2 任务栏 XAML 树的
   时钟/日期元素**命中 0 个**（`last_hit_count=0`），core 自动退回固定几何
   `fallback_clock_rects`，采样判定全程未中断（`samples=592`）。
   这同时是 **T02.3 判据②「UIA 不可用不降级」的真机证据**，但
   「UIA 矩形发现」本身**明确记为未取得**，不拿 fallback 冒充 UIA 成功。
   只读核查过注册表：两个 CLSID 均已注册且 `ThreadingModel=Both`，exe 为
   x64 —— 故不是位数/线程模型问题，属于该环境 UIA 可视树结构差异。

4. **诊断面板窗口截图未取得**。`WS_EX_NOACTIVATE` 窗口在 DWM 重定向表面下，
   `BitBlt(GetDC(NULL))` 抓到纯白、`GetDC(hwnd)` 抓到近纯黑，两次尝试均无法
   取得 `WM_PAINT` 内容。按「卡住就报、不无限尝试」停止，改为
   `distinct_rgb_count` 严格校验（<3 色即判无效、不写文件、日志标注 NOT obtained），
   并给 EX-24 / A-20 补上**真实屏幕采样区截图**作为有效替代证据。
   A-02 主截图自始有效。

### 10.6 实测数据（可复现）

```
# 正式取证（10 分钟）
dist/随影_ShadeBar_v1.0.0.exe --verify full evidence/t04
  -> T04 verify done: 5/5 obtained; wall 12m19s
# 快速冒烟（明确不是 10 分钟实测）
dist/随影_ShadeBar_v1.0.0.exe --verify fast evidence/t04-smoke
  -> 4/5 obtained; A-20 明确标注「10 分钟 CPU 实测未取得」

真机环境（取证期间只读，未改动）：
  explorer pid 31792、TranslucentTB pid 32532 —— 全程未重启/未注入/未停止
  注册表未写、壁纸未改、用户已打开窗口未动
  主屏 2560x1440 @96dpi、12 逻辑核、任务栏底部 48px、非自动隐藏
```

`5/5 obtained` 指 5 个证据条目（A-02 / EX-24 / A-13 / A-14 /
A-20+NFR-03）的**截图 + 数值 + 日志齐备**，**不等于判据全通过** ——
NFR-03 单次预算失败、A-13/A-14 字面零翻转偏差、UIA 矩形未取得、
诊断面板截图未取得，均在 10.5 逐条列明。

### 10.7 额外真机发现（T02.1 硬判据的补充证据）

| 项 | 真机结果 |
|---|---|
| 真实 `TrayNotifyWnd` 定位 | `FindWindowExW` 直接命中，类名精确，非几何近似 |
| 自动隐藏双保险 | `SHAppBarMessage(ABM_GETSTATE)` + 几何越界，`auto_hide=false`，两判据均为负 |
| 多显示器选屏 | 本机 1 屏 `bounds=(0,0)-(2560,1440)`；`sample_rect` 全在屏内 |
| DPI / 拓扑 | dpi=96；`on_dpi_changed()` 在 EX-24 故障恢复后触发重算成功（`dpi_recompute_after_fault=true`） |
| 600 s 闭环 | 592 samples（1 Hz，含段首结算），0 bad、0 flips；`A-03 ≥590` 口径下达标 |
| DIB 复用 | 10 分钟 `dib_rebuilds=1`，证实「只在尺寸变化时重建」 |
| 无忙等 | `sleeper_waits=727`，定时器驱动 |

### 10.8 明确未做（本批范围外）

* SP-2 任何实验、explorer 注入/重启、TranslucentTB 任何操作 —— **一律未做**。
* T03.2~T03.6 设置页（35 项）、T04.1 单实例/托盘/正式启动流程 —— 未做；
  `--verify` 是独立命令行入口，无参数启动仍按现状正常退出，不制造「已常驻」假象。
* `ConfigStore` / `RegisterApplicationRestart` / IPC / 注入 DLL —— 未做（后续批次）。
* 端到端改色效果（A-29）仍为人工验收项；L2/L3 注入着色不在 v1.0.0 L1。
* NFR-03 ≤5 ms 的优化未做（10.5①给出根因与建议，交 team-lead 决策）。

### 10.9 全局一致性审查

* 依赖方向单向：`shadebar-app::engine → shadebar-core::engine`，无循环依赖；
  core 的 `engine/` 本批只加 1 个 `as_bytes_mut()`，96 个单测全部保持通过。
* 10 类 trait 全部有真实实现，`EngineDeps` 12 字段签名与调用点逐一核对一致；
  `EngineSession` 字段解构借用使 `&mut deps` 与 `&dyn Clock` 不冲突。
* 分位数统计只保留一份实现（`RecordingDiagSink::cost_stats_of`），
  `cost_stats()` 与取证本段切片共用口径，无重复实现。
* 错误统一走 `SbError` 既有变体（`BitBltFailed` / `TaskbarNotFound` /
  `UiaUnavailable` / `PathNotWritable` / `Unsupported`），未改 `error.rs`。
* 单位铁律：采样链路全 px（`RECT`/`i32`）；`windows` 0.58 的 `Result<()>` /
  裸值 / `BOOL` 三类返回值逐一按 §11.9 处理（本批踩到的坑见 10.10）。
* UIA 元素引用严格限制在 `query_rects` 单次调用帧内，主线程只拿 `Vec<MaskRect>`
  值，不缓存 COM 指针（§11.10）。
* `windows` 版本锁 `=0.58.0`，feature 清单内**无任何 `UI_Xaml*`**（§9.1）。
* 禁用项扫描全 0；`clippy --workspace --all-targets -D warnings` 零告警
  （含本批修掉的 `render.rs:513` 两条）；release 零 warning。

**IS_PASS: YES**（接线与证据交付一致、可编译可复现；
判据层面 NFR-03 单次 ≤5 ms **未达标**、A-13/A-14 字面零翻转有编排伪影、
UIA 矩形未取得 —— 三项已如实上报，不参与「代码一致性」结论）

### 10.10 本批最大的坑

1. **`windows` 0.58 返回值三态**（§11.9 的真实代价）：`GetWindowRect` /
   `GetClientRect` 返回 `Result<()>`，`GetDC`/`SelectObject`/`DeleteObject` 返回
   **裸值**，`BitBlt`/`CreateDIBSection` 返回 `Result<T>`，`SHAppBarMessage` 返回
   `usize` 位掩码，`GetDpiForWindow` 返回裸 `u32`。混用 `?` 会直接编译失败；
   另外 `PTIMERAPCROUTINE` 在 `Win32::System::Threading` 而非
   `UI::WindowsAndMessaging`，`CreateWaitableTimerW` 还需要 `Win32_Security`
   feature 前置。本批逐条 grep registry 源码核准后才写，避免反复试错。
2. **DWM 重定向表面抓不到自建窗口**：`WS_EX_NOACTIVATE` 窗口用
   `BitBlt(GetDC(NULL))` 得纯白、用 `GetDC(hwnd)` 得近纯黑，
   `InvalidateRect + UpdateWindow` 强制重绘也无解。两次尝试后按纪律停手，
   改以真实屏幕截图 + 严格有效性校验兜底，并如实标注未取得。
3. **UIA COM 实例化**：架构只写了 `CUIAutomation`，真机上该 CLSID 两种
   CLSCTX 分别返回 `REGDB_E_CLASSNOTREG` / `E_NOINTERFACE`；
   `CUIAutomation8 + CLSCTX_INPROC_SERVER` 才成功。已实现 4 组顺序回退并把
   每组 HRESULT 写进诊断日志（不静默吞错）。
4. **统计口径污染**：首轮 `cost_stats()` 用了全量累积样本，把前面故障注入和
   黑白段的耗时混进 A-20 的 p95。修为按 `cost_mark` 切片统计本段，并保留
   一行明确标注「全量累积，不参与判定」的对照值。
5. **stored DEFLATE PNG 大小与内容无关**：同尺寸截图字节数恒定，
   首轮误以为「三张 panel 大小相同 = 内容相同」。改用独立 Python 解码器核验
   像素后才定位到纯白根因，并把 CRC32 常量与 Python `zlib.crc32` 交叉核对
   （曾把 `crc32(b"IHDR")` 的期望值写错，已修正并加 chunk CRC 自洽单测）。

---

## 11. L1 主体收尾：设置页 + 托盘 + 体积收口 + 独立 session 重跑（T03.2~T03.6 / T04.1 / #9）

> 本批对应共享任务 **#9**，在需求基线 **v1.4**、架构 **v2.0** 口径下完成。
> 铁律不变：不注入 / 不重启 explorer / 不动 TranslucentTB(pid 32532) /
> 不改注册表 / 不动用户已打开的窗口；可视化一律新建本进程
> `WS_EX_NOACTIVATE` 窗口，用完销毁。取证全程只读。

### 11.1 交付清单

| 类别 | 文件 | 内容 |
|---|---|---|
| 新增 | `crates/shadebar-app/src/config.rs` | 35 项配置 `SettingKey`/`SettingMeta`/`Config`，JSON 原子落盘 + 损坏回退 `.bak`，`to_engine_config()` 是设置→引擎唯一换算点 |
| 新增 | `crates/shadebar-app/src/app.rs` | `Arc<App>` 共享状态、真实采样线程（每拍前 `set_config` 实现 A-10 同拍生效）、1 Hz 诊断节流、零依赖 stored ZIP 诊断包、剪贴板复制版本号 |
| 新增 | `crates/shadebar-app/src/ipc_server.rs` | 单实例互斥体 + 第二实例激活、`WM_SHADEBAR_*` 私有消息常量 |
| 新增 | `crates/shadebar-app/src/tray.rs` | 程序生成 16×16 托盘图标（判定色点 / 受限角标）、6 项菜单、左键三选一、首启气泡 |
| 新增 | `resources/lang/zh-CN.tsv`、`en-US.tsv` | 142 个 key 的双语文案表，`include_str!` 内嵌，无侧车文件 |
| 新增 | `crates/shadebar-app/src/ui/i18n.rs` | 查表 + A-26 对照用的 `no-en` 分支 |
| 新增 | `crates/shadebar-app/src/ui/layout.rs` | dip 布局度量、滚动视口、换行 / 省略号 |
| 新增 | `crates/shadebar-app/src/ui/widgets.rs` | 统一 `Widget` trait，7 类控件（含 ARGB ColorField） |
| 新增 | `crates/shadebar-app/src/ui/pages/{mod,general,color,region,elements,behavior,diagnostics}.rs` | 6 个页面 + 诊断面板 |
| 重写 | `crates/shadebar-app/src/ui/mod.rs` | `View`：控件树 / 卡片 / 滚动 / 焦点 / 悬停 / 弹层 / 语言热切换 / 二次确认 |
| 重写 | `crates/shadebar-app/src/window.rs` | 接入 `View` + 托盘 + 单实例激活；`WM_CLOSE` 按 S-41 最小化到托盘或退出 |
| 重写 | `crates/shadebar-app/src/main.rs` | 正式启动路径；`--verify` 分支 `cfg(feature="verify")` 隔离 |
| 修改 | `crates/shadebar-app/src/engine/mod.rs` | `verify` 特性门控取证模块与 UIA 适配器；新增正式版 `FixedFallbackClockRectSource` 与 `set_config`/`config_snapshot` |
| 修改 | `crates/shadebar-app/src/engine/platform.rs` | `flip_flags` 逐拍翻转标志、`FixedConfigSource::update/get` |
| 修改 | `crates/shadebar-app/src/engine/evidence.rs` | `--verify endpoints` 档、黑白各起独立 `EngineSession`、稳态段 flips、判据端点核对、NFR-03 双口径报告 |
| 修改 | `crates/shadebar-app/Cargo.toml` | `[features] verify / no-en`，`Win32_Globalization` / `Win32_System_DataExchange` / `Win32_System_Memory` |
| 修改 | `crates/shadebar-app/src/ui/theme.rs` | 控件级调色板（accent / track / disabled / warning / danger / ok），全部 α=255 |
| 修改 | `crates/shadebar-app/src/render.rs` | `paint_skeleton_frame` 保留为 T03.1 对照帧，正式重绘入口改为 `ui::View` |
| 修改 | `build.ps1` | 三产物（正式 / verify / no-en）各跑同一套五道门禁 + A-26 增量断言 |
| 新增 | `README.md`、`LICENSE`、`CHANGELOG.md` | NFR-14 / D-01~D-05；README「已知限制」按 v1.4 §7.1 的 **K-01~K-07 共 7 条** |
| 新增 | `evidence/t04-rerun/` | A-13 / A-14 独立 session 重跑证据（日志 + 截图 + 汇总） |
| 新增 | `evidence/NFR-03-dual-criteria.md` | CPU / `cost_us` 双口径核对记录（派生自 T04 full 档，放根目录以免重跑清空） |

### 11.2 完成判据逐条对照

| 判据 | 实测 | 结果 |
|---|---|---|
| 139+ 单测全过 | `cargo test --workspace`：**110（app）+ 96（core）+ 2（doc）= 208** | ✅ |
| `clippy --workspace --all-targets -D warnings` 零告警 | 4 种特性组合（默认 / `verify` / `no-en` / `--all-features`）全部 `Finished`，零 warning | ✅ |
| `build.ps1` 六门禁全绿（两个 profile 各一次） | 正式 / verify / no-en **三个产物各跑 5 道门禁**，`BUILD OK` | ✅ |
| 正式版体积 < 400 KB | **408064 B = 398.5 KB**（软目标内；硬限 4096 KB，余量 3697.5 KB） | ✅ |
| A-13 / A-14 稳态 `flips=0` | `2/2 obtained`，两者 `flips=0`、`flips_steady=0`、60 s 各 60 拍 | ✅ |
| `IS_PASS: YES` | 见 §11.10 全局一致性审查 | ✅ |
| `docs/spike-report.md` 新增 §11 | 本节 | ✅ |

### 11.3 体积收口（`cfg(feature="verify")`）

| 产物 | 大小 | 说明 |
|---|---:|---|
| T04 接线后（没有隔离） | 418816 B / 409.0 KB | 超 400 KB 软目标 9 KB，增量来自取证编排 |
| **正式发布版（本批，验收依据）** | **408064 B / 398.5 KB** | `verify` OFF：**398.5 KB < 400 KB** |
| 取证编排版（`--features verify`） | 498688 B / 487.0 KB | 不作体积验收依据（NFR-01 明确以发布版为准） |
| A-26 对照版（`--features no-en`） | 401920 B / 392.5 KB | 双语增量 **6144 B = 6 KB ≤ 40 KB** |

**关键发现（不是简单的「删掉三个文件就够了」）**：第一版只按任务书写隔离
`evidence.rs`/`visual.rs`/`png.rs` + `--verify` 分支，正式版反而涨到 **438272 B**
——新增 35 项 UI 超过了取证代码的节省。用「移除采样线程」的对照实验定位到
`std::thread` 只占约 1 KB，真正的 76 KB 大头是 **`engine/uia.rs`**：独立 MTA 线程 +
`mpsc` 看门狗 + `IUIAutomation2` COM vtable + 4 组 `CoCreateInstance` 回退。

于是按 **K-05 真机事实**（26H2 XAML 树 30 个后代、时钟/日期矩形命中 0 个，
core 始终走固定几何 fallback）把 UIA 适配器也门到 `verify` 档；正式版改用
`FixedFallbackClockRectSource`，直接返回 `SbError::UiaUnavailable`，由
`core::UiaProbe::refresh_into` 走**同一条**固定几何 fallback（T02.3 判据②已验证）。
这不是「假装 UIA 可用」，而是把「UIA 发现能力未取得」如实表达成显式错误；
采样与判定不中断，真机行为不变。完整 UIA 适配器保留在取证档，A-02/A-13/A-14
的 UIA 诊断日志不受影响。改后正式版 410112 B，再去掉运行期 142 项 key 自检
（其覆盖由更强的单测保证）后降到 **408064 B**。

### 11.4 A-13 / A-14 独立 `EngineSession` 重跑

按需求基线 v1.4 §9.1 表后注：**纯黑与纯白各起一个独立 `EngineSession`**，
只跑两项，判据数值一字未改（A-13 `Y ≤ 0.05`、A-14 `Y ≥ 0.95`、60 s 稳态翻转 0 次，
**没有放宽为「允许 1 次翻转」**）。新增 `--verify endpoints` 档：端点各 60 s
（不放宽为 fast 的 5 s），`cpu_ms()` 返回 0，刻意不跑 10 分钟 CPU 段。

`run_endpoint` 同时补齐了 T04 首轮缺的端点核对：覆盖生效（`delta ≤ 24`）、
有采样、稳态翻转 0、判定端与预期一致、末拍 Y 落在端点区间，任一不满足即
`obtained=false` 并写明原因。日志同时如实保留全段原始 `flips`、段首
`flips_head_excluded`、`flips_steady` 与 `steady_sample_ticks`。

#### 第一次重跑：**1/2 obtained（如实上报，没有硬凑）**

| 项 | 结果 |
|---|---|
| A-13 | ✅ `flips=0` / `flips_steady=0` / `side=Light` / `Y=0.011` |
| A-14 | ❌ `flips=1` / `flips_steady=1` / `side=Dark` / `Y=0.974` |

`flips_head_excluded=false` —— 这次翻转**不在段首第一拍**，所以「排除段首一拍」
的口径救不了它。根因追到 `core::Judge::new()` 的冷启动状态（`ewma_y=0`、
`current=None`）：旧编排为了拿到几何，先 `tick()` 一拍建立 `geom`，那一拍采到的是
**真实桌面（暗色 #01141B）**，把 EWMA 种子设到暗端；随后覆盖纯白，EWMA 需要
若干拍从 ~0.01 爬到 ~0.98，在段内第 4~5 拍穿过迟滞带，产生一次**EWMA 收敛翻转**。
它既不是产品抖动，也不是「共用 session 切背景」的编排伪影，而是**独立 session
冷启动 + 先采错背景**造成的新编排伪影。

#### 修复：让第一拍就采到目标背景

不再用 `tick()` 预热几何；改为 `session.taskbar_mut().read_probe()` +
`TaskbarGeom::from_probe()` **只读探测**（不产生任何判定），先建纯色覆盖窗口，
再让本会话的**第一拍**就采到纯黑/纯白。冷启动 `Judge::update` 首次直接以
`stats.y` 作为 EWMA 初值（`sampler.rs:465`），因此天然没有收敛瞬态。
日志里显式记录 `geometry: direct read_probe (no pre-loop tick, first sample sees overlay)`
与 `pre_loop_ticks=0`。

#### 最终重跑：`evidence/t04-rerun/`，**2/2 obtained**

| 判据 | 时长 | 采样 | 原始 `flips` | 稳态 `flips_steady` | `steady_sample_ticks` | 判定端 | 末拍 Y | 结果 |
|---|---:|---:|---:|---:|---:|---|---:|---|
| A-13 纯黑 | 60900 ms | 60 | 0 | 0 | 59 | `Light` / `#FFFFFF` | 0.000 ≤ 0.05 | ✅ |
| A-14 纯白 | 60859 ms | 60 | 0 | 0 | 59 | `Dark` / `#000000` | 1.000 ≥ 0.95 | ✅ |

> 静态纯色背景下 `change_threshold=8` 使 60 s 内只发布 1 次判定，故
> `steady_publishes=0`；判据关心的是采样拍数，`steady_sample_ticks=59` 才是
> 真正的稳态拍数。段首那拍 `flips_head_excluded=false`，排除与否都不改变
> `flips=0` 的结论 —— 也就是说**这次是按更严格的全段口径通过的**。

两项 `coverage_delta=0`（自建纯色窗口确实覆盖采样区），截图 `A-13-black-screen.png` /
`A-14-white-screen.png` 带采样框标注，基线像素 `#01141B` 与覆盖后 `(0,0,0)` /
`(255,255,255)` 构成区域对照。10 分钟 CPU 段的 `flips=0` 仍以 T04 full 档
`evidence/t04/A-20-NFR-03-cpu.log` 为准（本批按约定未重跑 10 分钟）。

### 11.5 NFR-03 双口径（`evidence/NFR-03-dual-criteria.md`）

**主判据**（`GetProcessTimes`，10 分钟）：单核 CPU **0.0312% < 0.1%** ✅；
**辅助判据**（`cost_us`，样本 592 ≥ 500）：p95 **6.725 ms ≤ 10 ms** ✅、
max **11.721 ms ≤ 15 ms** ✅。三项同时满足 → v1.4 口径通过。
交叉核算保留：`cost_us` 累计 **2.6912 s**（墙钟，含 DWM 抓屏阻塞等待）
vs 进程 CPU **0.1875 s**，差值 **2.5037 s** 属阻塞等待而非代码耗时。
原「单次 ≤ 5 ms」（241/592 超线）已废止，代码中记为
`budget_us_deprecated` / `nfr03_legacy_5ms_met`，**不改采样实现、不改口径去凑 5 ms**。
`evidence.rs::verify_cpu` 现同时输出 `nfr03_cpu_met` / `nfr03_wall_met` /
`nfr03_dual_met` 与 `cross_check` 行，QA 打勾 A-20 须核对双口径记录。

### 11.6 35 项设置与双语（A-25 / A-26）

* `SettingKey::ALL` 共 35 项，单测断言下标唯一、默认值与 §6 表格一致；
  6 个设置分组 + 诊断页共 7 个卡片，`View` 绑定 35 行 + 7 标题。
* 文案全部在 `resources/lang/*.tsv`（142 key），UI 逻辑零硬编码；
  `i18n::tests::both_tables_cover_every_required_key` 逐项核对两种语言下
  35 项标签/说明、7 个分组标题、全部下拉选项、诊断面板、托盘菜单 6 项、
  气泡、降级横幅、注入失败提示、恢复默认确认框、关于页、日志级别名称，
  且 `both_tables_have_identical_key_sets` 保证两表 key 集合一致。
* A-26：`--features no-en` 真正**不编译**英文表，实测增量 **6 KB ≤ 40 KB**；
  `dist\` 根目录仅 1 个 exe，无 `lang\` 侧车文件。
* S-04「跟随系统」按 `GetUserDefaultUILanguage` 主语言 ID 判定（2052 → 简体中文，
  其余 English）；切换即时生效、不重启进程、采样线程不中断（A-24）。
* 降级时 S-30~S-33 / S-35 / S-37 **置灰但保留用户设定值、不清零**（NFR-05 / §13-6），
  单测 `degraded_keeps_user_values_not_zeroed` 专门覆盖。

### 11.7 单实例 / 托盘 / 权限模型

* `Local\ShadeBar.SingleInstance` 互斥体（`shadebar-core` 常量，未改），
  第二实例 `FindWindowW` + `PostMessageW(WM_SHADEBAR_ACTIVATE)` 激活既有窗口后
  ≤1 s 退出（A-01 / FR-01）。
* 托盘 6 项菜单全部接线（打开设置 / 暂停自适应 / 开机自启动 / 导出诊断包 /
  关于 / 退出）；S-40 左键三选一、S-43 判定色小圆点（1 Hz 节流）、
  S-41 关闭窗口最小化到托盘（A-09）、降级受限角标。
* 图标程序生成 16×16 32bpp DIB → `CreateIconIndirect`，深/浅任务栏轮廓清晰，
  与 TranslucentTB 明显不同（A-06）；`app.ico` 仍内嵌供资源管理器与门禁 5/5。
* **全程普通权限、UAC = 0**：manifest `asInvoker`（门禁 4/5 校验），
  无 `runas`、无提权入口、不写注册表外观项（FR-20 / NFR-11）。

### 11.8 复现命令

```powershell
powershell -ExecutionPolicy Bypass -File build.ps1          # 三产物 + 三套五门禁 + A-26
cargo test --workspace                                      # 208 项
cargo clippy --workspace --all-targets -- -D warnings       # 默认档
cargo clippy --workspace --all-targets --features verify -- -D warnings
cargo clippy --workspace --all-targets --features no-en -- -D warnings
dist\verify\随影_ShadeBar_v1.0.0_verify.exe --verify endpoints evidence\t04-rerun
```

### 11.9 明确未做 / 与上游文档的差异（逐条如实登记）

1. **T04.2 计划任务自启注册、T04.3 崩溃恢复、T04.4 注入编排框架、
   T04.5 日志轮转 / 主题联动**：本批未做。S-02「开机自启动」只持久化用户设定并
   记日志，**不注册 schtasks、不弹 UAC**（NFR-11 UAC=0 不受影响）。
2. **T05 注入 DLL / L2 / L3 着色**：挂起，待 A-29 人工验收；L1 交付态因此如实
   显示降级横幅与「受限」角标，S-30~S-37 置灰。
3. **单实例激活通道与架构 §7.1 不同**：架构用 `\\.\pipe\ShadeBar.activate` 命名管道 +
   常驻监听线程；本实现改为 `FindWindowW` + 私有 `WM_APP+1` 消息。
   行为判据 A-01 不变，且省掉一个监听线程与两组 `windows` feature 绑定。
   `ACTIVATE_PIPE_NAME` 常量保留在 `shadebar-core` 供后续版本复用。
4. **正式版不编译 UIA 适配器**：见 §11.3，依据是 K-05 真机「命中 0 个」；
   取证档保留完整适配器。core 的 fallback 路径与真机一致。
5. **S-35 / S-37 默认值**：需求基线 §6 表格写 `#FFFFFF`，架构 §13-2 决策与
   T03.4 判据写 `#38FFFFFF`（alpha 0x38 ≈ 22%）。本实现**采用架构口径
   `#38FFFFFF`**，单测固定该值；这是两份文档的既有冲突，按「优先系统设计并
   说明差异」处理，请 PM / 架构师在 v1.5 对齐表格。
6. **README「已知限制」条数**：架构 §8.6 旧文要求 8 条，需求基线 v1.4 §7.1
   / D-04 已改为 **K-01 ~ K-07 共 7 条**。README 按 v1.4 的 7 条写；
   **K-05 明确写「UIA 时钟矩形未取得，30 后代命中 0，回退固定几何」，
   没有宣称「已通过 UIA 定位时钟」**；K-07 写明体积以发布版为准、
   400 KB 是工程软目标不是门禁。
7. **诊断包**：`S-46` 生成的是零依赖 stored ZIP（配置 + 环境 + 日志，≤2 MB），
   **没有自动打开所在文件夹**（避免新起 explorer 进程触碰用户环境）；
   只生成文件并显示提示。T04.5 的日志按天轮转未做。
8. **A-29 端到端改色**：仍为人工验收项，自动证据未取得（SP-2 结案口径不变）。
9. **诊断面板自身截图**：仍按 K-06 标「未取得」，用 A-02 主截图与
   EX-24 / A-20 真实屏幕采样区截图替代。

### 11.10 全局一致性审查

按 `IS_PASS` 流程，在**所有文件写完后**做一次跨文件核对（不做逐文件复查）：

| 检查项 | 结果 |
|---|---|
| 跨文件 import 一致性 | ✅ `config` / `ui` / `app` / `window` / `tray` / `ipc_server` / `engine` 互相引用均无缺失；4 种特性组合全部 `cargo check`/`clippy` 通过，无循环依赖（Rust 模块树本身无环） |
| 接口契约 | ✅ `Widget` / `View` / `App` / `EngineSession::set_config` / `VerifyProfile::endpoints_only` 等签名与调用点一致；`ConfigSource::snapshot` 与 `EngineConfig.version` 同拍生效链路核对通过 |
| 数据流 | ✅ `Config`（i64 值表）→ `to_engine_config()` → `EngineSession::set_config` → core `Engine::tick`；`DiagSnapshot` → `RecordingDiagSink` → `DiagView` → `View::paint`；`ViewAction` → `App::apply_actions` → 落盘。类型/字段逐层对齐 |
| 重复实现 | ✅ 分位数统一走 `RecordingDiagSink::cost_stats_of`；双语查表只有 `i18n::text` 一处；配置项只有 `SettingKey::ALL` 一张表；ZIP CRC32 与 PNG CRC32 各自独立且单测交叉核对 |
| 禁用项扫描 | ✅ 运行时零 `panic!` / `unwrap()` / `expect(` / `unimplemented!` / `todo!` / `UI_Xaml*`；构建脚本 `panic!` 属白名单例外；`unwrap_or_else` 用于 Mutex 中毒降级，不是吞错 |
| 体积 / 门禁 | ✅ 三产物各跑 5 道门禁，A-26 增量断言，`BUILD OK` |
| 真机证据 | ✅ A-13/A-14 独立 session `2/2 obtained`、稳态 flips=0；NFR-03 双口径记录齐备；UIA 命中 0 如实上报 |

**IS_PASS: YES**

### 11.11 本批最大的坑

1. **「隔离取证代码 = 体积达标」是假推断**：只按任务书 gate 三个文件后体积反而
   从 409.0 KB 涨到 428.0 KB。用「移除采样线程」对照实验定位到 `std::thread`
   仅 ~1 KB、真正大头是 `uia.rs` 的 COM + MTA 线程；再结合 K-05 的
   「UIA 命中 0，产品本来就在走 fallback」才敢把 UIA 适配器也门到取证档。
   **没有先测量就下结论，会白做一轮。**
2. **独立 session 反而制造了新的编排伪影**：T04 首轮 `flips=1` 是共用 session
   切背景的段首伪影；换成独立 session 后，旧代码「先 tick 一拍建几何」把
   EWMA 种子喂成暗端，纯白段内出现一次**收敛翻转**，且 `flips_head_excluded=false`
   ——「排除段首一拍」的口径救不了它。第一次重跑 1/2，如实上报、没有改判据、
   没有硬凑；根因定位后改为 `read_probe` 只读建几何，让第一拍就采到目标背景，
   才拿到 2/2。**判据不放宽，改的是编排。**
3. **冷启动 Judge 的 EWMA 初值语义**：`Judge::update` 首次以 `stats.y` 直接作
   初值、不经过 α 平滑，所以「第一拍背景正确 ⇒ 零收敛瞬态」是成立的前提；
   任何在第一拍之前插入的采样（哪怕只是为了拿几何）都会破坏它。
4. **PowerShell 5.1 的函数返回值会被管道展开**：`Invoke-Gates` 最初
   `return ,$failures` 会把 `List[string]` 整个当成一个元素塞进
   `List[string].Add()`，改成 `return $failures.ToArray()` 才让三个产物的
   失败项合并正确。
5. **`windows` 0.58 的 `Shell_NotifyIconW` 收 `*const NOTIFYICONDATAW` 且返回
   裸 `BOOL`**、返回 `Result<HWND>` 的 `FindWindowW`、`CreateMutexW` 的
   `P0` 泛型布尔参数、`SetClipboardData` 的 `HANDLE` 与 `HGLOBAL` 逐字段转换、
   `CF_UNICODETEXT` 在 `Win32::System::Ole` 而非常规位置 —— 全部先 grep
   registry 源码核准签名再写，避免了一轮试错。
