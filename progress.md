# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-18 20:25 (GMT+8)
> **用途**：跨对话续接。新对话看不到此前聊天记录，本文件是唯一的状态依据。
> **状态一句话**：**v1.1.3 已构建并真机验证**。常驻内存 92.7 MB → **1.8 MB 工作集**、
> CPU 1.10% → **0.052%**（NFR-02 / NFR-03 双双达标）；**指示条选中态改色已完成**
> （时间/日期/托盘图标/指示条两态全部可改，退出可完整还原）。
> 剩下的是多显示器、悬停高亮块验证、文档收口。
>
> 本文档已删除 v1.0.0 阶段「改色永久移出」「K-08 不可用」等过时结论，仅保留必要的溯源说明（见 §2.2）。

---

## 0. 新对话先看这里（三十秒上手）

| 项 | 值 |
|---|---|
| 产品 | 随影 / ShadeBar —— Windows 任务栏文字与系统图标的**自适应着色**工具 |
| 工作目录 | `E:\AI工作空间\随影ShadeBar` |
| 版本 | **1.1.3**（`Cargo.toml` 的 `[workspace.package] version` 是唯一来源） |
| 仓库 | https://github.com/Yi-One1/ShadeBar ，分支 `main`，最新 `dec3c61` |
| 交付物 | `dist/随影ShadeBar_v1.1.3.exe`，单文件、静态 CRT、**UAC=0** |
| 技术栈 | Rust 2021（`windows = "=0.58.0"` 单一 crate）+ Direct2D/DirectWrite 自绘 UI + **C++ 注入载荷** |
| 可改色 | 时间、日期、系统托盘字形图标、运行中指示条**正常态 + 选中态** |
| 待攻克 | 悬停高亮块（未验证）；多显示器副屏任务栏 |
| 构建顺序 | `cd payload && bash build.sh` → `cargo build --release`（**顺序不能反**） |
| 配置 | `%LOCALAPPDATA%\ShadeBar\config.json`；载荷释放到同目录 `ShadeBarTap_v1.1.3_<hash>.dll` |
| 日志 | `%LOCALAPPDATA%\ShadeBar\shadebar.log`（宿主）、`payload.log`（载荷） |
| 测试 | **214 项全绿**（116 + 96 + 2，`cargo test --release`） |
| 内存 | 收托盘 **1.8 MB 工作集 / 4.9 MB 私有**；设置页显示时 18 MB（NFR-02 ≤20 MB 已达标） |
| CPU | 托盘常驻 **0.052%**（NFR-03 ≤0.1% 已达标，修前 1.10%） |
| 设置项 | **28 项可见**（配置里保留 35 项编号，7 项未落地项从 UI 收起，值不丢） |

**接手第一件事**：`cargo test --release` + 一次真机冒烟（§5.1）。技术风险已清空，剩下是产品打磨。

**必读**：本文件 → `README.md`（对外口径）→ `docs/03-recolor-mechanism.md`（改色机制与全部踩坑）。

---

## 1. 已完成事项

### 1.1 产品能力（全部真机验证，非推测）

| 能力 | 状态 | 证据 |
|---|---|---|
| 时间 / 日期自适应着色 | ✅ 连续 60 s 稳定 | `evidence/recolor-spike/s5.bmp` → `s60.bmp` |
| 托盘系统字形图标着色（网络/音量/电池/输入法） | ✅ 第三方位图不受影响 | `G1.bmp`（系统图标变青，QQ/100 原样） |
| 指示条正常态着色 | ✅ | `FINAL_ind.bmp` |
| 分区独立判定（时钟 / 托盘 / 图标区各判各的） | ✅ | `run2_icons.bmp`、`run2_tray.bmp` |
| 自定义端色（S-13/S-14/S-15） | ✅ | `E1.bmp`（青/品红端色上屏） |
| 强制亮端 / 强制暗端（S-10） | ✅ | `C1.bmp`（强制浅色，浅背景上可见） |
| 宿主被强杀 → 7 s 内自动还原 | ✅ 状态条不消失 | `payload.log`：`心跳超时 → 还原并转入待命` |
| 宿主重启（不重启 explorer）→ 立即恢复 | ✅ | `K1_restart.bmp` |
| 退出后完全还原系统默认 | ✅ | `L_restored.bmp` |
| 打包版独立运行 | ✅ | `PKG1.bmp`（从 dist 直接跑） |

