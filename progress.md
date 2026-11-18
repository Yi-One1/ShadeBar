# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-15 12:30 (GMT+8)
> **用途**：跨对话续接。新对话看不到此前聊天记录，本文件是唯一的状态依据。
> **状态一句话**：**v1.0.0 已放行待发布**，所有 20 项团队任务 completed；代码、产物、文档均已冻结，只剩 v1.1 候选与 Minor 待办。

---

## 0. 新对话先看这里（三十秒上手）

| 项 | 值 |
|---|---|
| 项目 | 随影 ShadeBar — Windows 11 26H2 任务栏**亮度自适应采样/判定/诊断**工具 |
| 工作目录 | `E:\AI工作空间\随影ShadeBar` |
| 版本 | `1.0.0`（workspace `Cargo.toml`；SemVer 2.0.0） |
| 技术栈 | Rust 2021 / `rust-version = "1.99"`；**只依赖 `windows = "=0.58.0"`** 一个 crate；Direct2D + DirectWrite 自绘；静态 CRT、无 VCRUNTIME140 |
| 双 crate | `shadebar-core`（无窗口/无线程/可单测，平台能力全部 trait 化）、`shadebar-app`（真实 Win32/COM 适配 + UI） |
| 构建（一条命令） | `powershell -ExecutionPolicy Bypass -File build.ps1` |
| 运行 | `dist\随影_ShadeBar_v1.0.0.exe`（普通权限、**UAC = 0**、不改系统目录） |
| 版本控制 | **尚未 `git init`，也未推 GitHub**。`Cargo.toml` 里已写 `repository = "https://github.com/Yi-One1/ShadeBar"` |

**v1.0.0 交付范围 = L1**：采样任务栏区域亮度 → 迟滞判定 → 35 项设置 + 中英双语 + 诊断面板 + 单实例托盘。
**明确不含**：任务栏改色（真机证伪，永久移出）、计划任务自启注册、崩溃恢复、日志轮转。产品如实显示降级横幅与受限角标。

**必读文档（按优先级）**
1. 本文件
2. `README.md`（8 条已知限制 K-01~K-08，尤其 **K-08 不提供改色**）
3. `docs/qa-report.md`（482 行，最终验收与放行结论）
4. `docs/01-requirements.md` **v1.6**（§9 验收矩阵与发布条件、§4.7 放行裁定）
5. `docs/02-architecture.md` **v2.1**（§9.5 验收口径速查、§8.6 三层关系）
6. `docs/spike-report.md`（§1~§11 全部技术验证证据）
7. `CHANGELOG.md`、`sp2_manual_report.txt`（用户真机 A-29 报告）

**三条绝对不能踩的口径**
- 不要再宣称"产品会改任务栏颜色"或"会注入/hook explorer"。v1.0.0 全程只读屏幕采样。
- 不要用「单次采样 ≤5ms」判 NFR-03 —— 该口径**已废止**，见 §4.2。
- 不要动 `docs/02-architecture.md` §9.5 / §9 验收矩阵（冻结区）；也不要改 A-02/A-13/A-14/A-20/EX-24/NFR-03/EX-04/D-04 的判据数值。

---

## 1. 已完成事项

### 1.1 产品与架构
- 需求基线 `docs/01-requirements.md` **v1.6**（624 行）：FR 38（L1 19 / L2 8 / L3 6 / L4 5）、设置 35、NFR 19、边界场景 28、验收 48。含 CR-01~CR-06 与勘误 E-01。
- 系统架构 `docs/02-architecture.md` **v2.1**（2160 行）：任务分解、文件清单、调用序列、共享知识 §11、风险台账 §13。
- 两份文档口径一致，§9.5 为冻结的验收口径速查表。

