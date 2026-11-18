# T04 真机效果证据汇总

主日志：`evidence/t04-rerun\t04-main.log`

| 判据 | 是否取得 | 原因 | 截图 | 数值日志 |
|---|---|---|---|---|
| A-13 | 是 | ok | `evidence/t04-rerun\A-13-black-screen.png` | `evidence/t04-rerun\A-13-black-endpoints.log` |
| A-14 | 是 | ok | `evidence/t04-rerun\A-14-white-screen.png` | `evidence/t04-rerun\A-14-white-endpoints.log` |

## 逐项采样数值

### A-13

- `independent_session` = `true`
- `pre_loop_ticks` = `0`
- `baseline_center_pixel` = `#00131A`
- `baseline_rgb` = `(27, 38, 49)`
- `covered_rgb` = `(0, 0, 0)`
- `target_rgb` = `(0, 0, 0)`
- `coverage_delta` = `0`
- `duration_ms` = `60900`
- `samples` = `60`
- `published` = `1`
- `flips` = `0`
- `flips_head_excluded` = `false`
- `steady_publishes` = `0`
- `steady_sample_ticks` = `59`
- `flips_steady` = `0`
- `verdict_flips_scope` = `steady segment, head tick excluded (v1.4 9.1)`
- `side` = `Light`
- `expected_side` = `Light`
- `decision_color` = `#FFFFFF`
- `last_y_milli` = `0`
- `y_min_milli` = `none`
- `y_max_milli` = `50`
### A-14

- `independent_session` = `true`
- `pre_loop_ticks` = `0`
- `baseline_center_pixel` = `#00131A`
- `baseline_rgb` = `(27, 38, 48)`
- `covered_rgb` = `(255, 255, 255)`
- `target_rgb` = `(255, 255, 255)`
- `coverage_delta` = `0`
- `duration_ms` = `60859`
- `samples` = `60`
- `published` = `1`
- `flips` = `0`
- `flips_head_excluded` = `false`
- `steady_publishes` = `0`
- `steady_sample_ticks` = `59`
- `flips_steady` = `0`
- `verdict_flips_scope` = `steady segment, head tick excluded (v1.4 9.1)`
- `side` = `Dark`
- `expected_side` = `Dark`
- `decision_color` = `#000000`
- `last_y_milli` = `1000`
- `y_min_milli` = `950`
- `y_max_milli` = `none`

## 证据级别声明

- 像素均来自真实 `BitBlt` 抓屏；黑白对照由**本进程自建纯色窗口**覆盖
  采样区产生，**未修改用户壁纸 / 注册表 / 任何用户已打开的窗口**。
- 覆盖生效经「未覆盖基线像素 vs 覆盖后像素」校验；偏差超阈值即判「未取得」。
- 单次采样耗时来自 core 的 `Instant::elapsed()` 真机测量，不是假时钟。
- CPU 占用由 `GetProcessTimes` 的 ΔCPU / Δ墙钟 计算，并给出逻辑核数换算。
- **A-13 / A-14 各起一个独立 `EngineSession`**（v1.4 §9.1 表后注硬要求），
  不复用同一会话连跑黑→白；`flips` 按稳态段统计，排除切换背景后的段首第一拍；
  全段原始 `flips` 与段首 `flips_head_excluded` 也如实保留，判据数值未放宽。
- **NFR-03 双口径**：CPU 主判据（单核 < 0.1%）与墙钟辅助判据
  （`cost_us` p95 ≤ 10 ms / max ≤ 15 ms、样本 ≥ 500）分别记录，三项须同时满足；
  保留 `cost_us` 累计墙钟 vs `GetProcessTimes` ΔCPU 的交叉核算，差值即 DWM 阻塞等待。
  原「单次 ≤ 5 ms」已废止，仅留作溯源，不作门禁。
- 时长档位：`endpoints`。Fast 档的 CPU 项**明确标注未取得 10 分钟实测**；
  `endpoints` 档只跑 A-13/A-14（各 60 s），刻意不跑 10 分钟 CPU 段。

## 明确未做（本批范围外）

- SP-2 相关实验、explorer 注入/重启、TranslucentTB 任何操作：一律未做。
- L2/L3 注入着色效果验证：v1.0.0 L1 不含，未做。
- 端到端改色效果（A-29）：仍为人工验收项，自动证据未取得。