### 1.2 技术链路（已打通并产品化）

```
Rust 宿主
  ├ 采样：Shell_TrayWnd 真实矩形 → 顶边下 3/5/7 px 三行 → 三行取中位数（抗图标污染）
  ├ 分段：时钟 93~99% / 托盘 84~93% / 图标区 0~100%（S-20/S-21 可调）
  ├ 判定：EWMA(α=0.3) + 双阈值迟滞(0.06) → 二值化 → 浅色端/深色端
  ├ 下发：共享内存 Local\ShadeBar.Ipc.v1（80 B）
  └ 注入：CreateRemoteThread + LoadLibraryW（普通权限，UAC=0）
          ↓
C++ 载荷（ShadeBarTap.dll，211968 B，内嵌进 exe）
  ├ TAP 抢席位：InitializeXamlDiagnosticsEx("VisualDiagConnection1")
  ├ 订阅：AdviseVisualTreeChange 必须在 XAML 线程（SetSite 内）
  ├ 编组：CoreDispatcher::RunAsync + IAgileObject 回调 → UI 线程
  └ 改画刷：CreateInstance(SolidColorBrush) + SetProperty(elem, brush, idx)
```

### 1.3 设置与外壳

- 设置页 6 分组 + 诊断面板，中英双语热切换，无「应用」按钮，改动同拍生效并落盘。
- 托盘：单实例、6 项菜单、左键三选一、关闭窗口最小化（S-40/S-41）。
- 开机自启：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，启动时 `reconcile` 校正一次，
  修掉「开关看着是开的、其实没生效」。
- 降级横幅：仅当载荷释放或配色服务**真的**失败才显示（1.1.0 曾误报，1.1.1 已修）。

### 1.4 构建与打包

- `crates/shadebar-app/build.rs` 读 `payload/build/ShadeBarTap.dll`，算 FNV-1a 指纹 →
  `cargo:rustc-env=SB_PAYLOAD_TAG` → 载荷文件名 `ShadeBarTap_v1.1.3_<hash>.dll`。
- 内容一变文件名就变，**升级不再需要重启 explorer**。
- 版本号单一来源，`文件名 / 关于页 / exe 属性` 三处不可能漂移。

### 1.5 已推送 GitHub

```
dec3c61 docs: 修订需求基线 §4.6 的改色裁决（v1.1 已实现）
7cbd950 chore: 移除被 1.1.1 取代的 1.1.0 产物
cda09a5 fix: 降级横幅误报，版本对齐 1.1.1 (PATCH)
8e8e244 feat: 任务栏自适应着色 (v1.1.0)
```

`.gitignore` 排除：`target/`、`scratch/`、`payload/build/*.dll`、`*.pdb`、`.workbuddy/`。
仓库共 176 个文件、约 32000 行。

---

## 2. 关键决策（含理由，防止被重新发明）

### 2.1 不重做，在 v1.0.0 骨架上继续

v1.0.0 已有 16627 行 Rust（采样、判定、配置体系、双语、单实例托盘、D2D/Mica 自绘 UI、门禁）。
新需求要的正是在这上面"上屏"。重写等于扔掉 90% 工作量，还要重踩已记录在案的十几个坑。

**但"上屏层"（注入 + XAML）是全新写的**，v1.0.0 那部分为零。

### 2.2 技术路线：XAML 元素级改色（官方诊断接口）

- **否决**「只切系统明暗主题」：只有两档、连动开始菜单/通知中心，粒度太粗。
- **采纳** TAP 注入 + `IXamlDiagnostics` / `IVisualTreeService2/3`，与 CTM 工具同构。

**v1.0.0 的"证伪"必须正确理解**：被证伪的是两条**自制**路线——
叶函数 `E9` 近跳转、`OnApplyTemplate` 期间走查 XAML。共同错误是
**在任务栏构建期重入 + 跨时点持有瞬态指针**。这推不出"改色不可行"。
`docs/01-requirements.md` §4.6 已加 v1.1 修订块，**旧结论作废但保留溯源**。

### 2.3 语言分工：Rust 宿主 + C++ 载荷

- Rust 宿主：UI/采样/判定/托盘/自启全部复用现有代码。
- C++ 载荷：`IXamlDiagnostics` 一票 COM 接口 + vtable 槽位序，C++ 现成；
  Rust 要手写 winmd 槽位，前团队已在此反复踩坑。
