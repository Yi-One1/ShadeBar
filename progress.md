# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-27 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：v1.5.4 已发布。冷开机后新开程序指示条灰色的**根因已实证**：
> Windows XAML 诊断在单进程内只有最后连接者能收到增量 Add/Remove；冷开机时
> TranslucentTB 的 ExplorerTAP 后连接，随影只剩连接瞬间的全量快照。
> **第 1 组已完成并验收**（载荷可手动重连夺回事件流，提交 `e2cca6b`）；
> **第 2 组待做**：开机自动触发重连。详见 `evidence/INVESTIGATION-STATUS.md`。

---

## 0. 新对话先做这几件事

1. `git log -12 --oneline` —— 第 1 组修复提交为 `e2cca6b`，其下是证据归档与调查提交。
2. `cargo test --release` —— 应全绿（**235 项**：app 136 + core 97 + doc 2，1 ignored）。
3. **先读 `evidence/INVESTIGATION-STATUS.md`** —— 本次调查的真根因、已排除方向、剩余路径全在里面；
   不要重走已排除的方向。
4. 需要重新体验：双击 `1. 一键启动预览.bat`（重编并启动 `target\` 最新版，**不覆盖 dist**）。
   重新打包：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户明确要求——每组做完必须真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。
> **汇报要带任务清单与进度**；发现问题先说清根因，再给方案。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本号 | **1.5.4**（`Cargo.toml` → `[workspace.package] version`，版本唯一来源） |
| 发布产物 | `dist/随影ShadeBar_v1.5.4.exe`；GitHub Release 已发布（2026-11-27） |
| **HEAD** | `e2cca6b`（第 1 组：载荷重连夺回事件流），在 `v1.5.4` tag 之后**未升版、未发布** |
| `target\release\shadebar-app.exe` | 已重新编译（17:52，含只读探针，**默认关闭**），与 dist 版不同 |
| 门禁 | 5/5（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标） |

`dist/` 永远保留历史版本 exe（用户要求，便于回滚）；exe 不进仓库，只走 Releases。

> **版本未升**：真问题未解决，不该发版。用户明确要求严格 SemVer 2.0.0：
> 修 bug = PATCH，新功能 = MINOR，升版判定要写进提交信息。

---

## 2. 已完成事项

### 2.1 2026-11-27 本次调查（重点，全部有证据）

| 事项 | 结论 / 提交 |
|---|---|
| **锁定真根因** | 冷开机会话里 **IVTS 增量 Add 事件通道不通**：注入瞬间 300+ Add 全量发放（只有 QQ，ind=1），之后开记事本/浏览器新增 Add=0、`[Tree]/[Apply] ind` 恒为 1；warm 会话（重启 explorer）Add 正常（ind 6→7）。证据 `evidence/coldboot-20261127/after/payload.log` |
| **测试污染修复** | `cargo test` 的 app.rs 测试曾启动真实配色服务/写真实注册表，污染用户机器。三层修复：`6bd7223`（autostart 测试隔离）、`1dd539e`（测试不启动真实服务 + 载荷忽略陈旧 shutdown）、`c776316`（日志加 PID/时间戳、注释对齐、不变量固化、真值表测试） |
| **QA 独立评审** | 严过关评审结论"有条件通过"，指出冷开机未闭环、证据被轮转、根因时序不自洽——**后续证明 QA 是对的**，此前"已定位根因"的判断被推翻 |
| **接口论证** | 高见远查微软官方文档：点名的 `GetElementFromHandle/GetRootElement/Walker` 等在原生 xamlom.h 里不存在；首选 `IXamlDiagnostics::HitTest`（主动 pull，不重订阅）。文档 `evidence/coldboot-20261127/IVTS-feasibility.md` |
| **HitTest 探针（已证伪）** | `9432e20` 只读探针 + `f573833` 七坐标/缓冲区/线程判别。冷开机 3 轮实测：7 种坐标 + 预分配缓冲区 + 同线程 + Advise hr=0 全部排除，HitTest 恒 `E_INVALIDARG` 且与矩形无关 → **接口在此环境不可用，方案否决** |
| **证据归档** | `a393df7`：三轮冷开机/探针的 before/after 原始日志 + FINDINGS + 接口论证 + 总览 `evidence/INVESTIGATION-STATUS.md` |

**工程状态**：全量测试 235 项通过；载荷静态 CRT 编译通过（无 VCRUNTIME/MSVCP）；
4 个分析脚本（`check_chevron_diag.py`/`check_indicator_coldboot.py`/`check_shutdown_logic.py`/`check_hittest_probe.py`）均带自测。
探针代码**只读、默认关闭**（哨兵文件 `%LOCALAPPDATA%\ShadeBar\probe.trigger` 触发），不影响正常用户，用户决定**保留**。

### 2.2 已发布版本历史

- **v1.5.4**（2026-11-27）：当时记录为"修托盘箭头不着色"。**注意：用户后来澄清箭头一直是白的，
  这是误报**——相关 chevron 修复 `c61c660` 已由 `1e3aa45` 回滚。v1.5.4 其余发布内容仍有效。
- **v1.5.3**：一部分指示条永远不亮——宿主重复注入拿到失效连接；注入前读载荷活跃计数（`reserved[6]`）存活则跳过。
- **v1.5.2**：冷启动整条链路失效——第二次 `Advise` 必返 `E_UNEXPECTED`；删除 `ReplayTree`，只订阅一次。
- **v1.5.1**：冷启动指示条预热+重放方案，已被 v1.5.2 判定错误并移除。
- **v1.5.0**：亮/暗端预览语义修正；配色与判定粒度解耦；指示条状态改 250ms 轮询 `ActualWidth`。

---

## 3. 关键决策（含理由，别重新发明）

### 3.1 本次调查新增

| 决策 | 结论 | 理由 |
|---|---|---|
| **真问题根因** | 冷开机后 IVTS 增量 Add 事件不通（**不是** shutdown、**不是**箭头、**不是**日志截断） | 多轮冷开机证据；修复版加载后 shutdown=0 仍灰 |
| **错误方向回滚** | chevron（箭头）修复已回滚 | 用户澄清箭头一直是白的，问题是新开程序指示条 |
| **shutdown 修复保留** | `6bd7223`/`1dd539e`/`c776316` 保留 | 是独立真 bug（测试污染注册表/IPC），虽非本问题根因 |
| **HitTest 否决** | 不再尝试此接口 | 7 坐标+缓冲区+线程+Advise 四假设全排除，恒 `E_INVALIDARG` |
| **探针代码保留** | `9432e20`/`f573833` 保留 | 只读、默认关闭、有回归测试，是取证工具 |
| **不升版** | 仍 v1.5.4 | 真问题未解决，不该发版 |
| **暂停** | 按用户要求暂停 | 已重启 4 次/3 轮探针，继续盲试对用户不公平 |

### 3.2 历史产品决策

| 决策 | 结论 | 理由 |
|---|---|---|
| 分区配色粒度 | 8 色 = 4 内容 × 2 背景态 + 独立不透明度 | 用户拍板 |
| 统一/分区关系 | 解耦：判定粒度只决定明暗判断，配色始终用 8 色矩阵 | 用户拍板 |
| S-14/S-15 | 仅保留配置兼容，不参与取色，UI 收起 | 解耦后无消费方 |
| 亮端/暗端语义 | 亮端 = 浅色文字 → 深色任务栏 | 需求定义 |
| 指示条状态检测 | UI 线程 250ms（空闲 1s）轮询 `ActualWidth` | `OnElementStateChanged` 对指示条不触发 |
| 正常指示条透明度 | S-60（默认 45%），两种模式都生效 | 保留系统半透明手感 |
| 单击/双击共存 | 双击非「无操作」时单击挂起 `GetDoubleClickTime` | legacy 托盘先送单击 |
| S-22 语义 | 任务栏高度百分比，上下对称内缩 | 引擎本来就这么算 |
| 指示条两态 | 颜色可不同，写入路径不同 | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color` |
| 正常态一致性 | 所有正常态共用一个画刷 | 换色同一刻生效 |
| 托盘字号定位 | 只认 `AdaptiveTextBlock` 的 `Base` 层 `InnerTextBlock` | 其余两层透明/强调覆盖 |
| ~~托盘箭头定位~~ | ~~ChevronIconView 写 Foreground~~ **已回滚（误报）** | 箭头一直是白的 |
| 元素树更新 | 只订阅一次 + `Add/Remove` 增量；**禁止重放、禁止重复注入** | 第二次 Advise 必返 `E_UNEXPECTED` |
| 托盘补挂 | 响应 `TaskbarCreated`，先 MODIFY 探测再 ADD | 无条件 `NIM_ADD` 会撞 uID |
| 开机自启 | HKCU Run，不注册 schtasks | 零 UAC |
| 设置布局 | 四页签 + 任务栏直控预览；配色卡 4×2 矩阵 | 用户选定 |
| 版本 | 严格 SemVer 2.0.0 | 用户长期规则 |
| 验收方式 | 一律 `1. 一键启动预览.bat`，不覆盖 dist | 用户明确要求 |
| 打包产物 | 新 exe 只新增，不删旧 exe | 便于回滚与对比 |

