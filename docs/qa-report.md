# 随影 ShadeBar v1.0.0（L1）QA 独立验证报告

| 项 | 内容 |
|---|---|
| 报告编号 | SHD-QA-001 |
| 执行人 | 严过关（QA 工程师 / software-qa-engineer） |
| 验证对象 | `dist/随影_ShadeBar_v1.0.0.exe`（正式发布版，验收依据）+ `dist/verify/` 取证版 + `dist/compare/` A-26 对照版 |
| 权威判据 | `docs/01-requirements.md` **v1.4（含勘误 E-01）** §9 验收矩阵 48 项、§7.1 K-01~K-07、§9.1 表后注（主理人冻结口径）；`docs/02-architecture.md` v2.0 §9.5；`docs/spike-report.md` §11。**增量口径**：验证期间 PM 先后定稿 **v1.5（CR-05）** 与 **v1.6（CR-06，D-03 放行书面裁定 + §9 门禁口径修订）**，README/CHANGELOG 同步改版；本报告 §11 做增量核对，**冻结判据数值一律未改** |
| 交付范围 | **L1 = 采样任务栏区域亮度 → 判定 → 35 项设置 + 诊断面板，不改任务栏颜色** |
| 范围外依据 | A-29 真机验证（`sp2_manual_report.txt`）：Phase A `EXPLORER_DIED`、Phase B `FAIL maxRed=0`，两条改色路线在真机均不成立 → **L2/L3 改色相关验收项标「范围外」，不算失败，本次不测改色** |
| 测试轮次 | Round 1 一次完成（无失败测试、无源码 Bug、无测试代码 Bug），未进入 Round 2 |
| 路由判定 | **NoOne**（自动化测试 0 失败 / 门禁全绿 / 无需返工） |
| 放行结论 | **放行**（PM v1.6 §9.4 表后注判定 D-03「通过（放行）」+ 主理人确认；§10 四个放行条件**全部已关闭**） |
| 原始输出 | `evidence/qa/`（`round1-tests.log`、`build-ps1.log`、`pe-audit-final.txt`、`behavior-check.txt`、`single-instance-check.txt`、`disabled-scan.txt`、`test-inventory.txt`、`pe_audit.py` 等） |
| 本版说明 | 按主理人要求**只更正报告、未重跑全量测试**：代码零改动，878 次测试、15/15 门禁、3/3 PE 核对、12/12 CLI、A-01 真机结果继续有效（见 §11.7） |

---

## 1. 摘要

| 指标 | 实测 |
|---|---|
| `cargo test --workspace` 四档 | 默认 **208**、verify **231**、no-en **208**、all-features **231** 全部 `0 failed`；合计 **878 次测试执行 100% 通过** |
| `clippy --workspace --all-targets -D warnings` 四档 | **4/4 退出码 0，零 warning** |
| `build.ps1` | **BUILD OK**，退出码 0；Gate0 编码护栏（12 个 .ps1）+ 三产物各 5 道门禁 = **15/15 全绿**；A-26 断言通过 |
| 三产物 PE 独立核对 | **3/3 PASS**（版本/manifest/图标/CRT/体积/A-26/dist 根唯一 exe） |
| CLI 行为 | **12/12 PASS**（正式版 `--verify` = `EXIT_USAGE(2)`，取证版 `--version` 正常） |
| 单实例 A-01 真机 | **PASS**（第 2 次 0.026 s 退出、exit=0、进程数恒为 1、优雅退出零残留） |
| 禁用项扫描 | 运行时代码（剔除 3193 行 `#[cfg(test)]` 后）**0 命中** |
| 真机证据复核 | NFR-03 双口径三项同时满足；A-13/A-14 独立 session `2/2 obtained`、`flips=0`；A-02/EX-24/A-20 截图与日志一致 |
| **48 项验收（最终，按 §6 逐条表为唯一准据）** | **✅ 19 通过 ｜ ⚠️ 9 观察项（非失败）｜ 🔜 3 后续版本（A-04/A-05/A-27）｜ ⛔ 15 范围外（B-01~B-11 + C-01~C-04）｜ 👤 2 人工验收项（D-01、D-02）｜ 合计 48** |
| **v1.6 三态门禁口径** | `☐` 门禁内 **30 项**（A 组 **25 项 P0 门禁内** + D 组 5 项发布前总检查，单独核对）｜`范围外` **15 项**｜`后续版本` **3 项**｜合计 48；**v1.0.0 P0 门禁内 = 25 项** |
| Blocker / Critical | **0** |

> **数字口径说明（§1 / §6 / §9 / §10 四处完全一致，总数 48）**
> - **通过数核准为 19**（上一版 18 的变化来源单一：PM v1.6 §9.4 表后注明确 **D-03 判定 = 通过（放行）**，D-03 由 ⚠️ 升 ✅，故 ✅ 18→19、⚠️ 10→9）。B-11 仍按 v1.5 §9.2 归 **范围外**，未回到 ✅，故范围外仍是 **15 项**（不是 14 项）；本版只保留一套定稿数字，不再并列任何历史对比口径。
> - **A-04 / A-05 / A-27 由「未达成」改标「后续版本」**：PM v1.6 §4.7 书面裁定三项为 **v1.1 候选、仍计划、v1.0.0 明确不含**，**不计入 v1.0.0 P0 门禁、不勾选、不算通过也不算失败、QA 不得记为失败**；「范围外」在 v1.5 中专指 A-29 真机证伪 / 不再计划，语义不同、不得混用。本报告已按裁定移出 ❌ 分类。
> - **人工验收项 = 2 项（D-01、D-02）**：v1.6 §9 开头已把 **A-04 从人工验收项移除**（归入「后续版本」），两项均不再与 👤 重复计数；§9 旧文中造成重复计数的表述已按三态定义重新表述。
> - **⚠️ 9 项观察项是证据粒度标注**（长时/真机类未独立复现，均有单测/静态覆盖、0 失败），**不是失败、不阻断放行**；与 PM v1.6「门禁内 L1 核心全部实测通过、D-03 成立」的裁定不冲突。

**环境纪律执行结果**：全程未注入 explorer、未重启 explorer、未动 TranslucentTB、未改注册表/壁纸、未动用户已打开窗口。验证前后 `explorer pid=36264`、`TranslucentTB pid=27336` 均未变化；唯一自建进程（单实例测试）已通过自身 `MENU_EXIT` 优雅退出，结束检查 `ShadeBar leftovers = NONE`。

---

## 2. 实测输出（可复现）

### 2.1 四档 `cargo test --workspace`（`evidence/qa/round1-tests.log`）

| 档位 | app | core | doc | 结果 | EXIT |
|---|---:|---:|---:|---|---:|
| 默认 | 110 | 96 | 2（+1 ignored） | `ok. 110/96/2 passed; 0 failed` | 0 |
| `--features verify` | 133 | 96 | 2（+1 ignored） | `ok. 133/96/2 passed; 0 failed` | 0 |
| `--features no-en` | 110 | 96 | 2（+1 ignored） | `ok. 110/96/2 passed; 0 failed` | 0 |
| `--all-features` | 133 | 96 | 2（+1 ignored） | `ok. 133/96/2 passed; 0 failed` | 0 |

工程师自报「110+96+2=208」与实测一致。四档均 `Finished`，无 `error:` / `warning:` 行。

### 2.2 四档 `clippy --workspace --all-targets -- -D warnings`

四档全部 `EXIT_CODE=0`、`Finished dev profile`，日志中**无一条 warning**。

### 2.3 `powershell -ExecutionPolicy Bypass -File build.ps1`（`evidence/qa/build-ps1.log`）

```
Gate 0: scanned 12 .ps1 file(s) — OK: every non-ASCII script has a UTF-8 BOM
version = 1.0.0    VS: BuildTools 2022    MSVC 14.44.35207    SDK 10.0.26100.0
[skip] crates/shadebar-inject not present yet (arrives in T05); nothing embedded
formal : 398.5 KB (408064 B)   gates 1-5 all OK   icon extractable 32x32
verify : 487.0 KB (498688 B)   gates 1-5 all OK   icon extractable 32x32
no-en  : 392.5 KB (401920 B)   gates 1-5 all OK   icon extractable 32x32
A-26: formal=408064 B  no-en=401920 B  delta=6144 B (6 KB)  OK (<= 40 KB)
BUILD OK (formal + verify + no-en gates all green)      BUILD_PS1_LASTEXITCODE=0
```

