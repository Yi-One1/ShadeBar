# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-27（v1.5.5 收尾）
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：冷开机后新开程序指示条灰色的根因已定位并根治——
> Windows XAML 诊断在单进程内只有一个实时消费者，后连接者（TranslucentTB 的
> ExplorerTAP）会独占增量事件流。v1.5.5 起载荷在首连后自动重连、并在 TTB
> 每次启动时重新夺回席位；真冷开机已验收通过（新图标指示条为白色）。

---

## 0. 新对话先做这几件事

1. `git log -8 --oneline` —— HEAD 为 v1.5.5 收尾提交，其下是两组修复与证据归档。
2. `cargo test --release` —— 应全绿（**235 项**：app 136 + core 97 + doc 2，1 ignored）。
3. 读 `evidence/INVESTIGATION-STATUS.md` —— 根因、实证数据与最终结论，别重走已排除方向。
4. 预览：双击 `1. 一键启动预览.bat`（重编并启动 `target\` 最新版，不覆盖 `dist\`）。
   打包：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户要求每组实机验收，说「没问题」后才更新文档并进入下一组。
> 汇报必须带任务清单与进度；发现问题先说清根因，再给方案。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本号 | **1.5.5**（`Cargo.toml` → `[workspace.package] version`，唯一来源） |
| 发布产物 | `dist/随影ShadeBar_v1.5.5.exe`；GitHub Release `v1.5.5` |
| 载荷文件名 | `ShadeBarTap_v1.5.5_<内容指纹>.dll`（指纹保证内容变必换新文件） |
| 门禁 | 5/5（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标） |

`dist/` 保留历史版本 exe（用户要求，便于回滚）；exe 不进仓库，只走 GitHub Releases。
版本规则：严格 SemVer 2.0.0，修 bug = PATCH，新功能 = MINOR。

---

## 2. 已完成事项

### 2.1 冷开机指示条灰色（本次主线，已闭环）

| 阶段 | 结论 / 提交 |
|---|---|
| 根因锁定 | XAML 诊断**后连接者独占增量 Add/Remove**；冷开机随影先连（17:56:01）、TTB 的 `ExplorerTAP.dll` 后连（17:56:03），新窗口事件全归 TTB |
| A/B 实证 | 唯一消费者时开记事本 ind 9→10 且着色；TTB 后连接后开计算器/画图 0 个 Add，树停在 ind=9 |
| 第 1 组 | `e2cca6b`：新线程 + 新连接名重连（不重订旧连接）、手动哨兵、清树顺序修正、R1–R9 + C9 |
| 第 2 组 | `6acf2e5`：首连后 10 s 自动重连 + ExplorerTAP +2 s；`eb008dc`：修正一次性锁死，TTB **每次启动** +8 s 重连（实测用户场景：TTB 晚开 4 分钟仍能夺回） |
| 真冷开机验收 | 2026-11-27 22:28 重启；22:29:27 首连 #1 → 22:29:31 TTB 启动 → 22:29:34 自动重连 #2 → 之后开程序 `[Add] RunningIndicator` 持续到达，指示条白色 |
| 测试 | 235 项全绿；`check_reconnect_logic.py` R1–R12（含反向自测）、`check_payload_sync.py` C1–C9 全 PASS |

### 2.2 其它已完成

- **shutdown=1 卡死**（`1dd539e`/`c776316`）：载荷忽略陈旧 shutdown，写还原顺序固化，日志加 PID/时间戳。
- **测试污染**（`6bd7223`）：autostart 测试隔离，`cargo test` 不再写真实注册表 / 启动真实服务。
- **HitTest 探针（只读、默认关闭）**：7 坐标 + 缓冲区 + 线程全排除，接口恒 `E_INVALIDARG`，已否决，代码保留作取证工具。
- **证据归档**（`a393df7`）：三轮冷开机/探针原始日志 + FINDINGS + 接口论证。

### 2.3 版本历史（近几版）

- **v1.5.5**：冷开机事件流被 TTB 独占——自动新连接重连夺回（本次）。
- **v1.5.4**：托盘箭头写 `Foreground`（后经用户澄清箭头一直是白的；针对箭头的
  “改画刷 `Color`”尝试已回滚，直接写 `Foreground` 的代码保留，属误报修复）。
- **v1.5.3**：宿主重复注入拿到失效连接——注入前读载荷活跃计数，存活则跳过。
- **v1.5.2**：第二次 `Advise` 必返 `E_UNEXPECTED`——删除重放，只订阅一次。
- **v1.5.1**：冷启动预热 + 重放方案，已被 v1.5.2 证伪并移除。

---

## 3. 关键决策（含理由，勿重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 夺回事件流的方式 | **新线程 + 新连接名**重新 `InitializeXamlDiagnosticsEx` | 同一连接 `Unadvise` 后再 `Advise` 必 `E_UNEXPECTED`；失败线程复用无效 |
| 旧连接处置 | 保持订阅、不退订 | 退订会破坏唯一有效通道；新连接接管后旧连接自然失效 |
| 自动触发策略 | 首连 +10 s 兜底；TTB 进程启动 +8 s；`ExplorerTAP` 加载 +2 s（取最早） | 覆盖冷开机先/后连接、用户晚开 TTB 两类时序；不做无限周期重连 |
| 触发可重复性 | TTB 每次启动都重新排程，**不设一次性锁死** | 实测：一次性逻辑下，TTB 晚开 4 分钟会再次抢走事件流 |
| 重连间隔 | 两次自动重连至少间隔 10 s | 进程/模块信号可能叠加，避免连开两条会话 |
| 手动哨兵 | 保留 `%LOCALAPPDATA%\ShadeBar\reconnect.req` | 非 TTB 的第三方工具极晚连接时的人工兜底 |
| 清树时机 | `g_clearOnAdd` 在 `Advise` **之前**置位，失败回滚 | 快照在 `Advise` 内同步投递，晚置位会把整棵树误清（实测 n=10 → n=1） |
| 元素树更新 | 同一条连接只订阅一次 + `Add/Remove` 增量 | 重放会破坏连接；重连只能开新连接 |
| 探针代码 | 保留、只读、默认关闭 | 有回归测试，不参与染色 |
| 托盘箭头 | **已回滚**，勿再查 | 用户澄清箭头一直是白的，问题在新开程序指示条 |
| 分区配色粒度 | 判定粒度只决定明暗，配色始终用 8 色矩阵 | 用户拍板 |
| 指示条状态检测 | UI 线程 250 ms（空闲 1 s）轮询 `ActualWidth` | `OnElementStateChanged` 对指示条不触发 |
| 正常/选中指示条 | 正常态改元素 `Fill`（共享画刷）；选中态改画刷 `Color` | 选中态 `Fill` 被 VisualState 压住 |
| 托盘字形定位 | 只认 `AdaptiveTextBlock` 的 `Base` 层 `InnerTextBlock` | 其余两层透明/强调覆盖 |
| 开机自启 | HKCU Run，不注册 schtasks | 零 UAC |
| 版本 / 验收 | 严格 SemVer；每组实机验收 | 用户长期规则 |

---

## 4. 未完成待办（用户没催，不要主动扩范围）

- 悬停高亮块（S-32 / S-34~S-37）：生产载荷尚未实现。
- 多显示器：1.x 只处理含任务栏的主屏。
- 崩溃恢复：`RegisterApplicationRestart` 仅在注释里，未接线。
- 日志：单文件追加 + 262144 字节轮转，未实现按天切分。
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v*.dll` 无清理（有意设计）。
- 自启项跟随“最后运行的那个 exe”：从 `target\` 启动过就指回 `target\`。
- 非 TTB 的第三方诊断消费者若在随影自动重连之后才连接，仍需手动哨兵兜底。

---

## 5. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| 冷开机新开程序指示条灰 | 单进程 XAML 诊断只有最后连接者收增量事件；TTB 后连接 | v1.5.5 新连接重连 + TTB 启动触发 |
| 重连后树被清空成 n=1 | 快照在 `Advise` 内同步投递，清树标志置位太晚 | 先置 `g_clearOnAdd`，失败回滚 |
| 自动重连只做一次后失效 | TTB 晚于 10 s 窗口启动 | 改为 TTB 每次启动都重新排程 |
| 新版本没修好旧问题 | explorer 同时驻留多版载荷，旧副本在生效 | 清场；日志格式与源码对不上 = 跑的不是这份代码 |
| 开机后指示条不着色（旧） | 首拍重放 + `Advise` 返 `E_UNEXPECTED` | v1.5.2 起只订阅一次 |
| 一部分指示条永远不亮（旧） | 宿主重复注入拿到失效连接 | v1.5.3 注入前探测存活 |
| 指示条状态切换色滞后 ~2 s | `OnElementStateChanged` 对指示条不触发 | 250 ms 轮询 `ActualWidth` |
| 托盘图标“改了没变” | 只有 `Base` 层 `InnerTextBlock` 可见 | 按父级名收 `Base` 层 |
| 托盘箭头“改了没变”（**误报**） | 箭头一直是白的 | 勿再查箭头 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完回读画刷 `Color` |
| 跑完 `cargo test` 机器着色全失效 | 测试启动真实服务 / 写真实注册表 | 已隔离（`6bd7223` 等） |
| 日志证据消失 | `payload.log` 262144 字节轮转 | 取证前先归档 |
| `HitTest` 恒 `E_INVALIDARG` | 此 Win11 诊断会话下不可用 | 已否决 |
| 软件在跑、托盘没图标 | 不响应 `TaskbarCreated`；`added` 失败不清零 | 广播补挂 + 失败归零 |

---

## 6. 环境、命令与限制

### 6.1 构建与测试

```bash
cd "<repo>"