- **否决 C#**：self-contained ≥10 MB，与"尽量小体积"直接冲突。
- **否决纯 C**：COM/XAML 无封装。
- **否决全 C++**：16.6k 行 Rust（含已验收的 Mica+D2D UI）全部作废。

### 2.4 指示条选中态：**已解决（v1.1.3）**

**根因不是「改不动」，而是写进了被压住的层级。**

用 `GetPropertyValuesChain` 打印 `RunningIndicator.Fill` 的属性链，两态对比：

```
正常态（2 层）  Fill = <brushB> base=Local  /  Fill = 0 base=Default
选中态（3 层）  Fill = <brushA> base=Local  ← VisualState 写的
                Fill = <brushB> base=Local  ← 普通本地值（SetProperty 写这里）
                Fill = 0        base=Default
```

`xamlom.h` 的 `BaseValueSource` 里有独立的 `BaseValueSourceVisualState` 一档，
排在 `Animation` 之后。也就是说 WinUI 的 `VisualState.Setters` 不是动画层，
而是往 Local 插了一个**更高优先级的槽** —— `SetProperty` 写的普通本地值永远排在它后面。
于是出现三个看似矛盾的现象：`S_OK`、回读有值（诊断接口读属性链）、视觉不变（渲染取有效值）。

决定性实验：7 条 `RunningIndicator` 逐条染不同色，**6 条正常态全中、
唯独 `ActualWidth=16` 的选中态纹丝不动**（实测仍为 `(13,126,213)`）—— 同时排除了"改错元素"。

**解法：改 `Fill` 指向的画刷对象自身的 `Color`**，不经过属性优先级。
引用计数（扫描 301 个元素）：选中态画刷 **1 处**引用、正常态画刷 **29 处**引用
（含 `DefaultIcon`）→ 选中态改画刷安全；正常态继续用"新建画刷赋 `Fill`"，不碰共享画刷。

真机全链路：基线 `(15,127,214)` → 运行中 `(148,149,153)` → 强杀宿主后 `(13,126,213)`。

顺带修掉一个隐藏 bug：`RestoreAll` 把 `g_applied` 归零，主循环看到 `seq != g_applied`
会立刻重新上色，表现为"还原 `ok=1`、读回原色，但屏幕又被刷回自定义色"。

**已作废的旧结论**：v1.1.2 曾判定"无副作用路径全部证伪、视觉状态当帧接管、抢时序没有窗口"。
现象描述对、归因错。完整实验数据见 `docs/03-recolor-mechanism.md` §5.3。


### 2.5 设置页从 35 项收敛到 28 项可见

收起的是**未落地项**，配置字段全部保留（旧配置可平滑迁移）：
S-19（主题联动）、S-23（采样预览）、S-34~S-37（悬停块与固定色）、S-44（注入失败策略）。

语义调整：S-32 由「悬停高亮块」→「系统托盘图标」；
S-20/S-21/S-22 由「采样区偏移/宽度/内缩」→「图标区起点/宽度/采样带垂直偏移」。

### 2.6 版本号策略（严格 SemVer 2.0.0）

- 1.0.0 → **1.1.0 MINOR**：向下兼容地新增"自适应着色"能力，配置格式不变。
- 1.1.0 → **1.1.1 PATCH**：修降级横幅误报（用户可感知的 bug 就是 PATCH）。
- 1.1.1 → **1.1.2 PATCH**：修常驻内存超标（92.7 → 1.8 MB）与退出还原写错颜色；
  不新增能力、不改配置格式。
- 1.1.2 → **1.1.3 PATCH**：修选中态改色失效（属性优先级）与还原后自噬；
  不新增配置项、不改格式。
- 判定结果写进每次提交信息首段。

### 2.7 开机自启用 HKCU\Run

零 UAC、零额外进程、零侧车文件；任务管理器「启动」页可见可管。
否决计划任务（需一次提权，违背 UAC=0）、否决启动文件夹（多一个 .lnk 易残留）。

### 2.8 与其它工具共存

- **TranslucentTB**：可共存（实测它常驻 TAP 不影响我们抢席位）。
- **CTM 工具**：会抢同一个 `TextBlock`，同时开启颜色会打架。
  **不做自动检测**，只在 README 说明「建议只留一个」。

### 2.9 载荷文件名带内容指纹