重建后我再次独立解析 PE（`evidence/qa/pe-audit-final.txt`）：三产物体积与重建前**完全一致**（408064 / 498688 / 401920 B），门禁属性不变 → 构建可复现。SHA-256 变化仅来自 PE `TimeDateStamp`（formal/verify 重新链接，no-en 走缓存故 hash 不变）。

### 2.4 三产物独立 PE 核对（手写解析器，无第三方库）

| 项 | formal | verify | no-en | 判据 |
|---|---|---|---|---|
| 体积 | 408064 B / 398.5 KB | 498688 B / 487.0 KB | 401920 B / 392.5 KB | NFR-01 以发布版为准：398.5 KB < 400 KB 软目标、≪ 4096 KB 硬限 ✅ |
| `FileVersion` / `ProductVersion` | 1.0.0.0 / 1.0.0.0 | 同左 | 同左 | FR-16 / A-16 ✅ |
| `ProductName` / `OriginalFilename` | 随影 ShadeBar / 随影_ShadeBar_v1.0.0.exe | 同左 | 同左 | ✅ |
| RT_MANIFEST | `asInvoker` + `requestedExecutionLevel` + `PerMonitorV2` + UTF-8 | 同左 | 同左 | NFR-08 / NFR-11 ✅ |
| 图标资源（RT_GROUP_ICON） | 16/24/**32**/48/256，5 个 RT_ICON blob | 同左 | 同左 | FR-06 含 32×32 ✅ |
| 导入表 CRT | 无 VCRUNTIME140 / MSVCP140 / VCRUNTIME140_1 | 同左 | 同左 | FR-15 / A-15 ✅ |
| `UI_Xaml` 字符串 | 0 命中 | 0 命中 | 0 命中 | 架构禁令 ✅ |
| A-26 增量 | `408064 − 401920 = 6144 B = 6 KB ≤ 40 KB` | — | — | NFR-19 ✅ |
| `dist/` 根目录 | 仅 `随影_ShadeBar_v1.0.0.exe` 1 个 exe，无 `lang\` 侧车 | — | — | A-26 ✅ |

导入表（正式版）：`api-ms-win-core-synch-l1-2-0.dll, kernel32.dll, user32.dll, gdi32.dll, d2d1.dll, dwrite.dll, dwmapi.dll, oleaut32.dll, ntdll.dll, advapi32.dll, shell32.dll` —— 无网络 DLL、无 CRT 运行库 DLL。

### 2.5 CLI 行为（`evidence/qa/behavior-check.txt`，12/12 PASS）

| 用例 | 期望 | 实测 |
|---|---|---|
| formal `--version` / `-V` | exit 0 | `随影 ShadeBar v1.0.0`，exit 0 ✅ |
| formal `--help` / `-h` / `/?` | exit 0 | 用法全文，exit 0 ✅ |
| **formal `--verify fast`** | **EXIT_USAGE(2)**（分支未编译） | **exit 2，无输出** ✅ |
| formal 未知参数 | exit 2 | exit 2 ✅ |
| verify `--version` / `--help` | exit 0 | 正常输出，exit 0 ✅ |
| verify `--verify` 非法/缺失 profile | exit 2（不启动取证循环） | exit 2 ✅ |
| no-en `--version` | exit 0 | 正常输出，exit 0 ✅ |

全部用例均在 20 s 内退出，**无任何用例创建窗口或启动常驻采样**（`main.rs:76-88` 参数分支已核对）。

### 2.6 单实例 A-01 真机受控验证（`evidence/qa/single-instance-check.txt`）

```
pre-check: no ShadeBar process
1st instance pid=21072, alive=True, shadebar_pids=[21072]
2nd instance: exit=0 elapsed=0.026s timed_out=False, shadebar_pids=[21072]
FindWindowW("ShadeBar.MainWindow") hwnd=1182786
PostMessageW(WM_COMMAND, MENU_EXIT) posted=True
1st exit code=0   remaining ShadeBar pids after cleanup: []
A-01 SINGLE INSTANCE: PASS
```

验证前确认无既有 ShadeBar 进程；第 1 实例由本次自建、走自身托盘退出路径优雅销毁；验证后 `explorer/TranslucentTB` pid 未变、无残留进程。偏差①（激活通道由命名管道改为 `FindWindowW` + `WM_APP+1`）的**行为判据 A-01 已真机证实不变**。

---

## 3. 真机证据复核（不看工程师结论，直接核原始日志）

### 3.1 NFR-03 双口径（`evidence/NFR-03-dual-criteria.md` ↔ 原始 `evidence/t04/A-20-NFR-03-cpu.log`）

| 指标 | 派生表 | 原始 full 档日志 | 一致性 |
|---|---:|---:|---|
| `wall_ms` | 600184 | 600184 | ✅ |
| `cpu_delta_100ns` | 1875000 | 1875000 | ✅ |
| `logical_cpus` | 12 | 12 | ✅ |
| `cpu_core_pct` | 0.0312 | 0.0312 | ✅ |
| `cpu_machine_pct` | 0.0026 | 0.0026 | ✅ |
| `samples` | 592 | 592 | ✅ |
| `cost_avg_us` / `p95` / `max` | 4546 / 6725 / 11721 | 4546 / 6725 / 11721 | ✅ |
| `bad_frames` / `flips` | 0 / 0 | 0 / 0 | ✅ |

**三项判据同时满足**：单核 CPU `0.0312% < 0.1%`；`cost_us` p95 `6.725 ms ≤ 10 ms`；max `11.721 ms ≤ 15 ms`；样本 `592 ≥ 500`。
交叉核算自洽：`592 × 4546 µs = 2 691 232 µs ≈ 2.6912 s` 墙钟 vs `1 875 000 × 100 ns = 0.1875 s` 进程 CPU，差 `2.5037 s`（≈93%）为 DWM 抓屏阻塞等待，与「`cost_us` 不作 CPU 指标」的口径一致。
`evidence/NFR-03-dual-criteria.md` 已明确声明「**从 T04 full 档 600 s 日志派生，不是新一轮 10 分钟测量**」，本批 `endpoints` 档 `cpu_ms=0`（`t04-rerun/t04-main.log` 第 1 行）未伪造新 CPU 值 —— **派生关系如实，无包装**。
原始日志仍保留 `nfr03_budget_met=false` / `result_detail: single-sample budget NOT met`（按已废止的 5 ms 口径，241/592 超线）。需求 v1.4 CR-04 已废止该口径，派生表 §4 明确列为「已废止口径，仅溯源、不作门禁」；**A-20 打勾以 v1.4 双口径为准，同时核对本表与原始日志**，符合要求。

### 3.2 A-13 / A-14 独立重跑（`evidence/t04-rerun/`）

| 判据 | A-13 纯黑 | A-14 纯白 | 要求 |
|---|---|---|---|
| `independent_session` / `pre_loop_ticks` | true / 0 | true / 0 | 各起独立 `EngineSession`、第一拍即采到覆盖背景 ✅ |
| `covered_rgb` / `target_rgb` / `coverage_delta` | (0,0,0) / (0,0,0) / 0 | (255,255,255) / (255,255,255) / 0 | 覆盖生效 ✅ |
| `baseline_center_pixel` | `#00131A` | `#00131A` | 与区域基线对照一致 ✅ |
| `duration_ms` / `samples` | 60900 / 60 | 60859 / 60 | 60 s 各 60 拍 ✅ |
| **`flips` / `flips_steady`** | **0 / 0** | **0 / 0** | **未放宽，60 s 翻转 0 次** ✅ |
| `flips_head_excluded` | false | false | 段首也无翻转 → 按更严格的全段口径通过 ✅ |
| **`steady_sample_ticks`** | **59** | **59** | **稳态拍数以此为准（v1.5 §9.1 注写死）；`steady_publishes=0` 因 `change_threshold=8` 静态纯色只发布 1 次判定，属正常，不得据此判不通过** ✅ |
| `side` / `decision_color` | `Light` / `#FFFFFF` | `Dark` / `#000000` | ✅ |
| 末拍 Y | `0 ≤ 0.05` | `1.000 ≥ 0.95` | ✅ |
| `result` | `obtained=true reason=ok` | 同左 | 2/2 obtained ✅ |