---

## 4. 未完成的待办

### 4.1 冷开机后新开程序指示条灰色（根因已定，修复分两组）

**根因（2026-11-27 实证）**：Windows XAML 诊断在单进程内只有一个实时消费者——
后连接者接管 Add/Remove 增量流，先连接者只保留连接瞬间的全量快照。冷开机时随影
载荷先连（17:56:01），TranslucentTB 的 ExplorerTAP 后连（17:56:03），于是新开程序
的事件全归 TTB；开机时已在的程序不受影响。

**第 1 组（已完成、已验收，提交 `e2cca6b`）**
- [x] 载荷新增“新连接重连”：新线程 + 新连接名重新初始化，不重订旧连接
- [x] 手动哨兵 `%LOCALAPPDATA%\ShadeBar\reconnect.req`（每秒轮询，触发后自删）
- [x] 修正清树顺序：快照在 `Advise` 内同步投递，`g_clearOnAdd` 必须先置位，失败回滚
- [x] 回归：`tools/check_reconnect_logic.py`（R1–R9，含反向自测）；
      `check_payload_sync.py` C9（重连后必须重建快照并重新着色）
- 实测：TTB 后连接 → 开记事本 0 个 Add → 哨兵 → `SetSite #2`/`Advise hr=0`/
  `[Tree] n=18 time=1 date=1 tray=6 ind=10` → 再开记事本 Add 到达且指示条着色

