# 随影 ShadeBar

Windows 11 任务栏**文字与图标自适应着色**工具。实时采样任务栏自身的背景亮度，
分区域判断该用浅色还是深色，然后把结果直接写到时间、日期、系统托盘图标与
运行中指示条的画刷上——不需要你手动挑颜色。

配合 TranslucentTB 使用：任务栏透明/模糊后，时钟、日期、托盘图标在浅色壁纸上
容易糊成一片，在深色壁纸上又会发灰；随影按**当前这块背景**自动决定深浅。

> **当前版本：v1.1.1**。相对 v1.0.0（只采样不落地）本版新增了真实的元素级着色能力：
> 通过 TAP 注入 + XAML 诊断接口改写元素画刷，已在 Windows 11 26H2 Build 26300 真机验证。
> 不修改系统主题、不改壁纸、不重启资源管理器。

## 它能做什么

| 元素 | 是否着色 | 说明 |
|---|---|---|
| 时间（时钟文字） | ✅ | 按时钟区背景亮度自动取浅色/深色 |
| 日期 | ✅ | 与时间同区判定 |
| 系统托盘图标（网络/音量/电池/输入法…） | ✅ | 字形图标可着色；微信、QQ 等程序自绘位图图标保持原样 |
| 运行中指示条（正常态） | ✅ | 图标下方那条短横线 |
| 运行中指示条（选中态） | ⏳ 后续版本 | 当前由系统强调色接管（XAML 视觉状态会覆盖本地值）。**已列入待办**，将换路径重试 |
| 悬停高亮块 | ❌ | 尚未验证，设置项暂不开放 |
| 开始菜单 / 通知中心 / 应用内配色 | ❌ | 不动系统主题，影响面严格限制在任务栏 |

**判定粒度是分区独立的**：时钟区、托盘区、图标区各自采样各自判定。
遇到左深右浅的壁纸，各段会各自选对颜色，而不是全任务栏一刀切。

## 工作原理

```text
任务栏自身条带（顶边下 3/5/7 px，三行取中位数）
      │  分区：时钟 93~99% / 托盘 84~93% / 图标区可调
      ▼
感知亮度 → EWMA 平滑 → 双阈值迟滞 → 二值化（亮/暗）
      ▼
浅色端 / 深色端（默认 #F2F2F2 / #1A1A1A，可自定义）
      ▼
共享内存 IPC（80 字节）→ 已注入 explorer 的载荷
      ▼
CoreDispatcher 编组到 XAML UI 线程 → IVisualTreeService::SetProperty 改画刷
```

* **采样位置很关键**：必须采任务栏**自身**的条带。TranslucentTB 模糊后任务栏显示的
  颜色与它上方的壁纸并不相同，采上方会判反（真机踩过）。
* **中位数抗干扰**：三行各取行内均值，再取三行中位数，图标/文字边缘不会把判定带偏。
* **元素按 Name 定位**，不依赖符号、偏移或版本号：`TimeInnerTextBlock`、
  `DateInnerTextBlock`、`SystemTray.AdaptiveTextBlock`、`RunningIndicator`。

细节与全部实测数据见 [`docs/03-recolor-mechanism.md`](docs/03-recolor-mechanism.md)。

## 系统要求

| 项 | 要求 |
|---|---|
| 操作系统 | Windows 11（已在 26H2 Build 26300.9711 实测）；Windows 10 未验证 |
| 架构 | x86_64 |
| 运行库 | **无需安装 VC++ 运行库**（静态 CRT） |
| 权限 | 普通用户（Medium IL），`asInvoker`，**全生命周期 UAC = 0** |
| 网络 | 完全离线，无任何出站连接 |
| 体积 | 单 exe **628 KB**（载荷 DLL 内嵌，运行时释放到 `%LOCALAPPDATA%\ShadeBar\`） |

## 与其它工具共存

* **TranslucentTB**：可共存。它的 TAP 常驻不影响随影抢到诊断席位（已实测），
  透明/模糊照常生效，随影只改元素颜色。
* **CTM 工具的同类开关**：会互相抢同一个 `TextBlock`。两者同时开启时颜色会打架，
  建议只留一个。随影不检测也不改动其它工具的设置。

## 使用

| 命令 | 行为 |
|---|---|
| 双击 / 无参数 | 单实例常驻托盘 + 采样线程 + 着色；连续双击进程数恒为 1 |
| 右键托盘图标 | 打开设置 / 暂停自适应 / 开机自启动 / 导出诊断包 / 关于 / 退出 |
| `--ui` | 直接打开设置窗口 |
| `--version` | 打印 `随影 ShadeBar v1.1.1` |
| `--help` | 打印用法 |

设置页共 **28 项可见设置**（配置里保留 35 项编号，未落地的 7 项从 UI 收起但值不丢），
分 6 组 + 诊断面板；中英双语即时切换、无「应用」按钮、改动同拍生效并落盘到
`%LOCALAPPDATA%\ShadeBar\config.json`。

开机自启走 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`：不弹 UAC、
不生成额外文件，可在任务管理器「启动」页停用。