# 载荷（C++，必须先编，宿主编译期内嵌它）
& 'C:\Program Files\Git\bin\bash.exe' payload/build.sh

# 宿主 / 测试 / 打包
cargo build --release
cargo test --release            # 235 项
powershell -ExecutionPolicy Bypass -File build.ps1

# 回归分析（均带 --self-test）
python tools/check_payload_sync.py
python tools/check_reconnect_logic.py --self-test
python tools/check_shutdown_logic.py --self-test
python tools/check_indicator_coldboot.py --self-test
```

`payload/build/`、`dist/verify/`、`dist/compare/` 是构建中间物，不进仓库。

### 6.2 本机硬限制（踩过）

- 工具调用起的 GUI 进程会被回收：启动与断言必须写在同一命令里。
- 截图必须先征得用户同意（用户长期规则）。
- 工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- Git 远端 `git@github.com:Yi-One1/ShadeBar.git`（SSH 可用）；`gh` CLI 已安装并登录（Yi-One1）。
- dist 版进程名 `随影ShadeBar_v<ver>`，`target` 版是 `shadebar-app`；单实例互斥会让后者启动即退出——测新代码前先按 PID 停旧实例。
- Bash 里不能调 PowerShell；Win+R 不支持 `type` 等 cmd 内置命令。

### 6.3 数据与路径

- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（49 项编号）
- 日志：`payload.log`（载荷，带毫秒时间戳，262144 字节轮转）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 手动重连哨兵：`%LOCALAPPDATA%\ShadeBar\reconnect.req`（创建即触发，载荷消费后删除）
- 探针触发：`%LOCALAPPDATA%\ShadeBar\probe.trigger`（只读探针，默认关闭）
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%

### 6.4 关键参数

- IPC `ShadeBarIpc` **80 字节**；`reserved[0]` 元素掩码、`[1]` 代际、`[2]` 探针触发、`[6]` 载荷活跃计数。
- `SettingKey as usize` 队列位序，新增项只能追加末尾（`SETTING_COUNT`=49）。
- 分项色：S-52/53 正常条、S-54/55 选中条、S-56/57 托盘、S-58/59 时间日期；S-60 正常条不透明度。
- 采样必须用屏幕 DC，不能用窗口 DC；体积约 672 KB；`windows` 锁 `=0.58.0`。

---

## 7. 用户偏好（长期有效）

- 语言：简体中文。
- 截图前必须询问并取得同意。
- 安装软件先问、由用户选路径；删除文件先列清单、说明影响、获授权后再删。
- 版本号严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- 每组实机验收，说「没问题」才继续；汇报带任务清单与进度。
- 不要替用户判断“这个不重要”；发现问题先说清根因，再给方案。
- 不要用“应该没问题”交差，必须有证据。

---

## 8. 证据索引

| 路径 | 内容 |
|---|---|
| `evidence/INVESTIGATION-STATUS.md` | 总览：根因、实证、修复闭环 |
| `evidence/coldboot-20261127/FINDINGS.md` | 冷开机根因首证（Add 停在 201、ind 恒为 1） |
| `evidence/coldboot-20261127/IVTS-feasibility.md` | 接口论证（含官方来源） |
| `evidence/coldboot-20261127/{before,after}/` | 首轮冷开机前后日志 |
| `evidence/probe*-coldboot-20261127/` | HitTest 探针两轮记录（已否决） |
| `evidence/tap-consume-20261127/` | 唯一消费者 A/B 实证（**本地保留，未入库**，含机器路径） |