截图 `A-13-black-screen.png`（黑色采样条带 + 红框）/`A-14-white-screen.png`（白色采样条带 + 红框）与日志端点色一致。`UIA-real-probe.log` 记录 `UIA descendants enumerated: 30 / hits total: 0 / fallback=true`，与 K-05 一致。
对照 `evidence/t04/` full 档：A-13/A-14 为 `flips=1`（共用 session 切背景的段首伪影，`spike-report §11.4` 如实登记 1/2 → 根因定位为 `read_probe` 编排后 2/2）；**v1.4 要求的独立 session 重跑已满足，判据数值一字未改**。
> **PM 已正式认可该编排（v1.5 §9.1 注）**，三条理由：① 判据数值一字未动（A-13/A-14 仍要求 60 s 翻转 0 次）；② 修复消除的是**编排产物而非产品抖动**（`flips_head_excluded=false` 证明「排除段首一拍」救不了它，根因是 `core::Judge::new()` 冷启动 `ewma_y=0` + 旧编排为拿几何先 `tick()` 一拍采到真实桌面暗色 `#01141B`，把 EWMA 种子设到暗端，覆盖纯白后爬升穿过迟滞带产生收敛翻转）；③ `evidence/t04-rerun/` 2/2 obtained 且段首无翻转、全段口径同样通过。**第一次 1/2 obtained 记为编排缺陷，不作为判据放宽依据。**

### 3.3 A-02 / EX-24 / A-20 截图与日志

| 证据 | 复核结果 |
|---|---|
| `evidence/t04/A-02-geometry.log` | `found_real_tray=true`、`TrayNotifyWnd rect=(2155,1392)-(2560,1440)`、`sample_rect=(2155,1396)-(2560,1436)`、`contained=true`、`error_px=0 ≤ tolerance_px=4`、`dpi=96`、`auto_hide=false` → **A-02 通过** |
| `evidence/t04/A-02-tray-annotated.png` | 有效：可见真实托盘图标、时钟 `19:14`、日期 `2026/11/11` 及红/蓝采样标注框 → 可作 K-06 的「A-02 主截图」 |
| `evidence/t04/EX-24-bitblt.log` | 构造 `BitBltFailed{code:0}` ×4 + `BadFrameBurst` ×2；`last_decision_kept=true`、`published_delta=0`、`recovered_after_fault=true`、`no crash / no panic` → **BitBlt 失败不崩且保持上次判定通过** |
| `evidence/t04/EX-24-real-screen.png` | 有效：托盘区真实屏幕采样区截图 + 标注框 |
| `evidence/t04/A-20-real-screen.png` | 与 `EX-24-real-screen.png` **SHA-256 完全相同**（`8d48b42f…`），非独立截图；A-20 判据以 600 s CPU 日志为准，截图仅辅助 → 登记为观察项 Q-2 |
| `A-02-geometry.log` / `EX-24-bitblt.log` 均写 `panel screenshot NOT obtained: uniform blank area (window not composited)` | 与 K-06「诊断面板自身截图未取得」**如实一致**，未宣称已取得 |
| `evidence/t04-smoke/*-panel.png`（3 张） | SHA-256 全相同（`f8fc217c…`），画面全黑，正是 K-06 所述 BitBlt 限制；该档 `t04-summary.md` 已标注 A-20「10min CPU evidence NOT obtained」，不作验收依据 |

### 3.4 A-29 与范围外判定

`sp2_manual_report.txt`：Phase A `EXPLORER_DIED`（12:56:29，31 s）、Phase B `FAIL (maxRed=0)`（12:57:09，explorer 31716 消失），Phase C 恢复 TranslucentTB，前后红色像素基线均为 28。架构 v2.1 §12 / §13.3 已据此把 T05 永久移出。
按需求 **v1.6 §9 通过判定 / 三态定义**：**B-01~B-11（11 项）+ C-01~C-04（4 项）= 15 项全部标「范围外」**（能力经证伪 / 移出、不再计划），不算失败也不算通过、不勾选、不参与 P0 门禁；本次不测改色、不判失败。

---

## 4. 边界与错误路径用例清单（逐条核对）

| # | 边界场景 | 验证方式 | 结果 |
|---|---|---|---|
| 1 | 配置 JSON 损坏 `.bak` 回退 | `config.rs:503-517`：`from_json` 返回 `None` → `rename(path, config.json.bak)` → 默认值 + 日志；单测 `corrupt_json_is_rejected_not_panicking` | ✅ |
| 2 | 双语资源缺失回退 | `i18n.rs:99-115`：En 缺 key → `zh-CN` → key 本身；`no-en` 档英文表不编译并回退中文；单测 `both_tables_cover_every_required_key`、`english_flag_matches_feature` | ✅（口径澄清见 §7 Q-5） |
| 3 | 降级项 S-30~S-33/S-35/S-37 置灰但保留用户值不清零；**S-35/S-37 默认值按 v1.5 正常核对 = `#38FFFFFF`** | 单测 `degraded_keeps_user_values_not_zeroed`、`rebuild_syncs_values_and_greys_only_degraded_elements`、`app_defaults_are_l1_and_degraded`、`s35_s37_defaults_have_alpha_0x38`、`color_field_never_allows_fully_transparent` | ✅（口径已统一，见 §11.5） |
| 4 | S-04 跟随系统即时切换、不重启进程、采样不中断 | 单测 `language_switch_is_instant_and_keeps_pid_state`、`apply_actions_persists_and_hot_switches_language`、`language_resolution_values`；`GetUserDefaultUILanguage` 主语言 ID 2052→zh / 其余→English | ✅ |
| 5 | 任务栏自动隐藏 | 单测 `ex06_auto_hide_makes_geometry_unusable`、`auto_hide_geometry_detects_offscreen_bar`、`auto_hide_visible_bar_is_not_flagged`、`engine_auto_hide_suspends_and_keeps_last_decision`；真机 `A-02-geometry.log` `auto_hide=false source=SHAppBarMessage + geometry` | ✅ |
| 6 | 竖向任务栏暂停 + 提示 | 单测 `ex07_vertical_taskbar_is_detected`、`engine_vertical_taskbar_suspends_and_notifies_exactly_once` | ✅ |
| 7 | DPI 切换 ≤2 s 重算 | 单测 `nfr08_dpi_change_is_detected_and_recomputed`、`engine_dpi_change_recomputes_within_two_seconds`、`engine_explicit_dpi_notification_forces_recompute`；manifest PerMonitorV2 已核 | ✅ |
| 8 | BitBlt 失败不崩且保持上次判定 | 真机 `EX-24-bitblt.log`（`last_decision_kept=true`、`published_delta=0`、`no crash`）+ 单测 `bitblt_failure_returns_error_not_panic`、`bitblt_failure_propagates_and_burst_is_detected` | ✅ |
| 9 | 单实例二次启动激活已有窗口 | **真机受控实测**（§2.6）+ `ipc_server.rs` 4 项单测 | ✅ |
| 10 | WM_CLOSE 按 S-41 最小化到托盘 | `window.rs:472-489`（S-41=0 → `hide_to_tray`，=1 → `request_exit`）+ 单测 `close_action_default_keeps_process_alive` | ✅ |
| 11 | 诊断包 stored ZIP ≤2 MB | `app.rs:396-451`：`write_stored_zip` 末尾 `if out.len() > 2*1024*1024` 返回错误；单测 `stored_zip_has_valid_signatures_and_roundtrips_names`（`bytes.len() < 2 MB`）、`crc32_matches_known_vectors`（含 `0xCBF43926` 标准向量）；含配置/环境/日志 3 类 | ✅ |
| 12 | EX-04 瞬态峰值 <0.5% | **降级为观察项**（详见 §7.1），符合勘误 E-01 兜底条款 | ⚠️ |
| 13 | 鲁棒统计（15% 截尾中位数）/ 自反馈污染 | 单测 `a28_character_pollution_does_not_flip_judgement`、`histogram_skips_masked_pixels`、`hysteresis_prevents_threshold_chatter`、`judge_pure_black_and_pure_white_endpoints_do_not_flip`、`wcag_contrast_black_white_is_21` | ✅（真机极端污染未独立复现，见 A-03/A-28） |
| 14 | 配置越界 / 全透明色 / 恢复默认 | 单测 `sanitize_clamps_out_of_range_values`、`color_field_never_allows_fully_transparent`、`restore_defaults_resets_every_value`（A-12 按 v1.5 新默认值 `#38FFFFFF` 校验） | ✅ |
| 15 | 电池模式 / 暂停 / 600 s 闭环 | 单测 `battery_mode_enforces_2000ms_floor`、`engine_pause_freezes_sample_timestamp_and_resumes_in_time`、`engine_600s_closed_loop_meets_a03`、`full_closed_loop_600s_with_fake_clock` | ✅ |