## 构建

需要 Rust 1.99+（`rust-toolchain.toml` 锁定）、Visual Studio 2022 Build Tools 的
「Desktop development with C++」工作负载、Windows 11 SDK。

```bash
# 1) 先编 C++ 载荷（Rust 编译期会把 DLL 内嵌进 exe）
cd payload && bash build.sh && cd ..

# 2) 再编宿主
cargo build --release

# 3) 产物
target/x86_64-pc-windows-msvc/release/shadebar-app.exe
```

**顺序不能反**：`crates/shadebar-app/build.rs` 会读取 `payload/build/ShadeBarTap.dll`
并计算内容指纹，指纹进入载荷文件名（`ShadeBarTap_v1.1.1_<hash>.dll`）。
这样内容一变文件名就变，explorer 里不会因为「同名模块已加载」而静默跑旧代码。

版本号只有 `Cargo.toml` 的 `[workspace.package] version` 一个来源，
`文件名 / 关于页 / exe 属性` 三处不可能漂移。

## 已知限制

1. **需要注入 explorer**：这是元素级着色的唯一可行路径（与 CTM 工具同构）。
   写错会让任务栏重启——不会损坏系统，重启资源管理器即恢复；随影在订阅失败时
   会立刻收手，不做任何后续树操作。
2. **可能被杀软 / SmartScreen 标记**：注入行为天然敏感。若被拦截，`payload.log`
   会记录 `远程 LoadLibraryW 失败`，任务栏恢复系统默认外观，不会卡死。
3. **Windows 大版本更新后可能失效**：元素按 Name 定位，比重定位偏移稳，但微软仍可能
   改名。失效表现是「颜色不再变化」，不会崩溃。
4. **状态条选中态暂不由随影决定**：视觉状态会覆盖写入，当前保留系统强调色。
   该能力未放弃，已列入后续版本待办。
5. **第三方托盘图标不改色**：位图无法单色化，硬染会丢掉图标本身的辨识度。
6. **多显示器**：1.x 只处理含任务栏的主屏。
7. **历史记录**：v1.0.0 曾判定「任务栏改色不可用」。被证伪的是当时的两条自制路线
   （叶函数 `E9` 近跳转、`OnApplyTemplate` 期间走查 XAML），共同错误是在任务栏构建期
   重入并跨时点持有瞬态指针。v1.1 改用官方 `IXamlDiagnostics` + `IVisualTreeService`
   在 XAML UI 线程改画刷，已跑通。旧结论作废，过程记录保留在 `sp2_manual_report.txt`。

## 目录

```text
crates/shadebar-core/     共享核心库（无 UI / 无窗口 / 可单测）：引擎、几何、直方图、判定
crates/shadebar-app/      主程序：Win32/COM 适配、设置页、托盘、单实例、注入与 IPC
payload/                  C++ 注入载荷（TAP + XAML 诊断 + 画刷改写），编进 exe
resources/                图标、清单、Win32 资源脚本、内嵌双语文案（lang/*.tsv）
docs/03-recolor-mechanism.md  改色机制、元素定位表与全部踩坑记录
evidence/recolor-spike/   真机证据（改色/还原截图 + 载荷日志）
scratch/tap_probe/        开发期探针（不进产品、不参与构建）
```

## 许可

MIT License，见 `LICENSE`。变更记录见 `CHANGELOG.md`。