固定文件名 → explorer 已加载同名模块时 `LoadLibraryW` 不再执行 `DllMain` → 改了代码行为不变。
指纹进文件名后，升级与本地迭代都**不需要重启 explorer**。

### 2.10 内存：三件事一起做才达标（v1.1.2）

**实测先把账算清**（同一台机器，全部用 `GetProcessMemoryInfo` 读 WorkingSet/PrivateUsage）：

| 场景 | 工作集 | 私有 |
|---|---|---|
| 只建 `App` + 采样 + 着色，**不建窗口** | 11.3 MB | 2.4 MB |
| 只建窗口、不绘制 | 10.6 MB | 2.1 MB |
| 建渲染上下文后立即丢弃 | 10.6 MB | 2.1 MB |
| 真实绘制一帧后 | 92.7 MB | 65.9 MB |

结论：**窗口、采样、注入协调都不花钱，钱全在「第一次真正绘制」**。
`ID2D1HwndRenderTarget` 一创建就加载 `d3d11.dll` + `dxgi.dll`，把整条 D3D 设备栈拉进进程。

**三项改动，缺一不可**：

1. **渲染上下文惰性创建**（`window.rs`：`ctx: Option<RenderCtx>`）：首次 `WM_PAINT` 才建。
   用户从不打开设置页 → 永远不付这笔钱。
2. **隐藏即释放**（`hide_to_tray`）：置 `None`，从托盘恢复时重建。实测反复开关 10 轮不累积。
3. **设置页改软件渲染**（`D2D1_RENDER_TARGET_TYPE_SOFTWARE`）：界面是静态页，硬件光栅无收益，
   却要多占约 20 MB。真机截图确认文字/控件/分组卡片渲染正常（`scratch/ui_soft.bmp`）。
   注意：软件模式**仍会加载** `dxgi.dll`（D2D 走 DXGI 呈现到 HWND），只是不再用硬件光栅化。

**第 4 步是账面修饰，要如实理解**：`hide_to_tray` 末尾调
`SetProcessWorkingSetSize(-1,-1)` 修剪工作集，把 23.7 MB 打到 1.8 MB。
**它不减少真实资源**，只把不活跃页交还系统、让任务管理器读数不虚高；
采样线程 1 s 后会把用到的页换回来。真正省下来的来自第 1~3 步（92.7 → 23.7）。
之所以还是做了：NFR-02 的验收口径就是任务管理器里的工作集。

**残留的已知现象**：反复「显示→隐藏」时私有内存缓慢爬升（10 轮 4.99 → 7.72 MB，后期趋平）。
不是明显泄漏，疑似 DWrite 字形缓存/堆碎片，**需要 1 小时长跑才能定性**，已列为观察项。

### 2.11 CPU：`GetPixel` 是元凶（v1.1.2）

**先把归属搞清楚**：v1.1.1 实测 1.18%，v1.1.2（内存改造后）1.10% —— 几乎一样，
说明 CPU 超标**不是内存优化引入的**，是 v1.1 一直存在、此前从未复测的问题。

**定位过程**（每一步都用整数数据否掉一个假设）：
- 暂停自适应（`S-01` 关）后 CPU 没降 → 不是引擎采样线程；
- 线程级 `GetThreadTimes` 显示**全部 CPU 集中在 1 个线程**（3 个线程里 1 个吃掉 1.2%）；
- 读代码发现 `WM_SHADEBAR_DIAG`（1 Hz）做两件重活：重建整棵视图 + 无条件重建托盘图标；
- 更关键的是 `recolor::sample_regions` 用 `GetPixel` 逐点读屏幕：3 段 × 3 行 × 每段 96 点 ≈ 864 次/拍。

`GetPixel` 在 DWM 合成画面上每次都要读回 GPU 表面，单次开销是 μs 级的几倍，
864 次 × 2 Hz 就是那 1%。

**三项改动**：
1. **采样改 `BitBlt` + 内存取值**（`RegionSampler`）：一次抓任务栏条带到复用 DIB（60 px 高，
   覆盖 3/5/7 三行加 S-22 最大偏移），再按原步长从 BGRA 里取点。
   **必须从屏幕 DC 抓**（`grab_screen_region`），不能抓窗口 DC —— 窗口 DC 拿到的是
   `WM_PAINT` 的像素，不是 TranslucentTB 模糊后的合成结果，v1.0.0 因此踩过判定反转。