---

## 5. 禁用项扫描（`evidence/qa/disabled-scan.txt`）

扫描范围：`crates/` 下 **39 个 .rs / 16627 行**，其中 **3193 行**位于 `#[cfg(test)]` / `#[test]` 区域内，剔除后统计：

| 模式 | 总命中 | 测试区 | 注释 | **运行时代码** |
|---|---:|---:|---:|---:|
| TODO / FIXME | 0 / 0 | 0 | 0 | **0 / 0** |
| `panic!` | 12 | 3 | 0 | **9（全部在 `build.rs`，构建脚本白名单）** |
| `unwrap()` | 0 | 0 | 0 | **0** |
| `expect(` | 54 | 54 | 0 | **0** |
| `unimplemented!` / `todo!` / `unreachable!` | 0 / 0 / 0 | 0 | 0 | **0 / 0 / 0** |
| `UI_Xaml` | 0 | 0 | 0 | **0** |

**结论：运行时代码 0 命中**。`build.rs` 的 9 处 `panic!` 是编译期失败快速报错（rc.exe / 环境变量 / 版本号解析），不进入发布产物，`spike-report §11.10` 已事先声明为白名单例外；`unwrap_or_else` 用于 Mutex 中毒降级，未被计入。严格按「剔除 cfg(test) 后 0 命中」的字面口径，这两类属已声明例外，登记为观察项 Q-4，不判失败。

另核对 D-05「无测试用硬编码路径」：`crates/shadebar-core/src/error.rs:321` 的 `r"C:\Program Files\ShadeBar"` 位于 `#[cfg(test)] all_variants` 的错误变体构造（仅用于断言 i18n key 覆盖），非运行时路径逻辑、非调试残留；登记为 Minor 说明项。

---

## 6. 48 项验收逐条对照表（**唯一加总准据**）

图例：✅ 通过 ｜ ⚠️ 观察项（证据不足/部分通过，**非失败**）｜ 🔜 后续版本（v1.1 候选，**不计入门禁、不算通过也不算失败**）｜ ⛔ 范围外（不再计划）｜ 👤 人工验收项

### 6.1 A 组（L1，28 项）— ✅16 / ⚠️9 / 🔜3（**其中 25 项为 v1.0.0 P0 门禁内 `☐`**）

