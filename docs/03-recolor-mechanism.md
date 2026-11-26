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
| F15 | **托盘系统字形图标** = `SystemTray.AdaptiveTextBlock` 下名为 `Base` 的 `InnerTextBlock`；`Underlay` / `AccentOverlay` 是透明或强调覆盖层，写 `Foreground` 不可见 | 截图 `shot_tray.bmp`：麦克风/输入法/显示器/音量变绿，QQ/电量等第三方位图**不受影响**；v1.4.0 起按 Base 层收集 |

---

## 3. 元素定位结果（按 Name 定位，不依赖符号/偏移/版本）

| 目标 | XAML 元素 | Name / 判定条件 | 实测值 |
|---|---|---|---|
| **时间** | `Windows.UI.Xaml.Controls.TextBlock` | `TimeInnerTextBlock` | `Text = "7:14"` |
| **日期** | `Windows.UI.Xaml.Controls.TextBlock` | `DateInnerTextBlock` | 与时间同父 `SystemTray.DateTimeIconContent` |
| **托盘系统字形图标** | `Windows.UI.Xaml.Controls.TextBlock`（`InnerTextBlock`） | 父级为 `SystemTray.AdaptiveTextBlock`，且父级名 `Base` | 只染 Base 层；`Underlay` / `AccentOverlay` 宽 0 或强调覆盖，不染；第三方图标走 `ImageIconContent`，不染 |
| **图标状态条** | `Windows.UI.Xaml.Shapes.Rectangle` | `RunningIndicator` | `Height=3`、`RadiusX/Y=1.5`、`Fill`=画刷；**正常 `ActualWidth=6`，选中 `ActualWidth=16`** |
| 图标背景（选中/悬停） | `Border` / `Rectangle` | `BackgroundElement` / `BackgroundBorder` | 预留，v1.1 不做 |
| 图标区容器 | `SystemTray.StackListView` | `IconStack` | — |
| 托盘图标项 | `SystemTray.IconView` | `SystemTrayIcon` | — |
| 通知区 | `SystemTray.NotificationAreaIcons` / `NotifyIconView` | `NotifyItemIcon` | — |

> **状态区分方法**：状态条的「正常 / 选中」靠 **`ActualWidth`**（6 / 16）判定，**不依赖任何状态事件**；
> 每次应用前重新读，天然跟随系统状态变化，无需监听。
>
> **v1.5 实测补充**：`OnElementStateChanged` 对 `RunningIndicator` 的选中/正常切换**不会触发**，
> 只靠事件会让颜色滞留到宿主下一次下发（约 2 s）。现在载荷在 XAML UI 线程每 **250 ms**
> （空闲 **1 s**）只读一次 `ActualWidth`（`GetValue`，不写树），变化即置脏并置急，
> 主循环下一拍应用；实测切换延迟 **0.14–0.48 s**。
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
7. 退出时必须**写回缓存的原色**（不是 `ClearProperty`，见下方例外）。常规退出**不卸订阅**。
   **订阅只建立一次，不做重放**：第二次 `AdviseVisualTreeChange` 会返回 `E_UNEXPECTED`
   并破坏唯一有效的连接（v1.5.2 真机结论）；新元素由 `Add/Remove` 增量事件补齐。

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
  ├─ 采样：Shell_TrayWnd 真实矩形 → 顶边下 3/5/7 px → 三段（时钟 93~99% / 托盘 84~93% / 图标 0~84% 可调）
  │        每段三行取中位数（抗图标污染）
  ├─ 判定：EWMA + 双阈值迟滞，默认分区独立（可切全局统一）
  ├─ IPC ：Local\ShadeBar.Ipc.v1（80 字节共享内存，含心跳/状态/四个颜色）
  └─ 注入：CreateRemoteThread + LoadLibraryW，explorer 重启后按 pid 变化自动重注入
C++ 载荷 (ShadeBarTap.dll, ~220 KB)
  ├─ TAP 席位 → SetSite(XAML 线程) → Advise → 按 Name 扫描元素
  ├─ CoreDispatcher 编组 → SetProperty(Foreground/Fill)
  ├─ 心跳看门狗：宿主消失 7 s → 还原 + 常驻待命
  ├─ 元素更新：只订阅一次 + Add/Remove 增量事件（不重放）
  └─ 退出：还原原色，不卸订阅
