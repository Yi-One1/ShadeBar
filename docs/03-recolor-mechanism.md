# 随影 ShadeBar — 改色机制验证报告（v1.1 技术基座）

**日期**：2026-11-18
**结论**：🟢 **改色可行，已完成真机闭环验证（改色 + 还原 + 零崩溃）**
**性质**：推翻 v1.0.0「改色不可用」结论（README K-08 / 需求基线 v1.5 §4.6）

---

## 1. 为什么 v1.0.0 的结论要改

v1.0.0 把「时间 / 日期 / 悬停块 / 指示条」标为**永久移出**，依据是 A-29 真机证伪。但被证伪的其实只是**两条具体实现路线**：

- 叶函数**近跳转 `E9`** 打补丁 → 真机 31 s `EXPLORER_DIED`，故障码 `0xC0000409`（fail-fast）
- 在 `OnApplyTemplate` **模板构建期走查 XAML 改色** → `maxRed=0`，explorer 22 s 死亡

这两条的共同错误是：**在任务栏构建期重入、跨时点持有瞬态元素、用机器码补丁改别人函数**。
它们不能推出「改色不可行」。

**反证**（本机实测，同日）：

1. 本机 explorer 进程内常驻 `ExplorerTAP.dll`（TranslucentTB 提供），长期稳定；
2. 本机 CTM 工具的注入模块含 `IXamlDiagnostics / IVisualTreeService3 / TextBlock / SolidColorBrush`，
   即在 **XAML 元素级换画刷**，且其配置里「图标状态条颜色开关 = 开」并已生效。

---

## 2. 已验证的技术事实（全部本机真机实测）

| # | 事实 | 证据 |
|---|---|---|
| F1 | `Windows.UI.Xaml.dll` 导出 `InitializeXamlDiagnosticsEx` | `probe.log: GetProcAddress -> 00007FFBF3AA7C30` |
| F2 | **即使 TranslucentTB 占用 TAP，仍能抢到席位**，`VisualDiagConnection1` 直接 `S_OK` | `抢席位 okAt=1 last=0x00000000` |
| F3 | COM 契约：`CoCreate(clsid)` → QI `IObjectWithSite`(`FC4801A3-…`) → `SetSite(IXamlDiagnostics*)` | `[CreateInstance]/[QI]/[SetSite]` 日志 |
| F4 | site 同时支持 `IXamlDiagnostics` / `IVisualTreeService3` / `IVisualTreeService2` | `QI … hr=0x0` ×3 |
| F5 | **`AdviseVisualTreeChange` 必须在 XAML UI 线程调用**；换线程调用返回 `E_UNEXPECTED(0x8000FFFF)` | 同线程 `hr=0x0` vs 工作线程 `hr=0x8000FFFF` |
| F6 | **所有属性读写必须在 UI 线程**；跨线程返回 `RPC_E_WRONG_THREAD(0x8001010E)` | 33 个 TextBlock 全部 `0x8001010E` |
| F7 | UI 线程编组可行：`IXamlDiagnostics::GetDispatcher` → QI `ICoreDispatcher` → `RunAsync` | `[UI] 进入 XAML UI 线程 tid=10376` |
| F8 | **回调对象必须实现 `IAgileObject`**，否则 `RunAsync` 返回 `E_INVALIDARG(0x80070057)` | 补 IAgileObject 后 `hr=0x0` |
| F9 | **`GetPropertyIndex` 对任务栏元素返回 `E_INVALIDARG`，不可用**；须改用 `GetPropertyValuesChain` 里的 `PropertyChainValue.Index` | `GetPropertyIndex hr=0x80070057` → `IndexFromChain ok=1 idx=1167` |
| F10 | `CreateInstance(L"Windows.UI.Xaml.Media.SolidColorBrush", L"#AARRGGBB")` 可直接建带色画刷 | `hr=0x0 brush=0x30B734B8` |
| F11 | `SetProperty(element, brush, idx)` 改文字色**生效** | 截图 `shot_clock.bmp`：时间/日期变红 |
| F12 | `ClearProperty(element, idx)` **可还原系统默认** | 截图 `shot_restored.bmp` / `shot_restored2.bmp` / `shot_final.bmp`：全部复原 |
| F13 | 全过程 explorer **零崩溃**（pid 9508 稳定，跨 17 次注入） | 事件日志无新增 1000 |
| F14 | **图标状态条** = `Rectangle`，`Name=RunningIndicator`；`Fill` 可改 | 截图 `shot_icons.bmp`：全部变品红 |
| F15 | **托盘系统字形图标** = `SystemTray.AdaptiveTextBlock` 三层（`Underlay`/`Base`/`AccentOverlay`），`Foreground` 可改 | 截图 `shot_tray.bmp`：麦克风/输入法/显示器/音量变绿，QQ/电量等第三方位图**不受影响** |