| 序号 | 需求 | 结果 | 依据 |
|---|---|:---:|---|
| A-01 | FR-01 单实例 5 次进程恒 1 | ✅ | 真机受控实测：2nd `0.026 s`/exit 0/进程数 1/优雅退出零残留；4 项 `ipc_server` 单测 |
| A-02 | FR-02 采样框误差 ≤4 px | ✅ | `A-02-geometry.log` `error_px=0`、`contained=true`；`A-02-tray-annotated.png` 有效 |
| A-03 | FR-03 10 min 动态壁纸 + 20% 字符污染 | ⚠️ | 真机 full 档 `592/600` 采样、`0 bad`；鲁棒统计/污染由 core 单测（fake clock）覆盖；**Wallpaper Engine 动态壁纸 + 人为缩采样区到时钟文字未独立真机复现**（观察项，非失败） |
| A-04 | FR-05 重启电脑自启 | 🔜 | **PM v1.6 §4.7 裁定归入「后续版本」（v1.1 候选）**：能力仍计划、未被证伪、v1.0.0 明确不含；不勾选、不算通过也不算失败、**QA 不得记为失败**。备查事实：T04.2 未注册 `schtasks`、S-02 仅持久化设定；CHANGELOG `Not included` 已明示 |
| A-05 | FR-05 关闭自启 | 🔜 | 同上，v1.6 §9.1 结果列标「后续版本」 |
| A-06 | FR-06 托盘图标 | ✅ | PE 5 尺寸含 32×32；程序生成 16×16（单测 `icon_size_is_16_pixels`）；A-02 截图可见托盘区 |
| A-07 | FR-07 托盘菜单 6 项 | ✅ | 单测 `six_menu_ids_are_sequential_and_unique`；`handle_menu` 6 分支；真机 `MENU_EXIT` 优雅退出已实测 |
| A-08 | FR-08 720×560 无截断 | ✅ | 单测 `min_window_size_meets_acceptance`；layout 换行/省略号单测 |
| A-09 | FR-08 关闭按钮后进程仍在 | ✅ | `window.rs:472-489` S-41 分支；单测 `close_action_default_keeps_process_alive` |
| A-10 | FR-09 改参数 3 s 生效 + 强杀保持 | ⚠️ | 单测 `set_config_is_reflected_by_next_snapshot`、`json_roundtrip_preserves_all_values`；**真机强杀重启未单独执行**（观察项，非失败） |
| A-11 | FR-10 只读目录回退 `%LOCALAPPDATA%` | ⚠️ | `config.rs:484-500` 路径回退实现完整；**未做 Program Files 真机实测**（避免改用户目录，观察项，非失败） |
| A-12 | FR-11 恢复默认 | ✅ | 单测 `restore_defaults_resets_every_value`、`restore_defaults_is_confirmed_action`；默认值按 v1.5 `#38FFFFFF` 校验 |
| A-13 | FR-12 纯黑 60 s | ✅ | `t04-rerun`：`flips=0`/`flips_steady=0`、Y=0.000≤0.05、`Light/#FFFFFF`、独立 session、`steady_sample_ticks=59`、2/2 obtained |
| A-14 | FR-12 纯白 60 s | ✅ | 同上：Y=1.000≥0.95、`Dark/#000000`、`flips=0`、`steady_sample_ticks=59` |
| A-15 | FR-15 干净虚拟机双击 | ⚠️ | 无虚拟机实测；PE 导入表无 VCRUNTIME140/MSVCP140/VCRUNTIME140_1（静态 CRT）可间接佐证（观察项，非失败） |
| A-16 | FR-16 三处版本一致 | ✅ | 文件名 `v1.0.0` / `--version` 输出 `随影 ShadeBar v1.0.0` / `FileVersion=1.0.0.0`（`OriginalFilename` 亦一致） |
| A-17 | FR-17 暂停/还原 | ✅ | 单测 `engine_pause_freezes_sample_timestamp_and_resumes_in_time`、`pause_and_autostart_toggles_are_idempotent_per_call` |
| A-18 | NFR-01 ≤4096 KB | ✅ | 408064 B = 398.5 KB（发布版）；重建复现一致；<400 KB 软目标 |
| A-19 | NFR-02 30 min 工作集 ≤20 MB | ⚠️ | 无 30 分钟真机工作集记录（观察项，非失败） |
| A-20 | NFR-03 CPU <0.1% | ✅ | **双口径同时核对**：`0.0312% < 0.1%`（GetProcessTimes）、p95 `6.725 ms ≤10 ms`、max `11.721 ms ≤15 ms`、样本 `592 ≥500`；派生表与原始 full 档日志数值逐项一致；双向禁止条款未触发 |
| A-21 | NFR-04 5 次冷启动中位数 ≤1.5 s | ⚠️ | 无 5 次冷启动计时记录（单实例实测 0.026 s 是第 2 次启动，非冷启动；观察项，非失败） |
| A-22 | NFR-12 1 h 无出站连接 | ⚠️ | 无 TCPView 1 h 记录；源码/Cargo features/PE 导入表均无网络栈依赖，静态可证（观察项，非失败） |
| A-23 | FR-18 中/英首启 | ⚠️ | 单测 `language_resolution_values` + `GetUserDefaultUILanguage` 实现；**未改系统 UI 语言做真机首启**（环境纪律，观察项，非失败） |
| A-24 | FR-18 语言热切换 ≤1 s / PID 不变 | ✅ | 单测 `language_switch_is_instant_and_keeps_pid_state`、`apply_actions_persists_and_hot_switches_language` |
| A-25 | FR-18 双语覆盖清单 | ✅ | 单测 `both_tables_cover_every_required_key`、`both_tables_have_identical_key_sets`、`known_texts_resolve_in_both_languages`；142 key 双表一致 |
| A-26 | NFR-19 增量 ≤40 KB + 仅 1 exe | ✅ | `6144 B = 6 KB`；`dist/` 根 1 个 exe；无 `lang\` 侧车；build.ps1 A-26 断言通过 |
| A-27 | FR-14 崩溃恢复 | 🔜 | **PM v1.6 §4.7 裁定归入「后续版本」（v1.1 候选）**：不勾选、不算通过也不算失败、**QA 不得记为失败**。备查事实：T04.3 未做（无 `RegisterApplicationRestart` 注册与重启计数）；CHANGELOG `Not included` 已明示 |
| A-28 | FR-03/EX-28 极端污染 60 s 不翻转 | ⚠️ | 单测 `a28_character_pollution_does_not_flip_judgement` + mask/hysteresis 单测；**真机 60 s 极端污染未独立复现**（观察项，非失败） |

### 6.2 B 组（L2，11 项）— ⛔11

| 序号 | 结果 | 说明 |
|---|:---:|---|
| B-01 ~ B-11 | ⛔ | **v1.5 §9.2 结果列已把 B-01~B-11 全部标为「范围外」**（含 B-06、B-11），不算失败也不算通过、不勾选、不参与 P0 门禁。B-01~B-10 为注入/时间日期着色/explorer 重挂/安全还原/5 种注入异常/元素独立开关/NFR-09/NFR-18；**B-06 原列人工验收项，v1.5 起随 L2 移出，不再同时算人工项**。**QA 实测留档（B-11 的产品约束）**：manifest `asInvoker` ✅、源码无 `runas`/提权入口 ✅、README K-01 ✅、UAC=0，约束由 NFR-11 / K-01 承接；L1 交付态「受限」角标系 NFR-05 功能降级而非权限不足 |

### 6.3 C 组（L3，4 项）— ⛔4

| 序号 | 结果 | 说明 |
|---|:---:|---|
| C-01 ~ C-04 | ⛔ | 悬停块 / 指示条着色与独立配置 —— **L3 改色，A-29 真机证伪，v1.0.0 范围外**（不测） |

### 6.4 D 组（发布前总检查，5 项）— ✅3 / 👤2（**单独核对，不计入「P0 需求项」统计**）

| 序号 | 结果 | 说明 |
|---|:---:|---|
| D-01 | 👤 | GitHub 推送为人工验收项（v1.6 §9 开头保留）；本地 `LICENSE`/`README.md`/`CHANGELOG.md` 齐备、内容核对无误 |
| D-02 | 👤 | Release 上传为人工验收项（v1.6 §9 开头保留） |
| D-03 | ✅ | **PM v1.6 §9.4 表后注书面判定「通过（放行）」，QA 可直接引用打勾**：依据 v1.6 修订门禁口径（P0 门禁 = §9.1 结果列 `☐` 的 25 项全部通过且无 Blocker/Critical），QA 实测路由 `NoOne`、Blocker/Critical = 0、878 次测试 0 失败。**勾选时须同时核对发布说明 / CHANGELOG `Not included` 段已明示四项未做与降级态** —— 已核对：`CHANGELOG.md` L72–75 明示 T04.2/3/4/5「本批未做」，`README.md` 顶部 L1 范围声明 + 降级横幅/受限角标如实 → **D-03 勾选条件全部满足** |
| D-04 | ✅ | README「已知限制」按冻结的 **K-01~K-07 共 7 条**逐条齐备，措辞与 §7.1 一致（详见 §6.5）；K-08 按 §9.4 表后注作观察项核对，不勾选 D-04 |
| D-05 | ✅ | 运行时禁用项 0 命中；`build.rs` 9 处构建期 `panic!` 与 `error.rs:321` 测试构造路径为已声明例外（Q-4） |

### 6.5 K-01 ~ K-07 逐条（D-04 交付清单）

| 编号 | README 落点 | 核对 |
|---|---|:---:|
| K-01 | 无需管理员权限（全程不提权）、UAC=0、无「以管理员身份重启」入口 | ✅ |
| K-02 | 可能被杀软 / SmartScreen 误报，提供「仅 L1 模式」规避路径（v1.5 已标注为历史注入路线风险） | ✅ |
| K-03 | Windows 更新后可能失效，自动降级为 L1 并提示（v1.5 已标注为历史注入路线风险） | ✅ |
| K-04 | 第三方托盘图标不改色（1.x 范围外） | ✅ |
| K-05 | UIA 时钟矩形未取得：`IUIAutomation2` 枚举 **30 后代命中 0**、回退固定几何；**未宣称「已通过 UIA 定位时钟」** | ✅ |
| K-06 | 诊断面板截图受限：DWM 重定向表面下 BitBlt 不可用，改用 A-02 主截图 + EX-24/A-20 采样区截图；**未宣称已提供面板自身截图** | ✅ |
| K-07 | 体积软目标：409.0 KB 超 400 KB 软目标约 9 KB，正式版 398.5 KB 已回到软目标内；**4096 KB 硬限不变、400 KB 非门禁、以发布版为准** | ✅ |

> 架构 §8.6 的 **12 条**是「产品已知限制 + 工程限制 + A-29 结论」的并集，其自身已澄清 QA 仍以需求基线 **K-01~K-07 共 7 条**为交付核对清单；本次未混用、未按 12 条去数 README。
> **K-08 观察项核对（v1.5 §9.4 表后注）**：README 最终版（md5 `62ba39935b8711feeed7b48677864fe8`，158 行）L83 标题为「已知限制（K-01 ~ K-08，共 8 条）」，L121–135 K-08 正文含 Phase A 31 s `EXPLORER_DIED`、Phase B `FAIL (maxRed=0)`、复原红像素 28→28、`sp2_manual_report.txt`、「v1.0.0 仅交付 L1 采样/判定/诊断能力，改色移出且不再计划（需求基线 v1.5 §4.6）」、「除非重新设计上屏方案」及技术资产保留的范围纪律；L137–139 写明 D-04 冻结按 7 条、K-08 为观察项 → **K-08 观察项核对通过，文档缺陷不存在**。D-04 仍按冻结的 7 条判定 ✅。

**汇总（§6 逐条表加总，与 §1、§9、§10 三处一致）：✅ 19 ｜ ⚠️ 9 ｜ 🔜 3 ｜ ⛔ 15 ｜ 👤 2 = 48 项。**
**三态换算（v1.6 §9 定义）：`☐` 门禁内 30 项（A 组 25 项 P0 门禁内 + D 组 5 项发布前总检查）｜`范围外` 15 项｜`后续版本` 3 项 = 48 项；其中 `☐` 30 = ✅19 + ⚠️9 + 👤2。**

---

## 7. 观察项与 QA 独立发现

### 7.1 EX-04 降级为观察项（按勘误 E-01 兜底条款）

- **判据**：瞬态峰值单核 CPU <0.5%，窗口 ≤1 s、间隔 ≤100 ms、取峰值不取平均；与 NFR-03「10 分钟平均 <0.1%」分别测量、分别打勾，双向禁止豁免。
- **降级原因**：① 环境纪律禁止改壁纸、动用户窗口，无法制造「动态壁纸黑白快速交替」的可控切换时刻；② 取证档仅 `fast/full/endpoints` 三档，`evidence.rs` 中没有 ≤1 s 短窗口 + ≤100 ms 间隔的 CPU 增量峰值采样实现（检索 `crates/` 全量 `EX-04|transient|peak|100 ms|short window` 无测量代码，仅 `diagnostics.rs` 文档注释提及 T03.5「面板可见时 UI 线程 CPU <0.5%」，属另一判据）；③ NFR-03 的 `GetProcessTimes` 10 分钟窗口在分辨率上无法还原 100 ms 粒度峰值。
- **已做复现尝试**：全量源码关键词检索 + 三档 profile 编排核对 + T04 full 档 600 s 日志的 `cpu_delta_100ns` 粒度核算。
- **结论**：按 E-01 降级为**观察项**，记录数值但不据此判通过/失败；判据以 NFR-03 的 10 分钟平均为准（0.0312% 已通过）。**未用 0.5% 放宽 NFR-03，也未以 NFR-03 达标豁免 EX-04**。

### 7.2 S-35 / S-37 默认值 —— **冲突已关闭，按正常核对项处理**

PM 已在 `docs/01-requirements.md` **v1.5 §6.4** 完成口径统一：采用 **`#38FFFFFF`**（WinUI/XAML `#AARRGGBB`，alpha `0x38` ≈ 22% 半透明白），不采用旧表格的 `#FFFFFF`；理由是 S-35 指示条固定色 / S-37 悬停块固定色是叠在任务栏与壁纸之上的**装饰性覆盖层，不是文字**，不透明纯白会形成生硬白块遮挡壁纸、违背 Fluent 观感。需求表格、架构 §13-2 / T03.4 与实现三者现已一致，**工程师实现本来就是这个值，实现正确、无需改代码**。