**关键技术约束（勿再试错）**
- 同一连接 `Unadvise` 后再 `Advise` 必 `E_UNEXPECTED` → 只能开**新连接**
- 每次 `InitializeXamlDiagnosticsEx` 尝试必须在**新线程**；失败过的线程复用无效
- 连接名 `VisualDiagConnection1..60`；每次 InitTap 都从 1 号重扫（失败不推进槽位）
- 强制重启 explorer 后 XAML 诊断需热身约 30–60 s；正常开机约 1–2 s

**第 2 组（待做）**：自动触发重连（无需用户手建哨兵）。完成后才算
“冷开机问题彻底解决”，需按 QA 硬条件做一次真冷开机验收。

**其余备选路径**（若第 2 组仍不足再评估）：
1. `GetCollectionElements` 有限重扫（不依赖增量事件，但句柄时效需先验证）。
2. 换 WinComp/DWM 染色层（v2 重构，代价大）。
3. 产品层接受限制（提示重启 explorer，当前 workaround）。

- **取证注意**：冷开机验证前先归档旧日志（日志有 262144 字节轮转，证据会被冲掉）。

### 4.2 长期缺口（用户没催，不要主动扩范围）

- 悬停高亮块（S-32 / S-34~S-37）：生产载荷尚未实现。
- 多显示器（副屏任务栏元素契约未用探针确认）。
- 崩溃恢复（`RegisterApplicationRestart` 仅在注释里，未接线）。
- 日志 7 天轮转未实现（**实测为单文件追加 + 262144 字节轮转，不是"按天切分"**）。
- 载荷副本累积：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll` 无清理（有意设计）。
- 自启项跟随"最后运行的那个 exe"：从 `target\` 启动过就指回 `target\`；
  **本次已修测试污染路径**（`6bd7223`），但产品行为本身未改。

---

## 5. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| 新版本没修好旧问题 | explorer 同时驻留多版载荷，旧副本在生效 | 清场；日志格式与源码对不上 = 跑的不是这份代码 |
| **冷开机新开程序指示条灰（本次真问题）** | 单进程内 XAML 诊断只有最后连接者收增量事件；冷开机时 TTB 后连接 | 第 1 组已实现手动重连夺回；第 2 组做自动触发 |
| 开机后指示条不着色（旧） | 首拍重放 + `Advise` 返 `E_UNEXPECTED` | v1.5.2 起只订阅一次 |
| 一部分指示条永远不亮（旧） | 宿主重复注入拿到失效连接 | v1.5.3 注入前探测存活 |
| 指示条选中/正常切换色滞后 ~2s | `OnElementStateChanged` 对指示条不触发 | UI 线程 250ms 轮询 `ActualWidth` |
| 托盘图标"改了没变" | 按 `AdaptiveTextBlock` 匹配，只有 `Base` 的 `InnerTextBlock` 可见 | 按父级名收 `Base` 层子 TextBlock |
| 托盘箭头"改了没变"（**误报**） | 用户澄清箭头一直是白的，问题在别处 | 相关修复已回滚，勿再查箭头 |
| 指示条两态颜色无法区分 | 载荷选中态误用 `indicatorNormalColor` | 改读 `indicatorSelectedColor` |
| "改了颜色却没变化" | 纯黑纯白策略下白色目标与系统默认同色 | 先用红/绿辨识色复现 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完回读画刷 `Color` |
| **跑完 `cargo test` 机器着色全失效** | 测试启动真实服务/写真实注册表与 IPC，污染用户机 | `6bd7223`/`1dd539e`/`c776316` 测试隔离 |
| **日志证据消失** | `payload.log` 262144 字节轮转 | 取证前先归档旧日志 |
| **HitTest 恒 E_INVALIDARG** | 接口在此 Win11 诊断会话下不可用 | 已否决，勿再试 |
| 滑条只在某种模式能拖 | 命中检测写在 `per_region()` 分支内 | 命中检测覆盖所有模式 |
| 取色浮层最右列点不中 | 浮层向右展开超出窗口被裁 | x 夹紧在窗口内 |
| 语言表键值重复 | 追加脚本跑两次；只查存在不查唯一 | 改后检查重复键 |
| 软件在跑、托盘没图标 | 不响应 `TaskbarCreated`；`added` 失败不清零 | 广播补挂 + 失败归零 |
| 重复 `NIM_ADD` 返回 `E_FAIL` | 图标已存在撞同一 uID | 先 `NIM_MODIFY` 探测 |
| `build.ps1` 报 ps1 encoding guard | 脚本要求纯 ASCII，混入中文注释 | 注释只用 ASCII；需中文正则存 UTF-8 BOM |
| 界面能看见但点不中 | 绘制/命中两套坐标 | 统一坐标语义 + 纯 frame 函数 + 变异测试 |
| 还原留色 | 应用逐层染、还原只改一层 | 还原与应用对称 |

---

## 6. 环境、命令与限制

### 6.1 构建与测试
```bash
cd "<repo>"

