# 冷开机 IVTS 非订阅式元素枚举 — 接口可行性论证

论证人：高见远（首席架构师）　日期：2026-11-27
性质：接口可行性调研 + 探针方案，不含产品代码改动。
前置结论依据：`FINDINGS.md` 与 `after/payload.log`（16:54:48 冷开机会话）。

---

## 0. 结论先行

1. **任务点名的那批方法在 XAML 诊断 API 里不存在。** `GetElementFromHandle` / `GetRootElement` / `GetParent` / `GetChildren` / Walker / `GetAnnotationObjects` / `SetAnnotation` / `GetPropertyValues` —— 这些名字来自 **UIA 客户端 API 或 WPF 托管 API**，不是原生的 `xamlom.h` 接口族。官方 xamlom.h 接口清单只有 7 个接口（见 §2），逐个核对后确认无上述方法。**不要按这些名字写代码，会编译不过。**
2. **但不新建订阅、主动拿元素的路是通的**，有两条真实可行通道：
   - **首选：`IXamlDiagnostics::HitTest`** —— 按矩形主动返回可视树内元素句柄，官方要求"调用前须 Advise"，本项目首次订阅已建立，前提天然满足；它走**独立查询路径，不依赖增量事件通道**（冷开机增量断流不影响它）。拿到的 `InstanceHandle` 可直接复用现有 `GetPropertyValuesChain` + 染色逻辑，侵入最小。
   - **备选：`IVisualTreeService::GetCollectionElements`** —— 从一个已知集合容器句柄枚举其当前全部子元素；需缓存父句柄，而项目铁律 6 规定"句柄只当次有效"，跨时点复用是主要风险，需探针验证。
3. **UIA 路线（`IUIAutomation::ElementFromHandle` + `FindAll`）接口层面成立且完全不碰 IVTS**，但它拿到的元素无法直接映射成 XAML `InstanceHandle` 去改 Fill 画刷，且 RunningIndicator 是 XAML 内部 `Rectangle`、UIA 树未必暴露其 Name——**定位价值大于染色价值**，宜作探针对照组与触发器，不宜直接当染色通道。
4. **最可行方案：`HitTest` 主动重扫，并入现有 `PollIndicatorState()` 的 UI 线程轮询。** 探针只需一次冷开机 + 一次 warm 对照。若三条路全失败，兜底见 §6。

---

## 1. 证据边界（存在性核验门）

本次所有接口结论均以微软官方 Microsoft Learn 页面为准；查不到就标注"未找到官方依据"，未凭记忆编造签名。冷开机根因以本地取证为准，未重新推断。

**根因复核（来自 `FINDINGS.md` + `after/payload.log`）**：
- `16:54:48.029 SetSite` → `16:54:48.103 Advise hr=0x00000000`（订阅成功）；同期 300+ Add 全量发放，`[Diag] ADD total=300 ind=1`。
- 此后至 `16:57:18`，`[Tree] ind=1` / `[Apply] ind=1` 恒定，`[Add]` 总数停在 201（含全量），新开记事本/浏览器无新增 Add。
- warm 会话（重启 explorer）开记事本 Add 正常到达（ind 6→7）。
- 判定：**冷开机会话 IVTS 增量事件通道不通，全量发放正常。** 所以任何"继续等 Add 事件"的修法都无效；必须走主动查询。

---

## 2. 接口核查结果（逐条，附官方来源）

### 2.1 XAML 诊断 API 全接口清单（xamlom.h）

官方来源：https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/
（页面列出全部 interface / function / struct / enum，以下据此核对）

接口：`IBitmapData`、`IVisualTreeService`、`IVisualTreeService2`、`IVisualTreeService3`、`IVisualTreeServiceCallback`、`IVisualTreeServiceCallback2`、`IXamlDiagnostics`；入口函数 `InitializeXamlDiagnosticsEx`。

**逐方法核对（关键结论：无树遍历、无按 HWND 取根元素）：**

| 接口 | 方法（官方页面） | 能否主动拿树元素 | 来源 |
|---|---|---|---|
| IVisualTreeService | AddChild, AdviseVisualTreeChange, ClearChildren, ClearProperty, CreateInstance, **GetCollectionCount**, **GetCollectionElements**, GetEnums, GetPropertyValuesChain, RemoveChild, SetProperty, UnadviseVisualTreeChange | 仅 `GetCollectionElements` 可（需集合句柄） | https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nn-xamlom-ivisualtreeservice |
| IVisualTreeService2 | GetProperty, GetPropertyIndex, RenderTargetBitmap, ReplaceResource | 否（只按已给句柄读/改属性、渲染位图） | https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nn-xamlom-ivisualtreeservice2 |
| IVisualTreeService3 | AddDictionaryItem, GetDictionaryItem, RemoveDictionaryItem, ResolveResource | 否（资源字典） | https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nn-xamlom-ivisualtreeservice3 |
| **IXamlDiagnostics** | GetApplication, GetDispatcher, GetHandleFromIInspectable, GetIInspectableFromHandle, GetInitializationData, **GetUiLayer**, **HitTest**, RegisterInstance | **是：`HitTest` 主动枚举** | https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nn-xamlom-ixamldiagnostics |

