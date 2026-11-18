# T04 真机效果证据汇总

主日志：`evidence/t04\t04-main.log`

| 判据 | 是否取得 | 原因 | 截图 | 数值日志 |
|---|---|---|---|---|
| A-02 | 是 | ok | `evidence/t04\A-02-tray-annotated.png` | `evidence/t04\A-02-geometry.log` |
| EX-24 | 是 | ok | `evidence/t04\EX-24-real-screen.png` | `evidence/t04\EX-24-bitblt.log` |
| A-13 | 是 | ok | `evidence/t04\A-13-black-screen.png` | `evidence/t04\A-13-black-endpoints.log` |
| A-14 | 是 | ok | `evidence/t04\A-14-white-screen.png` | `evidence/t04\A-14-white-endpoints.log` |
| A-20/NFR-03 | 是 | 600s full profile measured; NFR-03 single-sample budget NOT met: p95=6725us max=11721us over_budget=241/592 (budget=5000us) | `evidence/t04\A-20-real-screen.png` | `evidence/t04\A-20-NFR-03-cpu.log` |

## 逐项采样数值

### A-02

- `found_real_tray` = `true`
- `bar_hwnd` = `0x000000000050223C`
- `tray_hwnd` = `0x000000000046052A`
- `tray_class` = `TrayNotifyWnd`
- `dpi` = `96`
- `monitor_count` = `1`
- `auto_hide` = `false`
- `auto_hide_source` = `SHAppBarMessage + geometry (both negative)`
- `tray_rect` = `(2155,1392)-(2560,1440)`
- `sample_rect` = `(2155,1396)-(2560,1436)`
- `contained_in_tray` = `true`
- `error_px` = `0`
- `tolerance_px` = `4`
- `usable` = `true`
### EX-24

- `last_decision_kept` = `true`
- `published_delta` = `0`
- `bad_frames_delta` = `6`
- `bitblt_calls` = `15`
- `recovered_after_fault` = `true`
- `dpi_recompute_after_fault` = `true`
- `outcomes` = `["Skipped(BitBltFailed { code: 0 })", "Skipped(BitBltFailed { code: 0 })", "Skipped(BitBltFailed { code: 0 })", "Skipped(BitBltFailed { code: 0 })", "Skipped(BadFrameBurst)", "Skipped(BadFrameBurst)"]`
### A-13

- `baseline_center_pixel` = `#688BBC`
- `baseline_rgb` = `(221, 223, 229)`
- `covered_rgb` = `(0, 0, 0)`
- `target_rgb` = `(0, 0, 0)`
- `coverage_delta` = `0`
- `duration_ms` = `60770`
- `samples` = `60`
- `published` = `11`
- `flips` = `1`
- `side` = `Light`
- `decision_color` = `#FFFFFF`
- `last_y_milli` = `25`
### A-14

- `baseline_center_pixel` = `#688BBC`
- `baseline_rgb` = `(224, 223, 230)`
- `covered_rgb` = `(255, 255, 255)`
- `target_rgb` = `(255, 255, 255)`
- `coverage_delta` = `0`
- `duration_ms` = `60832`
- `samples` = `60`
- `published` = `12`
- `flips` = `1`
- `side` = `Dark`
- `decision_color` = `#000000`
- `last_y_milli` = `988`
### A-20/NFR-03

- `wall_ms` = `600184`
- `cpu_delta_100ns` = `1875000`
- `logical_cpus` = `12`
- `cpu_core_pct` = `0.0312`
- `cpu_machine_pct` = `0.0026`
- `samples` = `592`
- `ticks` = `592`
- `bad_frames` = `0`
- `cost_avg_us` = `4546`
- `cost_p95_us` = `6725`
- `cost_max_us` = `11721`
- `budget_us` = `5000`
- `over_budget` = `241`
- `cost_sample_scope` = `this segment only`
- `nfr03_budget_met` = `false`
- `sleeper_waits` = `727`
- `dib_rebuilds` = `1`
- `full_10min` = `true`

## 证据级别声明

- 像素均来自真实 `BitBlt` 抓屏；黑白对照由**本进程自建纯色窗口**覆盖
  采样区产生，**未修改用户壁纸 / 注册表 / 任何用户已打开的窗口**。
- 覆盖生效经「未覆盖基线像素 vs 覆盖后像素」校验；偏差超阈值即判「未取得」。
- 单次采样耗时来自 core 的 `Instant::elapsed()` 真机测量，不是假时钟。
- CPU 占用由 `GetProcessTimes` 的 ΔCPU / Δ墙钟 计算，并给出逻辑核数换算。
- 时长档位：`full`。Fast 档的 CPU 项**明确标注未取得 10 分钟实测**。

## 明确未做（本批范围外）

- SP-2 相关实验、explorer 注入/重启、TranslucentTB 任何操作：一律未做。
- T03.2~T03.6 设置页（35 项）：未做。
- L2/L3 注入着色效果验证：v1.0.0 L1 不含，未做。
- 端到端改色效果（A-29）：仍为人工验收项，自动证据未取得。

