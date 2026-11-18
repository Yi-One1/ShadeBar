# NFR-03 双口径核对记录（需求基线 v1.4）

> **证据级别声明**：本文件是**从 `evidence/t04/A-20-NFR-03-cpu.log`（T04 full 档
> 600 s 真机实测）派生的口径核对表**，不是新一轮 10 分钟测量。
> 本批 `--verify endpoints` 按需求基线 v1.4 §9.1 表后注**只跑 A-13/A-14**（各 60 s、
> 独立 `EngineSession`），刻意不重跑 10 分钟 CPU 段，因此没有新的 CPU 实测值。
> 文件放在 `evidence/` 根目录（而不是会被 `--verify endpoints` 清空的
> `evidence/t04-rerun/`），便于 QA 反复核对。
> QA 打勾 A-20 时须同时核对本表与原始 `evidence/t04/A-20-NFR-03-cpu.log`。

## 1. 原始实测值（来源：evidence/t04/A-20-NFR-03-cpu.log）

| 指标 | 值 | 口径 |
|---|---:|---|
| `wall_ms` | 600184 | 10 分钟墙钟窗口 |
| `cpu_delta_100ns` | 1875000 | `GetProcessTimes` 的 Kernel+User 增量 |
| `logical_cpus` | 12 | 逻辑核数 |
| `cpu_core_pct` | 0.0312 | 单核 CPU%（ΔCPU ÷ 墙钟 × 100） |
| `cpu_machine_pct` | 0.0026 | 全机 CPU%（单核 ÷ 核数） |
| `samples` | 592 | `cost_us` 样本数（≥ 500） |
| `cost_avg_us` | 4546 | 本段 `cost_us` 平均（微秒） |
| `cost_p95_us` | 6725 | 本段 p95（微秒） |
| `cost_max_us` | 11721 | 本段 max（微秒） |
| `bad_frames` | 0 | 坏帧 |
| `flips`（CPU 段） | 0 | 10 分钟内判定翻转 0 次 |

`cost_us` 的测量范围是 `crates/shadebar-core/src/engine/sampler.rs:835` 的
`Instant::elapsed()` 所包裹的一段：**含真实 BitBlt 抓屏 + 直方图 + 判定，
不含 UIA 与几何计算**（需求基线 v1.4 §7 NFR-03）。

## 2. 交叉核算（DWM 阻塞等待，不是代码耗时）

```
cost_us 累计（墙钟） = 592 × 4546 µs = 2 691 232 µs ≈ 2.6912 s
GetProcessTimes CPU   = 1 875 000 × 100 ns = 0.1875 s
差值（DWM 合成/抓屏阻塞等待） = 2.6912 s − 0.1875 s = 2.5037 s
```

差值约占 `cost_us` 累计的 93%，说明**墙钟 `cost_us` 主要是阻塞等待，不是进程 CPU 消耗**。
这正是「单次墙钟 >5 ms」与「单核 CPU 0.0312%」同时成立的原因：两者度量的不是同一件事。
因此原「单次 ≤ 5 ms」被判定为**口径错误**并废止（需求基线 v1.4 CR-04），
但**墙钟要求没有删除、max 上限没有放宽到 20 ms 了事**。

## 3. v1.4 三项判据（须同时满足，不得互相替代）

| # | 判据 | 阈值 | 实测 | 结果 |
|---|---|---:|---:|---|
| 1 | 主判据：10 分钟平均单核 CPU | < 0.1% | 0.0312% | ✅ 通过 |
| 2 | 辅助：`cost_us` p95 | ≤ 10 ms（10000 µs） | 6.725 ms（6725 µs） | ✅ 通过 |
| 3 | 辅助：`cost_us` max | ≤ 15 ms（15000 µs） | 11.721 ms（11721 µs） | ✅ 通过 |

样本数 592 ≥ 500，满足取证要求。
**结论：NFR-03 v1.4 双口径三项同时满足。**
CPU 达标不得豁免墙钟超线；墙钟超线也不得反推 CPU 不达标（本批未出现超线）。

## 4. 已废止口径（仅溯源，不作门禁）

| 项 | 值 | 说明 |
|---|---:|---|
| `budget_us` | 5000 µs | 原「单次 ≤ 5 ms」 |
| `over_budget` | 241 / 592 | 按已废止口径统计的超线帧数 |

`evidence.rs::verify_cpu` 现在把 5 ms 记为 `budget_us_deprecated` /
`nfr03_legacy_5ms_met`，并新增 `nfr03_cpu_met` / `nfr03_wall_met` /
`nfr03_dual_met` 与 `cross_check` 交叉核算行；代码里没有任何为了凑 5 ms
而修改采样实现的改动（team-lead / PM v1.4 明确禁止）。

## 5. 与 A-13 / A-14 独立重跑的关系

本批 `evidence/t04-rerun/` 的 A-13 / A-14 日志来自 `--verify endpoints`：
纯黑与纯白**各起一个独立 `EngineSession`**，各 60 s，`flips` 按稳态段统计
（排除切换背景后的段首第一拍；实测两者全段 `flips=0`，段首也无翻转，
即按更严格的全段口径通过）。10 分钟 CPU 段 `flips=0` 的记录仍来自 T04 full 档；
两者互不影响，判据数值未放宽为「允许 1 次翻转」。
