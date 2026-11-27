# 冷开机指示条不着色 — 调查现状（2026-11-27）

## 用户原始问题
冷开机后，开机时就在的程序（QQ）指示条白色正常；**之后手动新开的程序（浏览器、记事本）指示条灰色不染色**。重启 explorer 后恢复。

## 真根因（最终结论，2026-11-27 18:5x 实证）
**Windows XAML 诊断在单进程内只有一个实时消费者：后连接者接管 Add/Remove 增量流，
先连接者只保留连接瞬间的全量快照。**
- 冷开机注入那一刻：全量 Add 正常（含开机时已在的 QQ），之后新开程序 0 个 Add
- 现场 A/B：唯一消费者时开记事本 → ind 9→10 且着色；ExplorerTAP 后连接后
  开计算器/画图 → 0 个 Add，树停在 ind=9
- 冷开机时序：随影载荷 17:56:01 先连，TranslucentTB 的 ExplorerTAP 17:56:03 后连
- 重启 explorer 时 TTB 先注入、随影 1–2 s 后注入 → 随影最后连接 → 正常
- 主证据：`evidence/coldboot-20261127/after/payload.log`、
  `evidence/tap-consume-20261127/`

## 修复进展（已闭环，v1.5.5）
- **第 1 组（提交 `e2cca6b`）**：载荷支持“新连接重连”，哨兵
  `%LOCALAPPDATA%\ShadeBar\reconnect.req` 手动触发；实测 TTB 后连接导致灰色后，
  哨兵可夺回事件流并让新开程序着色。
- **第 2 组（提交 `6acf2e5` / `eb008dc`）**：自动触发重连——首连后 10 s 兜底、
  TranslucentTB 每次启动 +8 s、`ExplorerTAP` 加载 +2 s；两次自动重连至少间隔 10 s。
  自动重连不能一次性锁死（用户可能在随影首连很久后才打开 TTB）。
- **真冷开机验收通过（2026-11-27 22:28）**：22:29:27 首连 #1 → 22:29:31 TTB 启动
  → 22:29:34 自动重连 #2 → 之后新开记事本/画图均送来 `[Add] RunningIndicator`，
  指示条为白色，全程无人工操作。
- 结论：根因已消除，问题闭环。

## 已排除的方向（附证据，勿重走）

### ✗ 托盘「显示隐藏的图标」箭头不着色
用户最初误报；澄清后箭头一直是白的。相关 chevron 修复已提交 `c61c660` 后由 `1e3aa45` 回滚。
证据：`evidence/coldboot-20261127/FINDINGS.md`

### ✗ shutdown=1 卡死导致新窗口不染色
曾怀疑 `cargo test` 污染 IPC shutdown=1 让载荷待命。冷开机验证推翻：
- 修复版载荷（build 16:28:36）确实加载、`shutdown=0`、`status=1(READY)`
- 指示条**仍灰色** → 问题与 shutdown 无关
- 相关修复 `6bd7223`/`1dd539e`/`c776316` 经用户确认**保留**（是独立 bug：测试污染注册表/IPC，本身该修），但非本问题根因

### ✗ Add 事件被日志预算截断（"其实来了只是没记录"）
加全局 Add 计数诊断后确认：`g_addTotal` 停在 201，新增事件真的没来，不是日志截断。

### ✗ IXamlDiagnostics::HitTest 主动命中（首选方案，已证伪）
架构师论证可行（官方要求先 Advise，本项目已满足 hr=0），实现只读探针 `9432e20`/`f573833`，冷开机触发 3 轮。**四种假设全部排除**：
| 假设 | 实测 | 结论 |
|---|---|---|
| 坐标系（屏幕/DIP/根局部） | 7 种坐标含原点小矩形全 `E_INVALIDARG` | 排除 |
| GetUiLayer 前提 | hr=0、ptr 非空 | 排除 |
| 缓冲区 count 容量 | 预分配 512 仍 `E_INVALIDARG` | 排除 |
| 线程亲和 | setSiteTid=probeTid=8716 同线程 | 排除 |
| Advise 前提 | Advise hr=0 | 排除 |
`E_INVALIDARG` 与矩形内容无关 → 该接口在此 Win11 环境/诊断会话下本身不可用。
UIA 对照 count=0（看不到 XAML 内部 Rectangle，符合预测）。
证据：`evidence/probe-coldboot-20261127/`、`evidence/probe2-coldboot-20261127/after-payload.log`

## 当前代码状态（HEAD = f573833，工作区仅证据目录未跟踪）
- 所有探针改动**只读、默认关闭**（哨兵文件触发），不影响正常用户
- 全量测试 235 项通过；载荷静态 CRT 编译通过
- 分析脚本 4 个：`check_chevron_diag.py`、`check_indicator_coldboot.py`、`check_shutdown_logic.py`、`check_hittest_probe.py`，均带自测
- 用户决定：探针代码**保留**

## 剩余可选方向（架构师论证 §6，按代价递增）
1. **GetCollectionElements 有限重扫**：枚举已知容器当前子元素，不依赖增量事件；风险是句柄时效（铁律6），需再验证
2. **宿主检测 explorer 启动后重新注入新 DLL 实例**：非同实例重 Advise，可能最彻底；需评估 v1.5.2 DLL 常驻约束
3. **换染色层**（WinComp/DWM 自绘）：v2 重构，代价大
4. **产品层接受限制**：冷开机后需手动重启 explorer（当前 workaround）

## 下一步（待用户决定）
用户选择**暂停**，下次继续时从"剩余可选方向"里选一条。推荐先评估方向 2（重新注入），因为根因是"增量通道不通"，换 DLL 实例重建订阅最直接；方向 1 需先验证句柄时效，仍可能重蹈覆辙。

## 关键文档
- `evidence/coldboot-20261127/FINDINGS.md` — 冷开机根因首证
- `evidence/coldboot-20261127/IVTS-feasibility.md` — 架构师接口论证（含官方来源）
- 本文件 — 总览与排除清单