**明确否定的方法名**（在 xamlom.h 全部接口中未找到，勿用）：
`GetElementFromHandle`、`GetRootElement`、`GetParent`、`GetChildren`、任何 `...Walker`、`GetAnnotationObjects`、`SetAnnotation`、`GetPropertyValues`（原生接口里对应的是 `GetPropertyValuesChain`）。这些是 UIA 客户端 / WPF 托管侧的概念，不属于 IVTS 原生接口族。

### 2.2 首选通道：IXamlDiagnostics::HitTest

- 官方签名与备注：https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nf-xamlom-ixamldiagnostics-hittest
- 签名：
  ```
  HRESULT HitTest([in] RECT rect, [out] unsigned int *pCount, [out] InstanceHandle **ppInstanceHandles);
  ```
- 官方备注要点（原文关键句）：
  - "在 XAML 可视化树上执行命中测试，返回所有元素，无论启用或不可见"；
  - "不返回折叠元素"（不参与布局）；
  - "元素不需要完全封闭在矩形区域内"；
  - **"在此方法之前，必须调用 AdviseVisualTreeChange"**；
  - 最低支持 Windows 10，仅桌面应用，头文件 xamlom.h。
- 可行性判断：**可行**。四点理由：
  1. 它是**主动拉取**（pull），与增量事件通道（push）无关，冷开机增量断流不影响它。
  2. "调用前须 Advise" 的前提本项目已满足（`16:54:48 Advise hr=0`），且**不要求重新 Advise**——不触碰 v1.5.2 的 `E_UNEXPECTED` 红线。
  3. 返回的是 `InstanceHandle`，正是现有 `GetPropertyValuesChain` / `ReadNumber` / `BrushHandleOfProp` 的输入类型，**染色逻辑零改**、可直接复用。
  4. `GetIInspectableFromHandle` / `GetHandleFromIInspectable` 可用句柄双向转换（`IXamlDiagnostics` 官方页），后续需读 XAML 属性时不必再依赖事件带来的句柄。
- **未找到官方依据的部分（探针必须验证）**：
  - `RECT` 的坐标系（屏幕坐标 / 窗口客户区坐标 / XAML 根局部坐标）——HitTest 与 GetUiLayer 页面均未说明；GetUiLayer 官方页只说"返回用于突出显示树中元素的视觉诊断根"（https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nf-xamlom-ixamldiagnostics-getuilayer），未提坐标。
  - 返回的 `InstanceHandle` 是否"当次有效"、生命周期是否与 Add 事件句柄一致（项目铁律 6）。这决定能否跨轮询时点持有，探针要按"每次现查现用"设计以规避。

### 2.3 备选通道：IVisualTreeService::GetCollectionElements

- 官方签名与备注：https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/nf-xamlom-ivisualtreeservice-getcollectionelements
- 签名：
  ```
  HRESULT GetCollectionElements([in] InstanceHandle instanceHandle, [in] unsigned int startIndex,
      [in,out] unsigned int *pElementCount, [out] CollectionElementValue **ppElementValues);
  ```
- 官方备注：对任何集合方法，调用方应先查询元素的属性，**仅当属性带 `MetadataBit::IsValueCollection` 位时才调用**。
- 可行性判断：**条件可行，风险高于 HitTest**。它能枚举某容器"当前"子元素（含新增），但必须持有一个集合容器的 `InstanceHandle`。全量发放阶段日志里 `parent=0x19DF...` 说明父句柄在 Add 事件里出现过，可从这里缓存任务栏图标面板的容器句柄。但项目铁律 6（句柄只当次有效）意味着缓存的父句柄可能冷开机后失效或指向别的对象——须验证，且命中后仍要按 `IsValueCollection` 位过滤。
- 配套：`GetCollectionCount`（同接口）可先取 count；`CollectionElementValue` 结构见 https://learn.microsoft.com/zh-cn/windows/win32/api/xamlom/ns-xamlom-collectionelementvalue 。

### 2.4 对照组/兜底通道：UIA 客户端 API