2. **托盘图标按外观缓存**（`TrayIcon::last_look`）：颜色/色点/降级态都没变就不碰 `Shell_NotifyIcon`。
3. **隐藏时跳过视图重建**：`IsWindowVisible` 为假就不 `sync_view`；重新显示时 `paint` 会自己同步。

**结果**：1.10% → **0.052%**（180 s 平均），页错误 370/s → 160/s。
改色正确性用两种独立方式复核：任务栏截图（深色壁纸下时钟/日期为浅色、托盘图标与指示条正常），
以及用独立脚本 `GetPixel` 读同一批采样点，三段亮度 0.067 / 0.534 / 0.000 均 < 0.55 阈值，与判定一致。

---

## 3. 未完成的待办

### 3.1 优先级高

1. ~~内存偏高~~ **已完成（v1.1.2）**。实测比记录更严重：冷启动 92.7 MB 工作集 / 65.9 MB 私有，
   收托盘后不降。根因是 `ID2D1HwndRenderTarget` 拉起整条 D3D11/DXGI 设备栈且从不释放。
   三项改动叠加后：收托盘 **1.8 MB / 4.9 MB**，显示时 30 MB / 11.6 MB。做法与本轮实测数据见 §2.10。
2. ~~CPU 占用未复测~~ **已完成（v1.1.2）**。实测 1.10%（NFR-03 要求 ≤0.1%，超 11 倍），
   且 v1.1.1 同样 1.18% —— **与内存改动无关，是 v1.1 一直存在、此前从未复测的问题**。
   根因是 recolor 的分区采样用 `GetPixel` 逐点读屏幕（每拍约 864 次，`GetPixel` 在合成画面上要读回 GPU 表面）
   + 托盘图标 1 Hz 无条件重建 + 隐藏时仍重建整棵视图。三项修完降到 **0.052%**。做法见 §2.11。

### 3.2 已完成（原"用户已确认要做"）

3. ~~指示条选中态自适应改色~~ **已完成（v1.1.3）**：改画刷 `Color` 穿透 VisualState，
   做法与数据见 §2.4。

### 3.3 中等

4. **悬停高亮块**（`BackgroundElement` / `BackgroundBorder`）：探针阶段验证过 `Background`
   写入生效（`parent_exp.bmp` 图标底色变青），但**未做稳定性验证**，故未开放。
   若要做：先验证 60 s 稳定，再决定是否恢复 S-32 原语义 / S-36/S-37。
5. **多显示器**：目前只处理含任务栏的主屏（`Shell_TrayWnd`）；副屏是 `Shell_SecondaryTrayWnd`。
6. **深色壁纸真机证据**：当前机器壁纸偏浅，未在真实深色壁纸下拍过"输出浅色字"的对照图。
   可用 S-10 强制端色间接验证，但缺一张真实场景证据。

### 3.4 文档（不一致但已标注）

7. `docs/01-requirements.md` 已加 v1.1 修订块，但全文仍有大量 v1.0.0 时代的
   "L2/L3 移出""A-29 证伪"表述。**不影响使用**，做开源门面时需通读重写。
8. `docs/02-architecture.md` 未包含注入层（TAP / IPC / 元素映射）的架构描述。

### 3.5 明确不做

- 第三方托盘位图图标染色（会丢图标辨识度）。
- 改壁纸 / 重启 explorer 作为常规手段。
- ~~指示条选中态改色~~ —— 已实现（v1.1.3，见 §2.4），不再属于"不做"清单。

### 3.6 已排查结论（v1.1.2 期间）

1. ~~S-03「启动时最小化到托盘」没生效~~ **已排除，不是缺陷（2026-11-18 晚）**。
   双向真机验证：`S-03=0` → 启动弹出窗口；`S-03=1` → 启动隐藏，连续 6 次全部一致。
   早前「窗口有时可见、有时不可见」的观测是**测试环境被残留实例污染**：
   一个旧进程占着 `Local\ShadeBar.SingleInstance` 互斥体，新进程走 `activate_existing`
   后立即退出（表现：`exitcode=0`、`shadebar.log` **零新增行**、进程查不到窗口）。
   当时配置也被「恢复默认设置」重置过（`config.json` 的 `version` 从 120 归 1）。
   **教训**：测启动行为前必须 `taskkill` 所有 `*ShadeBar*` 进程，并确认互斥体已释放，
   否则会得到自相矛盾的结论。