**按正常核对项逐条核对（不再是观察项）**：

| 核对点 | 结果 |
|---|:---:|
| S-35 / S-37 默认值 = `#38FFFFFF` | ✅ 单测 `s35_s37_defaults_have_alpha_0x38` 固定该值 |
| 取值范围为含 alpha 的 `#AARRGGBB` | ✅ `config.rs` ColorField 按 `#AARRGGBB` 解析 |
| 禁用 `00` 全透明 | ✅ 单测 `color_field_never_allows_fully_transparent` |
| 颜色选择器允许调 alpha | ✅ ColorField 控件含 alpha 通道 |
| A-12「恢复默认」按新默认值校验 | ✅ 单测 `restore_defaults_resets_every_value` |

> PM 另有一条保留意见仅供知悉、不影响核对：S-35 是 1–2 px 细线，22% alpha 在深色任务栏可能偏淡；但 v1.0.0 改色不可用，S-30~S-37 不产生可见效果，默认值用于配置兼容。**本项已从 §9 观察项与 §10 条件中移出；v1.6 §12 变更记录亦确认该项已定稿关闭，本报告全文已统一。**

### 7.3 QA 独立发现（工程师 §11.9 未登记的项）

| # | 严重度 | 发现 | 影响与处理 |
|---|---|---|---|
| Q-1 | Minor | `resources/app.manifest` 第 6-7 行注释仍写「需要提权的动作（注册计划任务）由 `ShellExecuteExW(runas)` 重新拉起自身完成」，与 K-01「全程不提权 / UAC=0」及实现（S-02 不注册 schtasks）矛盾；该注释随 RT_MANIFEST 嵌入三个 exe（PE 审计检出 `runas`/`ShellExecuteExW` 字符串） | **主理人裁定：本次不修，转 v1.5 待办，不阻塞 v1.0.0 发布**；**PM 已在 v1.6 §4.7 / §11 正式登记为「v1.5 遗留待办、本次不修、QA 不据此判失败」**（实测核对 L253 / L602）。理由：① 仅为 XML 注释，Windows 忽略，源码无 `runas` 调用、manifest 实为 `asInvoker`，**功能无影响**；② 修复需重建三产物，会使已 **15/15 全绿**的门禁与 **3/3 PE 核对**结果全部失效，收益不抵风险 |
| Q-2 | Info | `evidence/t04/A-20-real-screen.png` 与 `EX-24-real-screen.png` SHA-256 完全相同 | A-20 无独立截图；判据以 600 s CPU 日志为准，截图仅辅助 → 转 v1.5 待办 |
| Q-3 | Info | `evidence/t04-smoke/t04-summary.md` 截图路径引用不存在的 `evidence/t04-fast4/`；三张 `*-panel.png` 为同一张全黑图而汇总表 A-02/EX-24 标「是 ok」 | 与 K-06「未取得」的日志表述一致但易误读；该档为 fast smoke、明确标注 CPU 未取得，不作验收依据 → 转 v1.5 待办 |
| Q-4 | Minor | 字面口径「剔除 `#[cfg(test)]` 后 0 命中」下，`build.rs` 仍有 9 处构建期 `panic!`；`error.rs:321` 有测试构造硬编码路径 | 运行时代码 0 命中；两者均为已声明例外/测试数据，不判失败 → 转 v1.5 待办 |
| Q-5 | 口径澄清 | 任务书简写「双语资源缺失回退 English」，而需求 EX-26 与实现均为「缺失语言的键回退 `zh-CN`，`zh-CN` 也缺失时显示文案键本身」；`no-en` 对照档回退中文 | **按权威需求基线判定为通过**，此处仅澄清，未改判据 |

---

## 8. 已知偏差可接受性判断（不替工程师改动）

| # | 偏差 | QA 判断 |
|---|---|---|
| ① | 单实例激活通道由命名管道改为 `FindWindowW` + `WM_APP+1` | ✅ **可接受**：A-01 行为判据已真机实测通过（0.026 s / exit 0 / 进程数恒 1）；省常驻监听线程与两组 feature 绑定，有益于 400 KB 软目标；`ACTIVATE_PIPE_NAME` 常量保留 |
| ② | 正式版不编译 UIA 适配器，改 `FixedFallbackClockRectSource` 返回 `UiaUnavailable` | ✅ **可接受**：K-05 真机「30 后代命中 0」+ `UIA-real-probe.log` 佐证，core 走同一条固定几何 fallback；如实表达「发现能力未取得」而非假装可用；完整适配器保留在取证档 |
| ③ | S-35/S-37 默认值采用架构 `#38FFFFFF` | ✅ **冲突已关闭**：PM 已在 v1.5 §6.4 完成口径统一，实现正确、无需改代码；按正常核对项逐条通过（§7.2），**已从观察项移出** |
| ④ | README「已知限制」按需求侧 7 条核对（架构 §8.6 旧文 8/11/12 条为并集） | ✅ **可接受**：需求 v1.4 §7.1 / D-04 权威且 v1.6 明确保持冻结；架构 §8.6 自身澄清 12 条是并集、QA 以 7 条为交付清单；K-08 另作观察项核对 |
| ⑤ | 诊断包生成但不自动打开文件夹 | ✅ **可接受**：避免新起 explorer 触碰用户环境；相对 S-46「生成 zip 并打开所在文件夹」为轻微偏差 → 转 v1.5 待办 |
| ⑥ | **T04.2 计划任务自启 / T04.3 崩溃恢复 / T04.4 注入编排框架 / T04.5 日志轮转未做；S-02 只持久化设定不注册 schtasks** | ✅ **归属已由 PM v1.6 §4.7 书面裁定，QA 不得记为失败**：**T04.2 计划任务自启注册 / T04.3 崩溃恢复 / T04.5 诊断包日志轮转 → v1.0.0 明确不含、v1.1 候选补齐**；**T04.4 注入编排框架 → 永久移出、不再计划**（随 A-29 真机证伪，与架构 T05 一致处理），**不列入后续版本**。关联 A-04 / A-05 / A-27 归「后续版本」（§6.1）。CHANGELOG `Not included` 段与 README 顶部 L1 范围声明已如实明示，产品如实显示降级横幅与受限角标 → **D-03 勾选前置核对已满足**（§6.4） |
| ⑦ | 诊断面板自身截图按 K-06 标「未取得」，用 A-02 主截图 + EX-24/A-20 采样区截图替代 | ✅ **可接受且如实**：full 档两张 `-real-screen`/`-tray-annotated` 有效，日志明确写 `NOT obtained`，未虚报 |