### 1.2 技术验证（三个 Spike）
| 项 | 结论 |
|---|---|
| **SP-1**（Mica + D2D 共存） | 定案 **B3**：保留 DWM 标准标题栏、**不接管 `WM_NCCALCSIZE`**（自绘标题栏会把画在 frame 上的 Mica 盖掉）；`ID2D1HwndRenderTarget` + `D2D1_ALPHA_MODE_IGNORE`；处理 `D2DERR_RECREATE_TARGET`（GPU 复位 / 显示器切换 / RDP 断连会白屏）。 |
| **SP-2**（改色可行性） | 定位链打通：`symprobe` 从 msdl 取 PDB、枚举 33,631 符号，双算法互证定位 `DateTimeIconContent::OnApplyTemplate` RVA `0x2F28C`；偏移表 `data/symbols/26300.9711.json` 三重校验（build + TimeDateStamp + SizeOfImage）。**但最终改色在真机证伪，见 §2.2**。 |
| **A-29**（用户真机一键脚本） | `tools/manual-sp2-verify.ps1` 由**用户亲自执行完毕**，报告 `sp2_manual_report.txt`。Phase A 叶函数近跳转探针 31s `EXPLORER_DIED`；Phase B `FAIL (maxRed=0)`（HookFired ✅ / colorCount=1467 / t≈22.3s explorer 死亡）；基线红像素 28→28 无残留。**不需要再跑**。 |

### 1.3 代码实现（20 条团队任务全部 completed）
| 阶段 | 内容 |
|---|---|
| T01 | 构建骨架 + 六道门禁 `build.ps1`（编码护栏 / 体积 / 无 VCRUNTIME140 / FileVersion / PerMonitorV2+manifest / 图标） |
| T02 | `crates/shadebar-core/src/engine/` 7 文件：任务栏几何、抓屏采样、UIA 遮罩、直方图 P50、EWMA 迟滞判定、600s 闭环状态机。**10 类平台能力 trait 化**，纯逻辑可单测 |
| T03.1 | `shadebar-app` 渲染骨架（B3 落地），`HwndRenderTarget` |
| T03.2~T03.6 | 35 项设置（6 分组 + 诊断页共 7 卡片）、中英双语 142 key 内嵌 `include_str!`、配置 JSON 原子落盘 + 损坏 `.bak` 回退、降级项置灰但保留用户值 |
| T04 | `crates/shadebar-app/src/engine/` 8 文件（代码实测 **3803 行**；早期汇报的 3944 为转述误差，以 `wc -l` 为准）：`FindWindowW`/`SHAppBarMessage`、`BitBlt`+DIB 复用、`IUIAutomation2` 独立 MTA 线程 + 350ms 看门狗、`CreateWaitableTimerW` 驱动闭环、零依赖 PNG 编码器、诊断/纯色窗口 |
| T04.1 | 单实例（`FindWindowW` + `WM_APP+1`，替代架构原定的命名管道）+ 托盘；`WM_CLOSE` 按 S-41 最小化到托盘 |
| 收尾 | `README.md` / `LICENSE` / `CHANGELOG.md`；`cfg(feature="verify")` 隔离取证编排；正式版体积压回 400 KB 内 |

**源码规模**：39 个 `.rs` / **16627 行**。