```

**已实测通过的完整生命周期**：启动→改色 ✅ ｜ 宿主被强杀→7 s 自动还原且状态条不消失 ✅ ｜
宿主重启（不重启 explorer）→载荷恢复工作并重新改色 ✅ ｜ 版本升级→新文件名加载新载荷 ✅

**体积**：载荷 DLL 220 KB（224,768 B，静态 CRT），宿主 exe 674 KB（690,176 B，内嵌载荷），
仍远低于 NFR-01 的 4096 KB 硬限；**交付仍为单个 exe**（运行时释放载荷到 `%LOCALAPPDATA%\ShadeBar\`）。

---

## 5.2 v1.1 的交付边界（哪些改得动、哪些改不动）

| 元素 | 结果 | 依据 |
|---|---|---|
| 时间 `TimeInnerTextBlock.Foreground` | ✅ 稳定 | 改色后连续 60 s 保持（`s5.bmp` → `s60.bmp`） |
| 日期 `DateInnerTextBlock.Foreground` | ✅ 稳定 | 同上 |
| 托盘字形 `InnerTextBlock.Foreground`（Base 层） | ✅ 稳定 | v1.4.0 真机：麦克风/网络/显示器/音量变红；第三方位图不动 |
| 指示条**正常态** `RunningIndicator.Fill` | ✅ 可用 | `ActualWidth=6` 的那批写入有效；所有正常态共用同一个画刷 |
| 指示条**选中态** | ✅ 已解决（v1.1.3） | 改画刷自身 `Color` 可穿透 VisualState；必要时逐层染色，写完回读校验 |
| 悬停高亮块 `BackgroundElement` | ⚠ 未验证 | 探针阶段 `Background` 写入曾生效（`parent_exp.bmp` 图标底色变青），但未做稳定性验证，暂不开放 |

> 判定「改色是否生效」不能只看 `SetProperty` 的返回值：XAML 对已被视觉状态接管的属性
> 会**接受写入并立即覆盖**。唯一可信的验收方式是**截图看像素**。

---

## 5.3 选中态：**已解决**（v1.1.3；根因是属性优先级，不是"改不动"）

v1.1.2 曾判定「无副作用路径全部证伪」。那个判断**只对了一半**：
`SetProperty` 这条路确实走不通，但原因不是"被当帧接管、抓不到时机"，
而是**写进了被永久压住的层级**。v1.1.3 找到根因并已落地。

### 根因：VisualState 往 Local 层插了一个更高优先级的槽

用 `GetPropertyValuesChain` 打印 `RunningIndicator.Fill` 的属性链，两态对比：

```
正常态（props=242，2 层）
  Fill = '798780488'  base=Local(4)
  Fill = '0'          base=Default(1)

选中态（props=243，3 层）
  Fill = '251114040'  base=Local(4)   ← ① VisualState 写的
  Fill = '798780488'  base=Local(4)   ← ② 普通本地值（SetProperty 写这里）
  Fill = '0'          base=Default(1)