# 载荷（C++，必须先编，宿主编译期内嵌它）
& 'C:\Program Files\Git\bin\bash.exe' payload/build.sh

# 宿主 / 测试 / 打包
cargo build --release
cargo test --release
powershell -ExecutionPolicy Bypass -File build.ps1

# 本次新增分析脚本（均带 --self-test）
python tools/check_payload_sync.py
python tools/check_hittest_probe.py --self-test
python tools/check_shutdown_logic.py --self-test
```

`payload/build/`、`dist/verify/`、`dist/compare/` 是构建中间物，不进仓库。

### 6.2 本机硬限制（踩过）
- 工具调用起的 GUI 进程会被回收：启动与断言必须写在同一命令里。
- PowerShell `Add-Type` 在工具调用内被拦；截图/断言用 Python + `ctypes` + Pillow。
- **截图必须先征得用户同意，同意有时限**（用户长期规则）。
- 工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- 本机**没有 `gh` CLI**：Release 走 GitHub REST API（环境有 `GH_TOKEN`）；`git push` 走 SSH。
- dist 版进程名 `随影ShadeBar_v<ver>`，`target` 版是 `shadebar-app`；单实例互斥会让后者启动即退出——测新代码前先按 PID 停旧实例。
- **Bash 里不能调 PowerShell**（安全策略拦截），要用 PowerShell 工具；Win+R 运行框不支持 `type` 等 cmd 内置命令。

### 6.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（**49 项编号**；缺失尾部按默认补）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷）、`shadebar.log`（宿主，带 `[pid 时间]` 前缀）；
  **payload.log 有 262144 字节轮转，取证前先归档**
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 探针触发：`%LOCALAPPDATA%\ShadeBar\probe.trigger`（空文件，宿主消费后删除；默认关闭）
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%

### 6.4 关键参数（改代码前必看）
- IPC `ShadeBarIpc` **80 字节**；`reserved[0]` 元素掩码、`reserved[1]` 代际、
  `reserved[2]` 探针触发（新增）、`reserved[6]` 载荷活跃计数。
- 队列位序 `SettingKey as usize`，新增项只能追加末尾（`SETTING_COUNT`=49）。
- 分项色：S-52/53 正常条、S-54/55 选中条、S-56/57 托盘、S-58/59 时间日期
  （成对「背景偏暗/偏亮」），S-60 正常条不透明度（0–100，默认 45）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块。
- 采样必须用屏幕 DC，不能用窗口 DC。
- 体积单 exe 约 672 KB；不得引入新 crate（windows 锁 `=0.58.0`）。

---

## 7. 用户偏好（长期有效，别违反）

- 语言：简体中文。
- 截图前必须询问并取得同意，同意有时限。
- 安装应用必须先问、由用户选路径。
- 删除文件先列清单、说明影响，得到授权后才删。
- 版本号严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- 每组必须真机验收，他说「没问题」才继续；汇报要带任务清单与进度。
- 不要替他做"这个不重要"的判断；发现问题先说清根因，再给方案。
- **不要用"应该没问题"交差**——必须有证据；用户会追根因，判断错误要如实承认并纠正。

---

## 8. 本次调查证据索引

| 文件 | 内容 |
|---|---|
| `evidence/INVESTIGATION-STATUS.md` | **总览**：真根因、已排除方向、剩余路径（新对话必读） |
| `evidence/coldboot-20261127/FINDINGS.md` | 冷开机根因首证（Add 停在 201、ind 恒为 1） |
| `evidence/coldboot-20261127/IVTS-feasibility.md` | 架构师接口论证（含官方来源 URL、兜底方案） |
| `evidence/coldboot-20261127/{before,after}/` | 首轮冷开机前后日志 |
| `evidence/probe-coldboot-20261127/` | 第一轮 HitTest 探针（E_INVALIDARG 初现） |
| `evidence/probe2-coldboot-20261127/` | 第二轮七坐标探针（四假设全排除） |

---

## 9. 新对话冒烟验证（3 分钟）

```bash
# 1) 清场（停旧实例）
powershell -c "Get-Process *ShadeBar*,shadebar-app -ErrorAction SilentlyContinue | Stop-Process -Force"

# 2) 构建 + 测试与载荷回归
cargo build --release
cargo test --release                       # 应 235 项全绿
python tools/check_payload_sync.py
python tools/check_hittest_probe.py --self-test
python tools/check_shutdown_logic.py --self-test

# 3) 看五处：时间/日期、托盘字形、托盘箭头、正常/选中指示条是否按设置着色
# 4) 真问题复现步骤：冷开机 → 开记事本/浏览器 → 看新窗口指示条是否灰色
```