> **FR 侧范围说明（v1.5 §4.6）**：14 条 FR 标「v1.0.0 范围外」，其中 **FR-20 / FR-33 / FR-34 / FR-35 不依赖注入、不受 A-29 证伪影响**，其产品约束由 NFR-11 / K-01 与 §6 设置项 + L1 诊断面板承接；PM 明确「不得把这几项记为功能缺失或 QA 失败」—— **本报告未将这 4 条记为缺陷**。

---

## 9. 遗留问题（按严重度）

**Blocker / Critical：无。** 自动化测试 0 失败，门禁全绿，无源码 Bug 需返工。

**已关闭的流程决策项（原 Major，现全部关闭）**

1. ~~D-03 与 §9 发布条件的冲突~~ —— **已关闭**：PM v1.6 §9 修订通过判定（P0 门禁 = §9.1 结果列 `☐` 的 **25 项**全部通过且无 Blocker/Critical，D 组为发布前总检查单独核对），并书面判定 **D-03「通过（放行）」**（§9.4 表后注「QA 可直接引用打勾」）；主理人已确认放行。A-04/A-05/A-27 归「后续版本」、B/C 共 15 项归「范围外」，**均不计入门禁、不得记为失败**。QA 勾选 D-03 时须核对的「CHANGELOG `Not included` 段明示四项未做 + 降级态」**已核对满足**（§6.4）。
2. ~~S-35 / S-37 默认值口径冲突~~ —— **已关闭**：PM 已在 v1.5 §6.4 完成口径统一（`#38FFFFFF`），实现正确、无需改代码，按正常核对项逐条通过（§7.2）。

**Minor / 观察项（均转 v1.5 待办，不阻塞 v1.0.0）**

3. EX-04 瞬态峰值按 E-01 降级为观察项（原因与复现尝试见 §7.1）。
4. Q-1：manifest 注释残留 `runas` 措辞 —— **主理人裁定本次不修、转 v1.5 待办、不阻塞发布**；**PM 已在 v1.6 §4.7 / §11 正式登记**（理由见 §7.3：功能无影响，修复会重建三产物并使 15/15 门禁与 3/3 PE 结果失效，收益不抵风险）。
5. Q-4：`build.rs` 9 处构建期 `panic!`、测试构造硬编码路径（已声明例外）。
6. 长时/真机类证据未取得（**9 项观察项，均非失败**）：A-03 动态壁纸 10 min + 20% 污染真机复现、A-10 强杀重启、A-11 只读目录、A-15 干净虚拟机、A-19 30 min 工作集、A-21 5 次冷启动、A-22 1 h 出站监控、A-23 系统语言真机首启、A-28 极端污染 60 s —— 均由单测 / 静态 / 已有 full 档覆盖，但未独立真机复现，如实标观察项。
7. Q-2 / Q-3：A-20 与 EX-24 截图字节相同；`t04-smoke/t04-summary.md` 路径引用历史目录且 panel 图全黑。
8. 偏差⑤：诊断包不自动打开所在文件夹。

**范围外（不计失败、不参与 P0 门禁）：共 15 项 = B-01~B-11（11 项）+ C-01~C-04（4 项）**，依据 A-29 真机证伪与 v1.6 §9 三态定义；B-06、B-11 均在其中，**不再另算人工验收项**。

**后续版本（v1.1 候选，不计入门禁、不算通过也不算失败、QA 不得记为失败）：共 3 项 = A-04 / A-05 / A-27**，依据 PM v1.6 §4.7 书面裁定。

**人工验收项：按 v1.6 §9 开头共 2 项（D-01、D-02）**；**A-04 已从人工验收项移除**（归入「后续版本」），**B-06 已归入「范围外」** —— 同一项不再出现在两个分类里，无重复计数。

---

## 10. 智能路由判定与放行建议

### 路由判定：**NoOne**（本版更正不改变代码结论）

- **Send To Engineer（源码 Bug）**：否。四档 878 次测试执行 0 失败、clippy 0 告警、build.ps1 15/15 门禁全绿、CLI 12/12、PE 3/3、单实例真机 PASS；未发现任何「测试正确但实现错误」的用例，无需返工。
- **Send To QA（测试代码 Bug）**：否。本轮未出现断言写错的情况，无需自修。
- **轮次**：Round 1 一次收敛，未进入 Round 2；无 Round 2 遗留失败，故不标注 Known Issues 式的失败清单（§9 的「遗留问题」为观察项与已关闭流程项，非失败测试）。
- **本版更正**：仅更正报告内的归类与数字一致性，**未重跑测试、未改代码**，代码零改动 → 上述结论全部继续有效。

### 放行结论：**放行**

**48 项最终分布（与 §1、§6、§9 三处一致）：✅ 19 ｜ ⚠️ 9 ｜ 🔜 3 ｜ ⛔ 15 ｜ 👤 2 = 48。**
**v1.6 三态口径：`☐` 门禁内 30 项（A 组 25 项 P0 门禁内 + D 组 5 项发布前总检查）｜`范围外` 15 项｜`后续版本` 3 项 = 48 项。**

| 条件 | 状态 | 依据 |
|---|:---:|---|
| ① PM 书面裁定（S-35/S-37 + 偏差⑥范围归属） | **✅ 已关闭** | S-35/S-37 冲突 v1.5 §6.4 关闭；偏差⑥归属见 PM v1.6 §4.7（T04.2/3/5 → v1.1 候选；T04.4 → 永久移出、与 T05 一致） |
| ② 发布说明明示 | **✅ 已关闭** | `CHANGELOG.md` L72–75 `Not included (L1 范围外)` 明示 T04.2/3/4/5「本批未做」；`README.md` 顶部 v1.0.0 L1 范围声明（不注入 explorer / 不使用 hook / 不修改任何系统进程）+ K-08 + K-01~K-04 范围声明；产品降级横幅与受限角标如实 |
| ③ D-03 放行口径由主理人确认 | **✅ 已关闭** | **主理人确认放行**；依据 PM v1.6 §9.4 表后注「D-03 判定 = 通过（放行）、QA 可直接引用打勾」+ QA 路由 `NoOne` + Blocker/Critical = 0 + 门禁内 25 项全绿 |
| ④ EX-04 观察项、Q-1/Q-4 转待办 | **✅ 已关闭** | EX-04 按 E-01 记观察项并写明原因与复现尝试；**Q-1 已由 PM 登记进 v1.6 §4.7 / §11**（v1.5 遗留待办、本次不修）；Q-2/Q-3/Q-4/偏差⑤ 均保留为 Minor 待办 |

**§10 未关闭条件：无（0 项）。** 四个放行条件全部已关闭，无需再等任何书面确认或流程回传。

**QA 立场**：代码质量与 L1 核心功能本次独立验证充分、可交付；PM v1.6 书面裁定与主理人放行确认已齐备，**D-03 勾选通过、结论为「放行」**。我未对任何判据、需求/架构文档、A-29 相关代码或用户环境做过改动；9 项观察项与 Minor 待办均为证据粒度/文档清理项，不构成失败、不阻断发布。

---

## 11. v1.5 / v1.6 定稿后的增量核对（本版新增）

> 本章记录**验证执行期间上游文档的定稿变更**及其对 QA 打勾的影响。全部为文档/口径变更，**零代码改动**，不触发重新构建。

### 11.1 变更时间线与实测校验值

| 文件 | 实测 md5（本报告执行时） | 行数 | 与主理人给定值 |
|---|---|---:|---|
| `README.md` | `62ba39935b8711feeed7b48677864fe8` | 158 | ✅ 与最终版完全一致 |
| `CHANGELOG.md` | `54087395dfd9cc5a33a947e3f11fac99` | 77 | ✅ 与最终版完全一致 |
| `docs/01-requirements.md`（**v1.6 定稿**） | `995f53e1309fad7fa009b0ce3c2f0b55` | 624 | ✅ 与主理人给定值完全一致（v1.6 定稿取代全部中间态） |
| `docs/qa-report.md`（本报告） | 见回传消息 | 见回传消息 | 更正后实测回传 |