```

`xamlom.h` 的 `BaseValueSource` 枚举里本来就有 `BaseValueSourceVisualState` 一档，
且排在 `Animation` 之后。也就是说 WinUI 的 `VisualState.Setters` **不是动画层**，
而是往 Local 里插一个优先级更高的槽。

这解释了三件此前互相矛盾的现象：
- 写入返回 `S_OK` —— 确实写进了普通 Local 槽；
- 回读能读到新值 —— 诊断接口读的是**属性链**，不是有效值；
- 视觉一帧不变 —— 渲染取的是有效值，也就是 ① 那一层。

所以"保持 15 秒"注定无效：不是抢不到时机，是层级被永久压住。

### 决定性实验：7 条指示条逐条染不同色（`mode=allind`）

遍历全部 `RunningIndicator`，按顺序给红/绿/蓝/黄/青/品红/橙，保持 14 s 后
截全任务栏、按 x 坐标逐条读平均色：

| 元素 | 宽度 | 期望 | 实测 | 结果 |
|---|---|---|---|---|
| #0 | 6 | 红 | (251,32,34) | ✅ |
| **#1** | **16** | **绿** | **(13,126,213)** | ❌ 系统蓝，纹丝不动 |
| #2 | 6 | 蓝 | (32,32,252) | ✅ |
| #3 | 6 | 黄 | (250,250,35) | ✅ |
| #4 | 6 | 青 | (33,250,252) | ✅ |
| #5 | 6 | 品红 | (251,32,252) | ✅ |
| #6 | 6 | 橙 | (251,142,35) | ✅ |

6/7 成功，唯一失败的就是宽 16 那条 —— 这同时排除了"改错元素"的假设。

### 解法：改画刷对象自身的 `Color`

选中态 `Fill` 指向的画刷，其 `Color` 是 `FF0078D4`。直接对**画刷对象**设 `Color`：

```
CreateInstance("Windows.UI.Color", "#FFFF0000") → hr=0x0
SetProperty(brush, colorObj, idx=737)          → hr=0x0
截图：x=157..172（宽16）RGB=(254,13,13)
```

原理：状态切换只改「`Fill` 指向哪个画刷」，不重建画刷对象。改 `Color`
不经过依赖属性优先级，因此穿透。

引用计数（`mode=refs`，扫描 301 个元素）：

| 画刷 | 引用次数 | 引用者 |
|---|---|---|
| 选中态画刷 | **1** | 仅当前那条 `RunningIndicator` |
| 正常态画刷 | **29** | 6 条 `RunningIndicator` + 一批 `DefaultIcon` |

结论：**选中态改画刷是安全的**；正常态继续用原来的"新建画刷赋 `Fill`"，
不碰那个被 29 处共享的画刷。

### 落地（`payload/ShadeBarTap.cpp`）

新增 `BrushHandleOfProp` / `ReadColorOfBrush` / `SetBrushColorDirect` / `SetBrushColorOfProp`；
`ApplyColors` 按状态分流：选中态 → 改画刷 `Color`，正常态 → 赋新画刷；
`RestoreAll` 三路保障（选中态走画刷、正常态走 `Fill`、按内容兜底）。

同时修掉一个隐藏 bug：`RestoreAll` 会把 `g_applied` 归零，而共享内存里的 `seq`
仍是最后一条命令，主循环看到 `seq != g_applied` 就立刻重新上色 ——
表现为「还原 `ok=1`、读回也是原色，但屏幕又被刷回自定义色」。
心跳超时分支现在把 `g_applied` 标为当前 `seq`。

真机全链路（2560×1440 / 100% DPI）：

| 阶段 | 选中条 RGB |
|---|---|
| 基线（无随影） | (15,127,214) |
| 随影运行中 | (148,149,153) |
| 强杀宿主后 | (13,126,213) |

### 早期实验（v1.1.2，保留作误判复盘）

当时的实验本身没有错，错在把"回读落空"解读成"被当帧接管"，
从而推出"抢时序没有窗口"这个不成立的结论。以下记录保留，供后续避免重复踩坑。

### 5.3.1 子树属性存活筛选（`mode=surv`）

对选中态 `TaskListButton` 的**整棵子树**（9 个元素）逐个尝试写入 4 个画刷属性
（`Fill` / `Background` / `Foreground` / `BorderBrush`），写品红后等 3 s 回读：

| 元素 | 属性 | 写入 | 3 s 后回读 |
|---|---|---|---|
| `TaskListButton` | Background / Foreground / BorderBrush | S_OK | 读不到本地值 |
| `IconPanel`(Grid) | Background / BorderBrush | S_OK | 读不到本地值 |
| `MostRecentlyUsedDivider`(Rectangle) | Fill | S_OK | 读不到本地值 |
| 匿名 Border | Background / BorderBrush | S_OK | 读不到本地值 |
| `BackgroundElement`(Border) | Background / BorderBrush | S_OK | 读不到本地值 |
| `MultiWindowElement`(Border) | Background / BorderBrush | S_OK | 读不到本地值 |
| **`RunningIndicator`(Rectangle)** | **Fill** | S_OK | **读不到本地值** |
| `DefaultIcon`(Rectangle) | Fill | S_OK | 读不到本地值 |

**14 项全部返回 `S_OK`，3 s 后全部读不到本地值。**

### 5.3.2 直接盯着选中态短条（`mode=hold`）

滑块筛选的回读结果有歧义（读不到 ≠ 没生效），所以补一个只看像素的实验：
定位 `ActualWidth=16` 的选中态 `RunningIndicator`，只写它的 `Fill`，保持 15 s 不恢复，
在 t+2 s / t+8 s 各截一次全任务栏（2560×110），统计品红像素（±60）。

结果：**两次都只有 1 个品红像素，位于托盘区 (2300,88)，是指示条以外的图标自身颜色。
短条颜色零变化。**

### 5.3.3 当时的结论（v1.1.3 已推翻前半段）

- ~~选中态短条由视觉状态「当帧接管」~~ → **错**。实际是 VisualState 往 Local 层
  插入了更高优先级的槽，我们的值一直在，只是永远不是有效值。
- `SetProperty` 写入既不报错也不留痕、连一帧都抓不到 —— 现象描述正确，
  但归因错了，导致「抢时序没有窗口」这个结论不成立。
- 子树里其他元素的画刷属性同样被视觉状态接管（回读全部落空）。
- 旁证：非选中态按钮的 `TaskListButton.Background` 写入**视觉上确实生效**
  （截图出现品红方块，探针清理前可见）——说明写入链路本身没问题，
  失效的是「选中态」这一特定状态下的属性。
- 因此 `progress.md` §2.4 的方向 2、3 **可以划掉**，不要再试。

### 当时以为剩下的两条路（现已被取代）

1. ~~改系统强调色~~ —— 不再需要，改画刷已能实现且无全局副作用。
2. ~~S-19 明暗跟随兜底~~ —— 同上。

> 复现：`cd scratch/tap_probe && bash build.sh`，`mode.txt` 写
> `allind`（逐条染色）/ `chain`（属性链）/ `refs`（引用计数）/ `brush`（改画刷试色）/
> `surv` / `hold` / `clean`，`injector.exe` 注入（每次换 DLL 文件名）。

---

## 5.4 冷启动整条链路失效（v1.5.2）

**现象**：开机后任务栏时间、日期、托盘、指示条全部不再着色；软件仍在运行，
只有重启 explorer 才恢复。v1.5.1 曾把范围缩小到"只有指示条"，但真机再次复现时
时间/日期也已失效。

**日志证据**（故障会话）：
`[19:19:02] [Replay] #2 reason=apply hr=0x8000FFFF` →
重建会话后 `[Tap] Advise hr=0x8000FFFF` → 此后每 10 s 一条
`[Init] 复用会话重新订阅 hr=0x8000FFFF`，持续 700+ 次。