- IUIAutomation::ElementFromHandle：`https://learn.microsoft.com/zh-cn/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-elementfromhandle`，签名 `HRESULT ElementFromHandle(UIA_HWND hwnd, IUIAutomationElement **element)`。
- IUIAutomationTreeWalker：`https://learn.microsoft.com/zh-cn/windows/win32/api/uiautomationclient/nn-uiautomationclient-iuiautomationtreewalker`，含 GetParentElement / GetFirstChildElement / GetNextSiblingElement / NormalizeElement 等，可从 HWND 根单步遍历子树。官方备注：TreeWalker 跨进程调用效率低于 FindAll；UIA 调用应在独立线程发出。
- IUIAutomationElement::FindAll：`https://learn.microsoft.com/zh-cn/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-findall`，签名 `HRESULT FindAll(TreeScope scope, IUIAutomationCondition *condition, IUIAutomationElementArray **found)`；可按 `TreeScope_Descendants` + Name 条件查后代。官方特别警示：从桌面查顶层窗口要用 `TreeScope_Children`，用 Descendants 会遍历数千项导致栈溢出。
- 可行性判断：**接口成立、染色价值低**。理由：
  1. 与 IVTS 完全独立，冷开机增量断流不影响，且能在 explorer 进程内 CoCreate 使用（官方支持桌面应用）。
  2. **关键不确定性**：RunningIndicator 是 `Windows.UI.Xaml.Shapes.Rectangle`，XAML 内部元素未必有 UIA AutomationPeer，`FindAll` 按 `Name="RunningIndicator"` 很可能**找不到**；UIA 树里任务栏图标通常表现为 Button，指示条未必是独立可寻址节点。
  3. 即便找到 `IUIAutomationElement`，也**不能直接换 XAML `InstanceHandle`** 去写 Fill 画刷（两套树、两套对象模型）。所以 UIA 更适合做"新窗口是否出现"的检测/触发器，或判定 Cold-boot 树是否刷新的探针对照组。

---

## 3. 三条路在目标场景下的判定汇总

场景：explorer 进程内、载荷已注入、已建立一次 IVTS 订阅、冷开机后用户新开窗口（新 RunningIndicator 未进 g_tree）。

| 通道 | 是否需新建订阅 | 是否依赖增量事件 | 能否定位新 RunningIndicator | 能否直接复用现有染色 | 判定 |
|---|---|---|---|---|---|
| HitTest | 否（须已有一次 Advise，满足） | 否，主动拉取 | 预期能（矩形内全部元素，含新窗口）——待探针确认坐标系 | 能（返回 InstanceHandle） | **首选** |
| GetCollectionElements | 否 | 否，主动拉取 | 条件能（需缓存有效父容器句柄） | 能 | 备选，风险在句柄时效 |
| UIA TreeWalker/FindAll | 否，完全独立 | 否 | 存疑（XAML 内部 Rectangle 未必暴露 Name） | 否（元素无法映射回 InstanceHandle） | 对照组/触发器/兜底检测 |

红线复核：三条路**都不需要第二次 `AdviseVisualTreeChange`**，不违反 v1.5.2 的 `E_UNEXPECTED` 教训，也不会弄坏首次订阅。

---

## 4. 探针设计（真机冷开机验证）

### 4.1 验证目标（一次探针回答四个问题）

- **P1（存在性）**：`IXamlDiagnostics::HitTest` 在注入载荷的 explorer 进程内，`hr` 是否 `S_OK`，`pCount` 是否 > 0？
- **P2（坐标系）**：RECT 传哪种坐标能命中 RunningIndicator？分别试 ①`Shell_TrayWnd` 的 `GetWindowRect`（屏幕坐标）②该 HWND 的 client rect ③任务栏图标区一个更紧的屏幕矩形。哪种能返回 name=RunningIndicator 的元素。
- **P3（可用性，最关键）**：对 HitTest 返回的每个 `InstanceHandle`，**立即**（同一 UI 线程调用栈内）用现有 `GetPropertyValuesChain` 读 `Name` / `Type` / `ActualWidth`，能否读到 `name="RunningIndicator"`、`type="Windows.UI.Xaml.Shapes.Rectangle"`？——这决定能否直接复用染色逻辑。
- **P4（冷开机增量断流场景下有效性）**：冷开机注入后，用户开记事本，再次 HitTest，RunningIndicator 元素数量是否从 1 增加到 ≥2（QQ + 记事本），而此时 Add 事件仍为 0。

对照组（同一探针顺带记录）：`IUIAutomation::ElementFromHandle(Shell_TrayWnd)` + `FindAll(RawViewWalker, Descendants, NameCondition("RunningIndicator"))` 命中数；确认 UIA 路线到底能不能看到该矩形。

### 4.2 判定标准

