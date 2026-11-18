# Changelog

All notable changes to 随影 ShadeBar are documented here. The version is a single
source of truth in `Cargo.toml` (`[workspace.package] version`), also used for the
artifact filename, the About page and the exe VERSIONINFO (FR-16 / A-16).

## [1.1.0] - 任务栏自适应着色

### Added

- **元素级着色（本版核心）**：TAP 注入 + `IXamlDiagnostics` / `IVisualTreeService2/3`
  在 XAML UI 线程改写画刷，覆盖**时间**、**日期**、**系统托盘字形图标**、
  **运行中指示条（正常态）** 四类元素。元素按 Name 定位，不依赖符号/偏移/版本号。
- **分区独立判定**：时钟区、托盘区、图标区各自采样各自二值化；左深右浅的壁纸
  也能各段选对颜色。时钟/托盘段固定贴住两端，图标区范围可调（S-20/S-21）。
- **共享内存 IPC**：`Local\ShadeBar.Ipc.v1`，80 字节，含心跳、状态、四个颜色与
  元素掩码；宿主被强杀时载荷 7 秒内自动还原并转待命。
- **开机自启**：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，
  零 UAC、零额外文件；启动时按配置校正一次，修掉「开关看着开的其实没生效」。
- **载荷内容指纹**：`build.rs` 计算 DLL 哈希并编进文件名
  （`ShadeBarTap_v1.1.0_<hash>.dll`），内容一变文件名就变，避免 explorer
  已加载同名模块导致静默跑旧代码。

### Changed

- 设置页从 35 项收敛到 **28 项可见**：主题联动（S-19）、采样预览（S-23）、
  悬停块与固定色（S-34~S-37）、注入失败策略（S-44）从 UI 收起，**配置字段保留**，
  旧配置文件可平滑迁移。
- S-32 由「悬停高亮块」改为「系统托盘图标」；S-20/S-21/S-22 语义改为
  「图标区采样起点 / 宽度 / 采样带垂直偏移」。
- S-10「自适应模式」改为直接返回端色（此前用推极端阈值实现，加减迟滞带后强制不彻底）。
- 采样点从「任务栏上方 8 px 的壁纸」改为「任务栏自身条带顶边下 3/5/7 px」：
  前者会被 TranslucentTB 的模糊差异判反。

### Fixed

- **重启随影后再也不改色**：`ensure_payload_dll` 覆盖写被 explorer 映射的 DLL 时
  失败并静默返回 `None`，导致配色服务根本没起来。改为「写失败则复用同名文件」
  （文件名已含内容指纹，同名即同内容）。
- **宿主退出后载荷永久失效**：收到 `shutdown` 时原实现直接退出循环、DLL 却仍驻留
  explorer，后续注入命中已加载模块、`DllMain` 不再执行。改为「还原后继续待命」。
- **状态条变透明**：`RunningIndicator.Fill` 由视觉状态驱动，`ClearProperty` 会让它
  变全透明。改为「覆写前缓存原色、退出写回原色」，拿不到缓存则保持现状。
- **多处载荷副本互相覆盖**：同一 explorer 里驻留多个版本时会用旧逻辑覆盖新结果。
  引入代际令牌（`reserved[1]`），后注入者接管，旧副本静默退位且**不还原**。
- 手动重置事件未复位导致第二次 `Wait` 立即返回、还原被跳过。

### Known issues

- **指示条选中态**不做着色：它在选中/取消选中时被 XAML 视觉状态覆盖，
  `SetProperty` 返回 `S_OK` 但随即失效。与其做无用功，不如完全不碰，
  让系统按主题给出对比度合适的强调色。

## [1.0.0] - L1 主体交付

### Added

- **T03.2~T03.6 设置页**：35 项配置（S-01~S-46）全部可操作，6 个分组卡片 +
  诊断面板；纵向流式布局、滚动视口、长文案省略号 + Tooltip、键盘可达
  （Tab / Space / Enter / 方向键 / PageUp/PageDown）。
- **中英双语**：`resources/lang/*.tsv` 经 `include_str!` 内嵌，文案与 UI 逻辑分离；
  「跟随系统」按主语言 ID 2052 判定，切换即时生效、不重启进程、不中断采样
  （FR-18 / A-23 / A-24 / A-25）。`no-en` 对照构建实测双语增量 6 KB（NFR-19 ≤ 40 KB）。
- **T04.1 单实例 + 托盘外壳**：`Local\ShadeBar.SingleInstance` 互斥体 +
  既有窗口激活；程序生成 16×16 托盘图标（判定色点 S-43、降级受限角标）、
  6 项菜单、左键三选一（S-40）、关闭窗口最小化到托盘（S-41 / A-09）。
- **正式启动编排**：真实 `EngineSession` 采样线程，每拍前同步最新配置快照
  （A-10 同拍生效）；诊断面板 1 Hz 节流刷新，含代表色 / 判定色 / Y / 对比度 /
  最近 10 次折线 / 对比度不足标红 / 采样区过小预警。