**根因**：`AdviseVisualTreeChange` 对一个 XAML 诊断连接只允许成功一次。
第二次调用（无论先 `Unadvise`、还是换新的回调对象）都会返回 `E_UNEXPECTED`；
如果重放时先退订，唯一有效的订阅就被拆掉，整条链路（含时间/日期）立即失效。
之后再 `InitializeXamlDiagnosticsEx` 拿到的是同一条已失效的连接，`Advise` 永远失败。

**决定性实测**：保持订阅、完全不重放时打开计算器，任务栏新增一条指示条，
载荷**未收到显式 Add 日志但元素已进入树并即时着色**（ind 4 → 5），
说明 XAML 会持续投递增量事件，重放既无必要也有害。

**修复**（`payload/ShadeBarTap.cpp`）：
- 删除 `ReplayTree` 与全部触发点（宿主重连、指示条缺失、定时兜底）；
- 删除会话重建、启动预热、重放退避；
- 只保留一次订阅 + `Add/Remove` 增量事件；
- `Add` 诊断日志预算 60 → 200，便于观察新窗口元素。

**回归**：`tools/check_payload_sync.py` C7 改为「不发起自动重放」（出现 `[Replay]`
即判失败）；C2 改看 `[Tap] Advise hr=0`。真机 explorer 重启后 C1–C7 全 PASS、
新开计算器指示条正常着色。

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