2. **显示/隐藏循环中私有内存缓慢爬升**（10 轮 4.99 → 7.72 MB，后期趋平）。
   需 1 小时长跑定性：是 DWrite 字形缓存收敛，还是真实泄漏。

---

## 4. 其它需记录的内容

### 4.1 踩坑清单（本项目最贵的资产，全部已修）

| 现象 | 真因 | 修法 |
|---|---|---|
| **第二次启动随影后再也不改色，无任何日志** | `ensure_payload_dll` 覆盖写"已被 explorer 映射"的 DLL 失败 → 静默返回 `None` → 服务根本没起来 | 写失败即视为"同名同内容，复用"（文件名已含指纹）。**排查耗时最久的一个** |
| 宿主正常退出后载荷永久失效 | 收到 `shutdown` 直接 `break` 退出循环，DLL 仍驻留 explorer，后续注入命中已加载模块、`DllMain` 不执行 | 改为"还原 + 清零状态 + 继续待命"，不退出、不 `Unadvise` |
| 多个载荷副本互相覆盖 | 同 explorer 内多版本共享同一块 IPC，旧副本用旧逻辑持续覆盖新结果 | 代际令牌 `reserved[1]`：后注入者接管，旧副本静默退位且**不还原** |
| **状态条变透明（用户可见故障）** | `RunningIndicator.Fill` 由视觉状态驱动，`ClearProperty` 后无人重新赋值 → 全透明 | 覆写前缓存原色，退出**写回原色**；无缓存则保持现状 |
| 判定反了（浅底给浅色） | 采样点取在任务栏**上方 8 px 的壁纸**，TranslucentTB 模糊后任务栏显示的颜色更亮 | 改采**任务栏自身**条带（顶边下 3/5/7 px） |
| 状态条全被当成"正常态" | `Width` 实测值是字符串 `"Auto"`，解析失败 | 改用 `ActualWidth`（6 / 16） |
| `E_UNEXPECTED` | 在自建线程调 `AdviseVisualTreeChange` | 移到 `SetSite`（XAML 线程）内 |
| `RPC_E_WRONG_THREAD` | 工作线程读写属性 | `CoreDispatcher::RunAsync` 编组 |
| `E_INVALIDARG`（RunAsync） | 回调未实现 `IAgileObject` | QI 里放行 `IAgileObject` |
| `E_INVALIDARG`（GetPropertyIndex） | 该 API 对任务栏元素不可用 | 从属性链取 `Index` |
| explorer `c0000409` 崩溃 | `GetExitCodeThread` 取 64 位模块基址被截断，远程线程跳野地址 | 改由 DLL 自报基址 |
| 还原被跳过 | `g_uiDoneEvent` 手动重置事件未复位，第二次 `Wait` 立即返回 | `PostToUi` 投递前 `ResetEvent` |
| 强制端色不彻底 | S-10 用"把阈值推到 0/1"实现，加减迟滞带后仍落进区间 | `PolicyCfg.force` 直接返回端色 |

**通用教训**：`SetProperty` 返回 `S_OK` **不代表生效**。唯一可信的验收方式是**截图看像素**。

### 4.2 关键技术参数（改代码前必看）

| 项 | 值 |
|---|---|
| 时间元素 | `TextBlock name=TimeInnerTextBlock` |
| 日期元素 | `TextBlock name=DateInnerTextBlock` |
| 托盘字形 | `SystemTray.AdaptiveTextBlock`，每个图标三层 `Underlay`/`Base`/`AccentOverlay` |
| 状态条 | `Windows.UI.Xaml.Shapes.Rectangle`，**Name** 是 `RunningIndicator`（按 Type 匹配会全漏） |
| 状态条尺寸 | `Height=3`、`RadiusX/Y=1.5`、正常 `ActualWidth=6`、选中 `16` |
| 原始状态条色 | 正常 `#72000000`、选中 `#FF0078D4`（系统强调色，随主题变） |
| 共享内存 | `Local\ShadeBar.Ipc.v1`，80 B，`reserved[0]`=元素掩码，`reserved[1]`=代际 |
| 元素掩码 | bit0 时间 / bit1 日期 / bit2 托盘 / bit3 指示条；未置位 = 写回原色 |
| 端色默认 | 暗背景 `#F2F2F2`、亮背景 `#1A1A1A`；阈值 0.55、迟滞 0.06、EWMA 0.3 |
| 状态条正常态 alpha | `0x72`（与系统一致） |
| XAML 树规模 | 约 285 个元素，其中目标元素约 26 个 |