---

## 3. 元素定位结果（按 Name 定位，不依赖符号/偏移/版本）

| 目标 | XAML 元素 | Name / 判定条件 | 实测值 |
|---|---|---|---|
| **时间** | `Windows.UI.Xaml.Controls.TextBlock` | `TimeInnerTextBlock` | `Text = "7:14"` |
| **日期** | `Windows.UI.Xaml.Controls.TextBlock` | `DateInnerTextBlock` | 与时间同父 `SystemTray.DateTimeIconContent` |
| **托盘系统字形图标** | `SystemTray.AdaptiveTextBlock` | `Underlay` / `Base` / `AccentOverlay`（每个图标 3 层，共 5 组） | `Foreground` 可改；第三方位图走 `ImageIconContent`，不染 |
| **图标状态条** | `Windows.UI.Xaml.Shapes.Rectangle` | `RunningIndicator` | `Height=3`、`RadiusX/Y=1.5`、`Fill`=画刷；**正常 `ActualWidth=6`，选中 `ActualWidth=16`** |
| 图标背景（选中/悬停） | `Border` / `Rectangle` | `BackgroundElement` / `BackgroundBorder` | 预留，v1.1 不做 |
| 图标区容器 | `SystemTray.StackListView` | `IconStack` | — |
| 托盘图标项 | `SystemTray.IconView` | `SystemTrayIcon` | — |
| 通知区 | `SystemTray.NotificationAreaIcons` / `NotifyIconView` | `NotifyItemIcon` | — |

> **状态区分方法**：状态条的「正常 / 选中」靠 **`ActualWidth`**（6 / 16）判定，**不依赖任何状态事件**；
> 每次应用前重新读，天然跟随系统状态变化，无需监听。
>
> ⚠ **必须用 `ActualWidth` 而不是 `Width`**：实测多数会话里 `Width` 的值是字符串 `"Auto"`，
> 解析失败会让所有状态条都被当成「正常态」（v1.1 踩坑）。
>
> **重要陷阱**：`RunningIndicator` 是元素的 **Name**，元素 **Type** 是 `Windows.UI.Xaml.Shapes.Rectangle`。
> 按 Type 匹配会全部漏掉（本次踩过）。

---

## 4. 关键工程约束（写进架构，避免重蹈覆辙）

1. **禁止**任何形式的机器码补丁 / 近跳转 / 内联 hook。只走 `IXamlDiagnostics` 官方接口。
2. **禁止**在 `OnApplyTemplate` 期间或任务栏构建期操作元素。
3. **禁止**跨时点持有元素指针：句柄只当次有效，用前必须校验（`GetIInspectableFromHandle` 失败即丢弃）。
4. 一切 XAML 调用**必须**经 `ICoreDispatcher::RunAsync` 回到 UI 线程；回调对象必须 `IAgileObject`。
5. 属性索引**只能**从 `GetPropertyValuesChain` 取，不能用 `GetPropertyIndex`。
6. 订阅失败（`AdviseVisualTreeChange != S_OK`）必须**立即收手**，不得继续访问树（v1 探针正是在此之后 AV 崩溃）。
7. 退出时必须**写回缓存的原色**（不是 `ClearProperty`，见下方例外），且**不要**调
   `UnadviseVisualTreeChange`：卸载订阅后同一进程内无法再恢复，等于自断后路。
   只有 explorer 退出才真正清空。