- **HitTest 方案成立**：P1 `S_OK` 且 P2 找到至少一种坐标使 P3 读到 `name=RunningIndicator` 的句柄，且 P4 冷开机新开窗口后命中数增加。四项全中 → 可直接落地"并入 PollIndicatorState"。
- **部分成立**：P1/P3 中但 P4 不中（数量不增）→ 说明 HitTest 只读到旧树快照，放弃 HitTest，转 GetCollectionElements 探针。
- **失败**：P1 非 `S_OK`，或 P3 读不到 Name/Type（句柄不可用）→ HitTest 路否决，转 UIA / 兜底。

### 4.3 侵入点（最小）

- 只在 UI 线程加一个 `UiOp::UiProbe` 分支（与现有 `UiPoll` 同构，走 `PostToUi` + `ICoreDispatcher::RunAsync`），在 `PollIndicatorState()` 同一线程上下文里调用 HitTest + GetPropertyValuesChain，结果写日志。
- 探针**只读不写**：不 SetProperty、不 ClearProperty、不碰 g_tree、不改 Advise/Unadvise、不改颜色。
- 触发方式：新增一个 IPC 标志位（复用 `ShadeBarIpc.h` 预留位）或由宿主下发一条"probe"指令触发一次；默认关闭，正常用户不受影响。
- 侵入量：新增一个 UiOp + 一段约 50–80 行的只读探测函数 + 一个 IPC 开关；**不改订阅状态机、不改染色路径**。

### 4.4 用户配合步骤

- **第 1 次冷开机（必需）**：真机重启 → 注入载荷 → 开 QQ（开机自启）→ 探针触发一次，记录 P1/P2/P3 基线 → 用户手动开记事本 → 再触发一次，记录 P4。
- **第 2 次 warm 对照（强烈建议，1 次 explorer 重启）**：重启 explorer（或注销重登）→ 重复开记事本 → 记录同样指标，作为"增量通道正常时"的对照组，用于区分"HitTest 本身不通"与"冷开机特有问题"。
- 合计：**2 次**（1 次冷开机 + 1 次 explorer 重启）；若只做冷开机也可出结论，但缺对照会使 P4 的解释力下降。

---

## 5. 风险评估

| 风险 | 影响 | 缓解 |
|---|---|---|
| HitTest RECT 坐标系猜错（官方未写） | 探针 P2/P3 读不到元素，误判方案失败 | 探针显式试 3 种坐标，且从父句柄属性反推边界；不以单次失败定论 |
| 句柄"当次有效"（铁律 6） | 不能跨时点持有 HitTest 句柄 | 设计为"每次轮询现查现用"，命中后立即染色、不缓存 |
| HitTest 前置 Advise 在冷开机受影响 | 与"增量通道不通"同源导致 HitTest 也失效 | 探针 P1 直接验 hr；若失败转 UIA/兜底 |
| UIA 看不到 XAML Rectangle | 对照组无命中 | 属预期风险，探针记录即止，不作为主方案依据 |
| 探针误写属性弄坏树 | 重演 v1.5.2 事故 | 探针硬性只读，代码中不出现 SetProperty/ClearProperty |

---

## 6. 兜底方案（全部失败时，按代价从小到大）

1. **UIA 触发 + 有限重扫**：用 UIA（或宿主 Rust 侧 `EnumWindows`/窗口创建事件）检测新任务栏按钮出现，触发载荷对"已知容器句柄"做一次 GetCollectionElements 重扫；绕开增量事件但不依赖 XAML 句柄长期有效。
2. **按已知元素签名重查**：从全量发放中记录每个 RunningIndicator 的 Name/Type/parent 三元组，冷开机后定时用 HitTest/GetCollectionElements 重查是否存在新签名，只对新签名染色。
3. **宿主侧定时重建诊断会话**：宿主检测 explorer 启动完成后重新注入一次（注意：不是在同一载荷实例里重 Advise，而是让新 DLL 实例在新会话里首次 Advise）——需评估 v1.5.2 的 DLL 常驻/`E_UNEXPECTED` 约束，风险最高。
4. **换染色层**：放弃 XAML `InstanceHandle` 写 Fill，改用 WinComp/DWM 或用 UIA 命中后的坐标在任务栏上叠加自绘层——改动大，属 v2 重构，不作为本轮方案。
5. **产品层接受限制**：维持现状，冷开机后提示用户重启 explorer 以恢复染色（当前已知 workaround）——作为最后兜底，需产品确认。

---

## 7. 给实现工程师的一句话

**先做 HitTest 探针（P1–P4 四项，2 次重启），只读不改订阅；四项全中就把主动重扫并入现有 `PollIndicatorState()`，否则按 §6 顺序降级。全程不得第二次 Advise。**