### 4.3 环境与命令

```bash
# 构建（顺序不能反：宿主编译期要内嵌载荷）
cd payload && bash build.sh && cd ..
cargo build --release

# 测试
cargo test --release          # 214 项

# 打包
cp target/x86_64-pc-windows-msvc/release/shadebar-app.exe "dist/随影ShadeBar_v1.1.2.exe"

# 推送
git push origin main
```

- Rust 1.99+（`rust-toolchain.toml` 锁定）、VS2022 Build Tools（C++ 工作负载）、Win11 SDK。
- 载荷用 `payload/build.sh` 手工拼 `cl.exe` 环境（不依赖 vcvars，避开 `reg.exe` 被安全策略拦）。
- 开发机：Win11 26H2 Build 26300.9711，单显示器 2560×1440，TranslucentTB + Wallpaper Engine 常驻，
  **火绒实时防护开启**（目前未拦注入，但被拦时 `payload.log` 会记 `远程 LoadLibraryW 失败`）。

### 4.4 用户偏好（长期强制）

- **截图必须先征求同意**，同意有时效、过期需重新申请。（本项目开发期已获授权，交付后失效）
- **安装软件必须先问，且由用户选路径**。
- 版本号严格按 SemVer 2.0.0，判定结果写进提交信息。
- 只用简体中文交流。
- 交付物默认只打包可直接运行的 exe，文件名带版本号（`随影ShadeBar_v1.1.2.exe`）；
  只有明确要求才生成安装包。

### 4.5 本项目验证有效的开发方法

1. **先证最高风险点**：动手写产品前，用一次性探针把"注入能不能改色"跑通，再谈架构。
   探针留在 `scratch/tap_probe/`（不进产品、不进仓库）。
2. **验证必须看像素**：日志里的"成功"可能是假的。
3. **改完立刻归档证据**：`evidence/recolor-spike/` 的截图是后续判断"是否回归"的基线。
4. **一个已知坑要顺手写进文档**：`docs/03` §5 与本文件 §4.1 是同一份清单的两个入口。

---

## 5. 新对话续接建议

### 5.1 冒烟验证（3 分钟）

```powershell
# 1) 确认 explorer 加载的是当前版本载荷
(Get-Process explorer).Modules | ? ModuleName -like '*ShadeBar*'

# 2) 启动
dist\随影ShadeBar_v1.1.2.exe

# 3) 日志应有：[Init] 代际=N → [Apply] seq=2 mask=F time=1 date=1 tray=15 ind=7
Get-Content "$env:LOCALAPPDATA\ShadeBar\payload.log" -Tail 5

# 4) 截图看时间/日期/托盘图标颜色是否跟随背景
# 5) 退出后应见 [Restore] restored=N，任务栏恢复默认
```

### 5.2 不可推翻的结论（别重新论证）

- 元素级改色**可行**，路线是官方诊断接口 + UI 线程改画刷。
- 注入**不会**让 explorer 崩，前提是：订阅失败立即收手、所有 XAML 调用回 UI 线程、
  不跨时点持有句柄。
- 时间/日期/托盘的 `Foreground` 与指示条**正常态**的 `Fill` 都是稳的（60 s 验证）。

**关于选中态（v1.1.3 已解决）**：
`SetProperty` 写 `RunningIndicator.Fill` 确实是死路（返回 `S_OK`，但被 VisualState
插在 Local 层的更高优先级槽压住；每秒重写、改父级 `Foreground`/`Background` 也都无效）。
有效路径是**改 `Fill` 指向的画刷对象自身的 `Color`**，详见 §2.4 与 `docs/03` §5.3。
不要再回到"改元素属性"的思路上来。

### 5.3 下一步建议（按用户排期）

1. 若用户未另行指定：**降内存**。功能已齐，46 MB 对一个常驻托盘工具是明显短板，
   而用户对"尽量小的体积和低系统占用"有明确要求（这也是当初选 Rust 的原因之一）。
2. 若用户提出做**选中态改色**：按 §2.4 的四个方向逐个试；动手前先确认是否接受
   "改系统强调色"的全局影响（那是目前最可能一次成功的路径）。