> ⚠ **例外（v1.1 实测踩到的生产事故）**：`RunningIndicator` 的 `Fill` **不能用 `ClearProperty` 还原**。
> 该属性由视觉状态（VisualState）驱动，清空本地值后没有任何机制重新赋值，状态条会变成**全透明**，
> 表现为「运行中程序的指示条消失」。
>
> **正确做法**：首次覆写前先读出原画刷颜色并缓存，退出时**写回原色**。
> 原色读取路径：元素属性链取 `Fill` 的非零画刷句柄 → 读该句柄属性链的 `Color`（形如 `#72000000`）。
> 实测缓存值：正常态 `#72000000`（半透明黑）、选中态 `#FF0078D4`（系统强调色）。
> 只在读不到缓存时才退回 `ClearProperty`。证据：`evidence/recolor-spike/t_apply.bmp`（绿）→ `t_restore.bmp`（灰条仍在）。

---

## 5. 已排除的错误做法（本次踩坑记录）

| 现象 | 真因 | 修法 |
|---|---|---|
| `0x8000FFFF` E_UNEXPECTED | 在自建线程调 `AdviseVisualTreeChange` | 移到 `SetSite`（XAML 线程）内调用 |
| `0x80070057` E_INVALIDARG（RunAsync） | 回调未实现 `IAgileObject` | QI 里放行 `IAgileObject` |
| `0x8001010E` RPC_E_WRONG_THREAD | 工作线程读属性 | `CoreDispatcher::RunAsync` 编组 |
| `0x80070057`（GetPropertyIndex） | 该 API 对任务栏元素不可用 | 从属性链取 `Index` |
| explorer `c0000409` | 用 `GetExitCodeThread` 取 64 位模块基址被截断，远程线程跳到野地址 | 改由 DLL 自报基址 |
| **状态条消失（用户可见故障）** | `ClearProperty(Fill)` 清掉视觉状态驱动的属性，无人重新赋值 → 全透明 | 改「写回原色」；无缓存则**保持现状不清空** |
| 载荷代码改了但行为没变 | `LoadLibrary` 命中 explorer 里**已加载的同名模块**，`DllMain` 不再执行 | 载荷文件名带版本：`ShadeBarTap_v{ver}.dll` |
| 同上（又一次） | 释放 DLL 时"大小相同则复用"导致复用旧产物 | 每次启动**强制覆盖写** |
| 载荷超时后永久失效 | 超时后线程退出，但 DLL 仍留在 explorer，后续注入全是空操作 | 超时后**还原并转入常驻待命**，心跳恢复即继续工作 |
| 还原被跳过（`从未改过色`） | `g_uiDoneEvent` 是手动重置事件且未复位，第二次 `Wait` 立即返回，标记在 UI 线程执行前被清零 | `PostToUi` 投递前 `ResetEvent` |
| **第二次启动随影后再也不改色** | `ensure_payload_dll` 覆盖写「已被 explorer 映射」的 DLL 失败 → 静默返回 `None` → 配色服务根本没起来，且**没有任何日志** | 写失败即视为「同名同内容，复用」；文件名已含内容指纹 |
| 宿主正常退出后载荷永久失效 | 收到 `shutdown` 直接 `break` 退出循环，但 DLL 仍驻留 explorer，后续注入命中已加载模块、`DllMain` 不执行 | 改为「还原 + 清零状态 + 继续待命」，不退出、不 Unadvise |
| 多个载荷副本互相覆盖 | 同一 explorer 里驻留多个版本时共享同一块 IPC，旧副本用旧逻辑持续覆盖新结果 | 代际令牌（`reserved[1]`）：后注入者接管，旧副本静默退位且**不还原** |
| 强制端色不彻底 | S-10 用「把阈值推到 0/1」实现，加减迟滞带后仍会落进区间 | `PolicyCfg.force` 直接返回端色，不再走阈值 |

---

## 5.1 已跑通的垂直切片（v1.1 当前状态）