### 1.4 测试与验收（QA 严过关，独立验证）
- `cargo test --workspace` **四档**：默认 208 / verify 231 / no-en 208 / all-features 231 = **878 次执行、0 failed**；文档测试 2 通过 / 1 ignored。
- `clippy --workspace --all-targets -- -D warnings` 四档全零告警；`build.ps1` **15/15 门禁全绿**（Gate0 + 三产物各 5 门禁），重建后三产物体积逐字节一致。
- 三产物 PE 独立核对 **3/3 PASS**（手写解析器，无第三方库）：FileVersion 1.0.0.0、`asInvoker`+`PerMonitorV2`、五帧图标含 32×32、无 VCRUNTIME140/MSVCP140、`UI_Xaml` 0 命中、`dist/` 根唯一 exe。
- CLI **12/12 PASS**：正式版 `--version`/`--help` exit 0，`--verify` 返回 `EXIT_USAGE(2)`（分支确未编译）。
- A-01 单实例真机受控实测 PASS（第二次启动 0.026s 退出、进程数恒为 1、零残留）。
- 运行时禁用项扫描（剔除 3193 行 `#[cfg(test)]`）：TODO/FIXME/panic!/unwrap()/expect(/unimplemented!/todo!/unreachable!/UI_Xaml **全部 0 命中**。
- **路由判定 `NoOne`**（无源码 Bug、无测试代码 Bug，Round 1 一次收敛），**Blocker / Critical = 0**。
- **48 项验收最终分布**：`✅19 通过 ｜ ⚠️9 观察 ｜ 🔜3 后续版本 ｜ ⛔15 范围外 ｜ 👤2 人工验收 = 48`。
- **D-03 通过（放行）**，`docs/qa-report.md` §10 未关闭条件 **0 项**。

### 1.5 交付物与实测数字
| 产物 | 大小 | md5 |
|---|---|---|
| `dist/随影_ShadeBar_v1.0.0.exe`（**验收依据**） | **408064 B / 398.5 KB** | `1213aac5a87a470ee7ec0d8fe4cbe778` |
| `dist/verify/随影_ShadeBar_v1.0.0_verify.exe` | 498688 B | `7a28a7e9cbaf1042f925ed79b0b82059` |
| `dist/compare/随影_ShadeBar_v1.0.0_no-en.exe` | 401920 B | `af4d0effec5aa70f6aecfeed5cc97f3d` |

- 正式版 **< 400 KB 软目标**，硬限 4096 KB；双语增量 **6144 B = 6 KB ≤ 40 KB**（A-26）。
- 真机证据：A-02 采样矩形误差 **0px**、A-20 CPU **0.0312%**、NFR-03 双口径达标、EX-24 真 `BitBlt` 失败不崩且保持上次判定、A-13/A-14 独立 session **2/2 obtained / flips=0**。

---

## 2. 关键决策（含理由，防止被重新发明）

### 2.1 范围与放行
1. **v1.0.0 = L1 核心，不修改任务栏外观**。理由：A-29 真机证伪了两条改色路线；L1 采样/判定/设置/诊断全部实测通过且零 Blocker，是可独立交付的完整产品。
2. **D-03 放行（v1.6 CR-06）**：把 §9 发布条件从「全部 P0 项勾选通过」**改写为**「v1.0.0 P0 门禁 = §9.1 A 组中结果列仍为 `☐` 的 **25 项**全部通过，且无 Blocker/Critical」。**选"修订文字"而非"新增范围例外"** —— 例外条款会被后续逐项援引、边界易被扩大。
   - 新增**三态定义**：`☐` 门禁内 ｜ `范围外` 不再计划 ｜ `后续版本` 仍计划、本版明确不含。
   - **A-04 / A-05 / A-27 标「后续版本」，不标「范围外」**（后者在 v1.5 中专指 A-29 证伪/不再计划，语义不同），不计入门禁、不算通过也不算失败、**QA 不得记为失败**。
   - D-01~D-05 属发布前总检查，单独核对，不计入「P0 需求项」统计。
3. **偏差⑥四项归属**：T04.2 计划任务自启 / T04.3 崩溃恢复 / T04.5 日志轮转 → **v1.1 候选补齐**；**T04.4 注入编排框架 → ⛔ 永久移出、v1.1 也不补**（随 A-29 证伪，与 T05 一致）。架构 §8.7 依赖图已加「图上的边只表示技术依赖、不表示待办」的读图须知。

### 2.2 改色功能：真机证伪，永久移出
- **结论措辞纪律（重要）**：被证伪的只是「把颜色改上去」的两条实现路线（近跳转叶函数 / `OnApplyTemplate` 期间走查改色）。**定位链（symprobe→RVA→双算法互证）、14 字节绝对跳转 hook 结构（10/10 存活）、偏移表三重校验、TTB 零 patch 检测、起点检测工具链、winmd 槽位解析、手写 WinRT ABI 全部仍然成立**，不得把整段技术积累写成失败。
- 附录 C 的 C.4 已更正：**Phase A 探测的是叶函数近跳转 `0x170DD4`，14 字节绝对跳转结构并未被 A-29 证伪**。
- **近跳转 `E9` fail-fast 是真机缺陷、非 AI 沙箱特异**（架构 §13.2 风险项 R10）。故障码 `0xC0000409`（fail-fast 家族，内核级不可拦截）—— 这也解释了 VEH 全程静默。
- `OnApplyTemplate` 只在任务栏构建时执行（启动期实测触发 1467 次、运行期静默）；期间走查 XAML 会重入破坏原函数栈。
- **元素引用必须「当次有效当次使用」**（架构 §11.10）：1467 个启动期瞬态实例大多已析构，跨时点持有指针 QI = 堆破坏。
- TranslucentTB 共存问题经 `--memdiff` 证实为**行为级冲突**（ExplorerTAP 对 `SystemTray.dll` **零代码 patch**），不是补丁碰撞；v1.0.0 不碰该进程空间，天然共存。

### 2.3 判据口径（全部已冻结）
4. **NFR-03 双口径（v1.4）**，三项须同时满足：
   - 主判据：10 分钟平均**单核 CPU < 0.1%**（`GetProcessTimes` Kernel+User ÷ 墙钟）；实测 **0.0312%** / 全机 0.0026%。
   - 辅助（墙钟）：`cost_us` **p95 ≤10ms**（6.725）、**max ≤15ms**（11.721）、样本 ≥500（592）。范围 `sampler.rs:835` 的 `Instant::elapsed()`，含真 BitBlt+直方图+判定、不含 UIA/几何。
   - `cost_us` 含 **DWM 合成/抓屏阻塞等待**，不作 CPU 性能指标（交叉核算：10min 累计 2.6912s vs 进程 CPU 0.1875s，差 2.5037s 为阻塞）。「CPU 达标不得豁免墙钟超线 / 墙钟超线不得反推 CPU 不达标」。
5. **EX-04 勘误 E-01**：**不统一数值、统一口径**。0.5% 保留但标注为「**瞬态峰值上限**」（≤1s 窗口、≤100ms 间隔、取峰值不取平均），与 NFR-03 的 10 分钟平均**分别测量分别打勾**；双向禁止豁免；QA 无法可靠复现时降级为观察项。
6. **A-13/A-14 稳态段编排获 PM 认可**：判据数值一字未动（仍 60s 翻转 0 次）。第一次独立重跑 1/2（A-14 `flips=1` 且不在段首），真因是 `Judge::new()` 冷启动 `ewma_y=0` + 旧编排为取几何先 `tick()` 一拍采到真实桌面暗色 `#01141B`，导致 EWMA 收敛翻转。修法：`read_probe()` + `TaskbarGeom::from_probe()` 只读几何、先建覆盖窗口，让**第一拍就采到目标背景**（`pre_loop_ticks=0`）。**稳态拍数看 `steady_sample_ticks`(=59)，不是 `steady_publishes`(=0)** —— 后者因静态纯色背景 `change_threshold=8` 属正常，不得据此判不通过。
7. **S-35/S-37 默认值裁定 `#38FFFFFF`**（WinUI `#AARRGGBB`，alpha `0x38`≈22% 半透明白），不采用旧表 `#FFFFFF`。理由：指示条/悬停块是装饰性覆盖层而非文字，不透明纯白会遮挡壁纸、违背 Fluent/Mica 观感。**工程师实现本来就是这个值，无需改代码**。取值范围含 alpha、禁用 `00` 全透明。
8. **体积收口**：隔离取证代码后正式版反而涨（新增 35 项 UI 超过取证代码节省）。对照实验定位真大头是 `engine/uia.rs` 约 **76 KB**（独立 MTA 线程 + COM vtable）；依据 K-05（26H2 UIA 枚举 30 后代命中 0）把 UIA 适配器也门到 `verify`，正式版用 `FixedFallbackClockRectSource`，**不假装 UIA 可用、真机行为不变**。

### 2.4 工程原则
9. **证据分级**：单测通过 ≠ 功能可用；API 返回成功 + 属性回读正确 ≠ 效果生效；**逻辑证据 ≠ 真机效果证据**。T02 的 96 单测全是假几何/假像素，直到 T04 接线补回 A-02/A-13/A-14/A-20/EX-24 真机像素与日志证据才算闭合。
10. **替换等价实现后必须用同一组采样点做逐字节回归**，不得凭目测。基准点集（浅色主题）：`caption #DADADB` / `client bg #F3F3F3` / `card fill #FFFFFF` / `left margin #D5AA7E` / `page heading #F3F3F3`。
11. **实现与文档冲突的处理**：先判「文档意图更合理」还是「实测更可靠」—— 意图合理则**改实现**（DCRenderTarget→HwndRenderTarget 那次），实测推翻则**改文档**（自绘标题栏那条）。**不得简化为"永远是实测对"**。
12. **崩溃归因四步、禁止跳步**：`--memdiff` 排除代码修改 → 引用生命周期 → 第三方事件/时机竞争 → 沙箱收割。「TTB 在场」不是有效归因。
13. **跨成员信息必须经主理人中转**（团队模式下）。多次因直连造成「已撤回结论被写成硬判决」「文档口径与转达打架」。
14. **Q-1 本次裁定不修**：`resources/app.manifest` L6–7 XML 注释残留「注册计划任务由 `ShellExecuteExW(runas)` 重新拉起自身」，与 K-01 矛盾、且描述未做的功能。理由：仅注释、功能无影响（源码无 runas 调用、manifest 实为 `asInvoker`），修它会重建三产物、使已 15/15 全绿的门禁与 3/3 PE 结果全部失效，收益不抵风险。转待办。

---

## 3. 未完成的待办

### 3.1 v1.1 候选（计划内、未被证伪，v1.0.0 明确不含）
| 项 | 现状 |
|---|---|
| **T04.2** 计划任务自启注册（FR-05 / A-04、A-05） | S-02 只持久化用户设定、**不注册 `schtasks`**；UAC = 0 不受影响 |
| **T04.3** 崩溃恢复（FR-14 / A-27） | `RegisterApplicationRestart` 约束不变，只是延后 |
| **T04.5** 诊断包日志轮转 | 按天切分已可用，延后的仅是 **7 天轮转策略** |
| **Q-1** manifest 注释清理 | 见 §2.4 第 14 条，仅删改 XML 注释 |

对应验收项 **A-04 / A-05 / A-27 标「后续版本」**，不计入 v1.0.0 门禁。

### 3.2 永久移出（**不要再排期**）
- **T04.4 注入编排框架**、**T05 任务栏着色 / L2 / L3**：A-29 真机证伪，v1.0.0 及后续版本均不再计划（除非重新设计上屏方案）。
- 需求侧相关 FR 条目**保留但标范围外**（保留溯源），共 15 项验收（B-01~B-11 + C-01~C-04）标「范围外」。

### 3.3 Minor / 证据瑕疵（均不阻塞发布，QA 报告 §9）
- **Q-2**：`evidence/t04/A-20-real-screen.png` 与 `EX-24-real-screen.png` SHA-256 完全相同，A-20 无独立截图；判据以 600s CPU 日志为准，截图仅辅助。
- **Q-3**：`evidence/t04-smoke/t04-summary.md` 引用不存在的 `t04-fast4/` 路径，三张 panel 图为同一张全黑图而汇总表标「是 ok」（该档为 fast smoke，已明示不作验收依据）。
- **Q-4**：`build.rs` 9 处构建期 `panic!`（已声明白名单、不进产物）、`error.rs:321` 测试构造硬编码路径 `C:\Program Files\ShadeBar`；运行时代码 0 命中。
- **偏差⑤**：诊断包生成 stored ZIP（≤2 MB）但不自动打开所在文件夹（当初为避免新起 explorer 触碰用户环境）。
- **EX-04 短窗口峰值**（≤1s 窗口 / ≤100ms 间隔）当前**无采样实现**（取证档只有 fast/full/endpoints）。若要做需新增采样能力 + 可控黑白交替；注意环境纪律（不改壁纸、不动用户窗口）。已按 E-01 降级观察项。
- **EX-26 口径**：双语资源缺失实际回退 **zh-CN，再缺失显示 key**（不是回退 English），按权威需求判通过。

### 3.4 9 项观察项（长时/真机未独立复现，均有单测或静态覆盖，0 失败）
A-03（动态壁纸 10min + 20% 污染真机）、A-10（强杀重启）、A-11（只读目录）、A-15（干净虚拟机）、A-19（30min 工作集）、A-21（5 次冷启动）、A-22（1h 出站监控；源码/导入表无网络栈依赖）、A-23（系统语言真机首启）、A-28（极端污染 60s）。**不要包装成通过**。

### 3.5 人工验收项 2 项
`D-01`、`D-02`（§9 声明，需用户/环境配合）。**A-04 已从人工验收项移除**（归后续版本）、B-06 归范围外。

### 3.6 工程基础设施
- **`git init` + 首次提交 + 推 GitHub**（`Cargo.toml` 已写 `repository = "https://github.com/Yi-One1/ShadeBar"`）。当前所有文件仅本地，无版本控制。
- 可选：v1.0.0 发布打包（zip + 校验值清单）、GitHub Release。

---

## 4. 其它需记录的内容

### 4.1 文件校验值（完整性基准；任何改动都会让 hash 变化）
| 文件 | md5 |
|---|---|
| `dist/随影_ShadeBar_v1.0.0.exe` | `1213aac5a87a470ee7ec0d8fe4cbe778` |
| `README.md`（158 行 / 9919 B） | `62ba39935b8711feeed7b48677864fe8` |
| `CHANGELOG.md`（77 行 / 5462 B） | `54087395dfd9cc5a33a947e3f11fac99` |
| `docs/01-requirements.md` v1.6（624 行 / 93107 B） | `995f53e1309fad7fa009b0ce3c2f0b55` |
| `docs/02-architecture.md` v2.1（2160 行） | `72ec9f7d6f7a0010e537c986ef43e2a4` |
| `docs/qa-report.md`（482 行 / 52565 B） | `5e1d5feb7a244040d7df96a12639d86d` |

### 4.2 已废止 / 不得再引用的口径（防新对话复用）
- ❌「单次采样耗时 ≤5ms」—— NFR-03 口径错误，v1.4 废止，改用双口径（§2.3 第 4 条）。
- ❌「**#B3B3B3 平板灰**」「**破坏 alpha 通道**」—— 两条关于 GDI `AlphaBlend` 的论据**已被撤回**（结论对、理由错），全仓仅在历史溯源处出现，不得作为任何决策依据。真正成立的两条理由：① DWM backdrop 不进客户区；② `WM_ERASEBKGND→TRUE` + 逐帧 AlphaBlend 会让不透明度按 `1 − 0.3ⁿ` 累积。
- ❌「自绘标题栏 / `WM_NCCALCSIZE` 返回 0」—— 会盖掉画在 frame 上的 Mica，已否决。
- ❌「注入需要管理员权限」—— SP-2 实测 medium IL 对 medium IL explorer 可完成注入；但 v1.0.0 根本不注入，权限模型仍是普通权限、UAC=0。
- ❌「SP-2 = 技术成立、证据受限」—— 已被 A-29 真机证伪取代为「真机证伪，改色路线移出」。
- ❌「T02 引擎体积 Δ=0」—— 是 `opt-level=z + lto=fat` 消除未引用代码的**假象**，接线后 +172.5 KB，收口后 398.5 KB。
- ❌「TTB 在场必崩 = 补丁碰撞」—— memdiff 证实零代码 patch，是行为级冲突。
- ❌「AI 沙箱特异导致 fail-fast」—— A-29 真机同样复现，已升级为 R10。

### 4.3 可复用经验（踩过的坑）
- `windows` 0.58 返回值三态混用会编译失败：`GetClientRect`/`GetWindowRect` → `Result<()>`（**不是 `Option`**，写 `is_some()` 直接报错）；`GetDC`/`SelectObject`/`GetDpiForWindow`/`GetWindowLongPtrW` → 裸值（不要加 `?`）；`BitBlt`/`CreateDIBSection` → `Result<T>`；`SHAppBarMessage` → `usize` 位掩码。收口方式：所有 Win32 调用经 `win/mod.rs` 的 `win_*` 包装转 `SbResult<T>`，业务代码不碰 `windows::` 返回类型。详见架构 §11.9。
- `PTIMERAPCROUTINE` 在 `System::Threading` 而非 `UI::WindowsAndMessaging`；`CreateWaitableTimerW` 需 `Win32_Security` feature。
- **publics RVA 常指向函数中间**（obj101 的 `0x170DDC` 实际是 +8，真起点 `0x170DD4`）；打补丁到指令中间必崩。起点检测（CC-padding 回扫 + E8 调用图 + 绝对指针引用扫描）已成必要工序，工具在 `tools/symprobe/`。
- **PSYM_ENUMERATESYMBOLS_CALLBACKW 是三参数原型**（首参 `SYMBOL_INFOW*`），网上流传的四参数老原型会导致枚举崩溃 —— 以本机 SDK `DbgHelp.h` 为准。
- `SystemTray.pdb` 是 publics-only 剥离版，`SymFromAddrW` 恒报 87；改用「全量枚举 + 最近邻匹配」。
- `build.ps1` 在 PS 5.1.26100 下 `Get-ChildItem | Remove-Item -Force` 会报 `ParameterBindingException: InputObjectNotBound`（dist 残留旧 exe 才触发）。已修为 `ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }`，**别改回管道写法**（架构 §11.11）。
- 主程序 `panic = "abort"`，但注入 DLL profile `release-inject` 用 `panic = "unwind"` —— 否则一次 panic 会杀掉 explorer。
- `GlobalAlloc` 分配计数器必须 `thread_local!` + `const { Cell::new(0) }`，否则并行测试互相污染、且避免分配器自身递归。
- UIA 快照 `clear()` 会抹掉 L3 的 `ColoredRects`；已拆 `external_masks` 独立保存、刷新后重新并入。
- 本 AI 执行环境有两个干扰源：① 沙箱在命令会话结束时**收割全部子进程**（无 WER 干净终止，曾把"explorer 消失"误判为崩溃）；② 近跳转匿名页触发 fail-fast 时 VEH 收不到记录。排查时先排除环境。

### 4.4 环境与团队
- 桌面环境：A-29 实验后 explorer / TranslucentTB 均已恢复；最后一次复核为 **explorer pid 36264、TranslucentTB pid 27336**（**pid 会随重启变化，别当常量用**）。`ExplorerTAP.dll` 在 explorer 内属正常共存。
- 全程未改用户壁纸、注册表（仅只读查询 UIA CLSID）、未动用户已打开窗口；取证可视化窗口均为本进程新建 `WS_EX_NOACTIVATE`，用完销毁。
- 团队模式：`software-company` 专家团（主理人 + 产品经理 + 架构师 + 工程师 + QA），共享任务台账 **20/20 completed**。若新对话继续走该模式，注意「成员产出必须由成员输出、跨成员信息经主理人中转」。
- 历史工作日志：`.workbuddy/memory/2026-11-11.md`（前期）、`.workbuddy/memory/2026-11-15.md`（交付总结与经验沉淀）。

### 4.5 用户偏好（长期强制）
- **简体中文**回答。
- **任何截图（窗口/区域/全屏/录屏）必须先询问并取得明确同意**，且同意仅在约定时段内有效，过期需重新询问。未经同意改用读文件/跑测试/读属性等方式验证。
- **安装应用必须先询问并取得同意，安装路径由用户选择**（可推荐）。
- **版本号一律遵循最新 SemVer 2.0.0**：不兼容变更 → MAJOR；向下兼容新功能 → MINOR；向下兼容问题修正（含用户可感知的 bug 修复）→ PATCH。改版本号前先判定并把结论写进提交信息。

---

## 5. 新对话续接建议

1. **先读本文件 + `README.md` + `docs/qa-report.md` §10**，再决定要做什么。不要从旧聊天记录里找结论。
2. **若只是查看成果**：直接运行 `dist\随影_ShadeBar_v1.0.0.exe`；复现构建用 `powershell -ExecutionPolicy Bypass -File build.ps1`。
3. **若要继续开发**：先确认目标是 §3.1 哪一项（建议从 `git init` 与 Q-1 注释清理起步，风险最低）。**不要动冻结的判据数值**，不要在没跑 `build.ps1` 的情况下交付新 exe。
4. **若考虑重启改色研究**：先读 `docs/spike-report.md` §2.6~§2.10 与 `sp2_manual_report.txt`。已有证据表明近跳转与模板内走查两条路在真机均崩；除非有全新上屏方案，否则不建议重开。
5. **A-29 一键脚本已由用户执行完毕**，不要再让用户跑第二次。
6. 每次改代码后按 §4.1 的 hash 表核对哪些文件变了，并同步更新本文件。