- **配置健壮性**：`%LOCALAPPDATA%\ShadeBar\config.json` 原子写入；损坏或越界时
  回退默认值并保留 `.bak`（NFR-16）。
- **诊断包导出**：零依赖 stored ZIP，含脱敏配置、环境信息、日志（S-46 / NFR-10）。
- **`cfg(feature="verify")` 体积收口**：`evidence.rs` / `visual.rs` / `png.rs`、
  UIA COM 适配器与 `main.rs --verify` 分支只在取证档编译；正式发布版不链接。
- **取证编排 `--verify endpoints`**：A-13 / A-14 各起独立 `EngineSession`，
  `flips` 按稳态段统计（排除段首第一拍），判据数值未放宽。
- **NFR-03 双口径报告**：CPU 主判据（`GetProcessTimes`）+ 墙钟辅助判据
  （`cost_us` p95 / max）分别记录，保留 DWM 阻塞等待交叉核算。

### Changed

- `build.ps1` 现在对**三个产物**（正式 / verify / no-en）各跑同一套五道门禁，
  并自动核对 A-26 双语增量与「dist 根目录仅一个 exe」。
- 正式发布版体积由 T04 接线后的 418816 B（409.0 KB，含取证编排）降到
  **408064 B（398.5 KB）**；取证编排版 498688 B 不作体积验收依据。
- `window.rs::WM_CLOSE` 由「直接销毁退出」改为按 S-41 最小化到托盘 / 直接退出。
- `render.rs::paint_skeleton_frame` 保留为 T03.1 B3 上屏链路的对照帧，
  正式重绘入口改为 `ui::View`。

### Scope decision (A-29 / 需求基线 v1.5 §4.6)

- **A-29 真机证伪**：用户真机执行 `tools/manual-sp2-verify.ps1`（报告
  `sp2_manual_report.txt`），Phase A 叶函数近跳转探针（只计数）**31 s 即
  `EXPLORER_DIED`**（近跳转 fail-fast 是真机缺陷、非 AI 沙箱特异）；Phase B
  `OnApplyTemplate` 模板内走查改色 + 像素轮询 **`FAIL (maxRed=0)`**
  （`HookFired ✅ / colorCount=1467 / walk hits=255`，t≈22.3 s explorer 死亡）。
  两条改色实现路线在真实机器上均不可用；基线红像素 **28 → 复原后 28、无残留**，
  explorer 与 TranslucentTB 均已恢复。**定位链 / hook 结构 / 偏移表等技术资产
  仍然成立**，被证伪的只是「把颜色改上去」这两条实现路线。
- **改色模块永久移出**：L2 / L3 任务栏着色能力移出 v1.0.0，**真机证伪后不再计划**，
  除非重新设计上屏方案。v1.0.0 **不注入 explorer、不使用 hook、不修改任何系统进程**，
  全程只读屏幕采样。
- **v1.0.0 交付范围确定为 L1**：真实屏幕采样 + 鲁棒判定 + 平滑防抖 + 35 项设置 +
  双语界面 + 单实例托盘 + 诊断面板 + 真机取证编排（**不注入 explorer、不使用 hook、
  不修改任何系统进程**）。产品计算判定色用于诊断与展示，**不修改任务栏外观**；
  S-30~S-37 元素开关在 v1.0.0 不产生可见改色效果。
- **验收归类（需求基线 v1.5 §9）**：§9 中 **B-01~B-11 + C-01~C-04 共 15 项**按
  「范围外」归类（不算失败也不算通过、不参与 P0 门禁）；**A-01~A-28 保留打勾**；
  **D-04 判据冻结，仍按 §7.1 的 K-01~K-07 共 7 条逐条核对**，K-08 作为发布前观察项。
- **文档同步**：`README.md` 已知限制扩为 K-01~K-08 共 8 条（新增 K-08），
  并校正 K-01~K-04 与其余章节中与交付态矛盾的「着色 / 注入 / L2/L3 / 改色」措辞。

### Known limitations

见 `README.md`「已知限制」**K-01 ~ K-08 共 8 条**（需求基线 v1.5 §7.1）。
其中 **D-04 判据冻结，仍按 K-01 ~ K-07 共 7 条打勾**；**K-08 由 QA 作为发布前
观察项核对**（不勾选 D-04、不计入其通过 / 失败）。

### Not included (L1 范围外)

- T04.2 计划任务自启注册、T04.3 崩溃恢复、T04.4 注入编排框架、T04.5 诊断包轮转：
  本批未做，见 `docs/spike-report.md` §11「明确未做」。
- **T05 注入 DLL / L2 / L3 任务栏着色：A-29 真机证伪后永久移出 v1.0.0，且不再计划**
  （见上方 Scope decision / K-08）。相关 FR 条目保留仅作需求溯源，不参与 v1.0.0 验收。