```
Rust 宿主 (shadebar-app)
  ├─ 采样：Shell_TrayWnd 真实矩形 → 顶边下 3/5/7 px → 三段（时钟 93~99% / 托盘 84~93% / 图标 25~70%）
  │        每段三行取中位数（抗图标污染）
  ├─ 判定：EWMA + 双阈值迟滞，默认分区独立（可切全局统一）
  ├─ IPC ：Local\ShadeBar.Ipc.v1（80 字节共享内存，含心跳/状态/四个颜色）
  └─ 注入：CreateRemoteThread + LoadLibraryW，explorer 重启后按 pid 变化自动重注入
C++ 载荷 (ShadeBarTap.dll, ~206 KB)
  ├─ TAP 席位 → SetSite(XAML 线程) → Advise → 按 Name 扫描元素
  ├─ CoreDispatcher 编组 → SetProperty(Foreground/Fill)
  ├─ 心跳看门狗：宿主消失 7 s → 还原 + 常驻待命
  └─ 退出：还原原色 → Unadvise
```

**已实测通过的完整生命周期**：启动→改色 ✅ ｜ 宿主被强杀→7 s 自动还原且状态条不消失 ✅ ｜
宿主重启（不重启 explorer）→载荷恢复工作并重新改色 ✅ ｜ 版本升级→新文件名加载新载荷 ✅

**体积**：载荷 DLL 206 KB（静态 CRT），宿主 exe 由 398.5 KB 增至约 621 KB（内嵌载荷），
仍远低于 NFR-01 的 4096 KB 硬限；**交付仍为单个 exe**（运行时释放载荷到 `%LOCALAPPDATA%\ShadeBar\`）。

---

## 5.2 v1.1 的交付边界（哪些改得动、哪些改不动）

| 元素 | 结果 | 依据 |
|---|---|---|
| 时间 `TimeInnerTextBlock.Foreground` | ✅ 稳定 | 改色后连续 60 s 保持（`s5.bmp` → `s60.bmp`） |
| 日期 `DateInnerTextBlock.Foreground` | ✅ 稳定 | 同上 |
| 托盘字形 `AdaptiveTextBlock.Foreground` | ✅ 稳定 | `G1.bmp`：系统图标变青，第三方位图不动 |
| 指示条**正常态** `RunningIndicator.Fill` | ✅ 可用 | `ActualWidth=6` 的那批写入有效 |
| 指示条**选中态** | ❌ 不介入 | `ActualWidth=16` 的元素写入返回 `S_OK`，但选中/取消选中时被视觉状态覆盖，屏幕始终显示系统强调色。**产品决策：完全不碰**，让系统按主题给对比色 |
| 悬停高亮块 `BackgroundElement` | ⚠ 未验证 | 探针阶段 `Background` 写入曾生效（`parent_exp.bmp` 图标底色变青），但未做稳定性验证，暂不开放 |

> 判定「改色是否生效」不能只看 `SetProperty` 的返回值：XAML 对已被视觉状态接管的属性
> 会**接受写入并立即覆盖**。唯一可信的验收方式是**截图看像素**。

---

## 6. 对 v1.0.0 文档的处置

| 文件 | 处置 |
|---|---|
| `README.md` K-08 | 改写为「旧路线证伪，新路线（XAML 诊断）已验证可用」，删除「不提供改色」的范围声明 |
| `docs/01-requirements.md` §4.6 | L2/L3 着色能力**从「永久移出」改回「v1.1 交付」** |
| `progress.md` §2.2 / §3.2 | T04.4 注入编排、T05 着色从「永久移出」恢复为「v1.1 核心」 |
| `sp2_manual_report.txt` | 保留为历史证据，注明「仅证伪两条旧路线」 |

---

## 7. 复现方式

```bash
cd scratch/tap_probe
bash build.sh                       # 需要 MSVC 14.44，脚本直接调 cl.exe，绕开 vcvars/reg.exe
printf 'set' > mode.txt             # dump | set | clear
./injector.exe "…\\tap_probe_vN.dll"   # 每次换文件名，避免命中已加载模块
cat probe.log
```

> 探针与注入器**不进入产品**，仅作机制验证与回归基线。