> 说明：主理人先前给出的「README 同步中、K-08 记待同步」及两版旧 md5 均已撤回；本次实测以**最终冻结版**为准。README L83 已是「K-01 ~ K-08，共 8 条」，L121–135 为 K-08 正文全要素，L137–139 写明 QA 核对口径；`v1.4 / 挂起 / 待 A-29 / 待附录 / 人工验收结果决定` 等过时措辞与禁用宣称均为 0 命中。

### 11.2 K-08 观察项与 D-04 冻结

- **D-04 判据冻结不动**：仍按 **K-01~K-07 共 7 条**逐条打勾（§6.5），判定 ✅。
- **K-08 = 任务栏改色功能不可用（A-29 真机证伪）**，按 v1.5 §9.4 表后注作为**发布前观察项**核对：README 已写明「Windows 11 26H2 上任务栏改色不可用、两条实现路线经用户真机证伪、v1.0.0 仅交付 L1」→ **观察项核对通过**；**不勾选 D-04、不计入 D-04 通过/失败**。
- 架构 §8.6 的 **12 条**是「产品已知限制 + 工程限制 + A-29 结论」并集，以其自身澄清为准，QA 仍以需求基线 7 条为交付清单 —— 未按 12 条去数 README。

### 11.3 范围外最终清单（15 项）与 FR 侧 4 条

- **B-01~B-11（11 项）+ C-01~C-04（4 项）= 15 项**，v1.5 §9.2 / §9.3 结果列已全部改为「范围外」，**不算失败也不算通过、不参与 P0 门禁**；**B-11 已归入范围外**（故范围外 15 而非 14），**B-06 已随 L2 移出、不再另算人工验收项**。
- **A-01~A-28（28 项）**：25 项保留 `☐` 打勾，**3 项（A-04/A-05/A-27）按 v1.6 归「后续版本」**。
- FR 侧 14 条标范围外，其中 **FR-20 / FR-33 / FR-34 / FR-35 不依赖注入、不受 A-29 证伪影响**，PM 明确「不得记为功能缺失或 QA 失败」；其约束由 NFR-11 / K-01 与 §6 设置项 + L1 诊断面板承接 → **本报告未记为缺陷**。

### 11.4 稳态段编排 PM 已正式认可

v1.5 §9.1 注已写入认可结论（三条理由见 §3.2）。**QA 口径写死**：稳态拍数看 **`steady_sample_ticks`（=59）**，不是 `steady_publishes`（=0）；静态纯色背景 `change_threshold=8` 使 60 s 只发布 1 次判定，`steady_publishes=0` 属正常，**不得据此判不通过**；第一次 1/2 obtained 记为编排缺陷，不作判据放宽依据。A-13 / A-14 判据数值一字未动、2/2 obtained 维持 ✅。

### 11.5 S-35 / S-37 冲突已关闭

PM 在 v1.5 §6.4 完成口径统一（`#38FFFFFF`，含 alpha `#AARRGGBB`、禁用 `00` 全透明、颜色选择器允许调 alpha、A-12 按新值校验），**工程师实现本来就是这个值，实现正确、无需改代码**。本报告 §7.2 已改为正常核对项逐条通过，§8 偏差③、§9 Major 2、§10 条件① 同步关闭；**v1.6 §12 变更记录亦确认该项已定稿关闭，全文已统一为定稿口径**。

### 11.6 发布说明明示（条件② 已关闭）

`CHANGELOG.md` L72–75：
```
### Not included (L1 范围外)
- T04.2 计划任务自启注册、T04.3 崩溃恢复、T04.4 注入编排框架、T04.5 诊断包轮转：
  本批未做，见 docs/spike-report.md §11「明确未做」。
```
配合 `README.md` 顶部 v1.0.0 L1 范围声明与 K-08 → **§10 条件② 已关闭**；该核对同时是 PM v1.6 要求的 **D-03 勾选前置条件**，已满足（§6.4）。

### 11.7 产物与既有测试结论的有效性复核

- `dist/` 三产物实测体积仍为 **408064 B / 498688 B / 401920 B**，与我 build.ps1 重建后 `pe-audit-final.txt` 一致；08:46 的时间戳变动来自**我本人执行 `build.ps1` 重新生成产物**，非交付内容被替换。
- README / CHANGELOG / 需求文档改版为**零代码改动** → **878 次测试执行、四档 clippy、15/15 门禁、3/3 PE 核对、12/12 CLI、A-01 真机结果全部继续有效**，按主理人要求**未重跑全量测试**。
- 环境纪律复核：验证前后 `explorer pid 36264` / `TranslucentTB pid 27336` 未变，ShadeBar 残留进程 0。

### 11.8 v1.6 定稿四项裁定（实测核对）

| # | 裁定 | 实测落点 | QA 处理 |
|---|---|---|---|
| 1 | **A-04 / A-05 / A-27 = 「后续版本」（v1.1 候选），不标「范围外」**，不计入 P0 门禁、不勾选、不算通过也不算失败、**QA 不得记为失败** | v1.6 §4.7 L232、§9.1 结果列 L502/503/525、§9 L490/493、§12 L624 | §6.1 三项改 🔜；§9 从 ❌ 移出；§8 偏差⑥ 改为「归属已裁定」 |
| 2 | **§9 发布条件改写**：P0 门禁 = §9.1 结果列 `☐` 的 **25 项**全部通过且无 Blocker/Critical；标「范围外」/「后续版本」不纳入门禁；D-01~D-05 单独核对、不计入「P0 需求项」统计 | v1.6 §9 L490、§9.4 L570 | §1 / §6 / §9 / §10 三态换算已写入；D 组标注「单独核对」 |
| 3 | **D-03 判定 = 通过（放行）**，QA 可直接引用打勾；勾选时须核对 CHANGELOG `Not included` 段明示四项未做与降级态 | v1.6 §9.4 表后注 L570 | §6.4 D-03 改 ✅ 并完成前置核对；§10 条件③ 已关闭；放行结论改为「放行」 |
| 4 | **新增「结果列三态定义」**：`☐` 门禁内｜`范围外` 不再计划｜`后续版本` 仍计划、v1.0.0 明确不含 | v1.6 §9 L493 | §6 图例与 §1 数字口径说明按三态重述；人工验收项按 v1.6 §9 开头改为 **D-01/D-02 共 2 项**（A-04 已移除、B-06 归范围外），无重复计数 |

**偏差⑥归属（v1.6 §4.7 L242–245，写进报告备查）**：
- `T04.2 计划任务自启注册` / `T04.3 崩溃恢复` / `T04.5 诊断包日志轮转` → **v1.0.0 明确不含，后续版本（v1.1 候选）补齐**；能力仍计划，S-02 只持久化设定、UAC = 0 不受影响。
- `T04.4 注入编排框架` → **永久移出、不再计划**（随 A-29 真机证伪，与架构 **T05** 一致处理），**不列入后续版本**。

**Q-1 登记状态（v1.6 §4.7 L253 / §11 L602）**：PM 已把 Q-1（manifest XML 注释残留 `runas`）正式登记为 **v1.5 遗留待办、本次不修、不阻塞 v1.0.0 放行、QA 不据此判失败**，理由与 QA/主理人一致（仅注释、功能无影响，避免重置已全绿的三产物门禁与 PE 结果）→ §10 条件④ 已关闭。

### 11.9 增量核对结论

v1.5 / v1.6 定稿只影响**归类与文档口径**（15 项范围外、3 项后续版本、K-08 观察项、S-35/S-37 关闭、稳态认可、D-03 通过），**不改变任何实测结果与代码结论**：
- **路由判定仍为 NoOne**，Blocker/Critical 仍为 0；
- **48 项最终分布 ✅19 / ⚠️9 / 🔜3 / ⛔15 / 👤2**；三态口径 `☐` 30（含 P0 门禁内 25）｜范围外 15｜后续版本 3；
- **放行结论 = 放行**，§10 四个条件全部已关闭、**无未关闭条件**。

---

*报告生成与更正：严过关 · 所有原始输出见 `evidence/qa/` · 验证结束环境复核：explorer pid 36264 / TranslucentTB pid 27336 未变，ShadeBar 残留进程 0。*
