# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-26 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：**v1.5.2 已发布**（2026-11-26，GitHub Releases，tag `v1.5.2`）——
> 彻底移除自动重放，修复开机后整条着色链路（时间/日期/托盘/指示条）失效；
> 待用户下一次冷开机做最终确认。

---

## 0. 新对话先做这三件事（五分钟上手）

1. `git log -8 --oneline` —— 最新应有 `fix(payload): 修复冷启动指示条不着色` 与发布提交。
2. `cargo test --release` —— 应全绿；`cargo check --release --all-targets` 零警告。
3. 需要重新体验：双击 `1. 一键启动预览.bat`（重新编译并启动 `target\` 最新版，
   **不覆盖 dist**）。重新打包：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户明确要求——每组做完必须由他真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本 | **1.5.2**（PATCH：移除自动重放，修复冷启动整条链路失效） |
| 版本唯一来源 | `Cargo.toml` → `[workspace.package] version`（资源 / 关于页 / 文件名自动跟随） |
| 发布产物 | `dist/随影ShadeBar_v1.5.2.exe`（687,616 B，671.5 KB） |
| SHA-256 | `24F66AF91F90655DEC932A814B162DC434707AA27AD497FF3568E64CC33EA982` |
| 门禁 | 5/5（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标） |
| git tag | `v1.5.2` → `0b99b65`（annotated，已推）；历史：`v1.5.1` → `018d135`、`v1.5.0` → `4484ce7` |
| GitHub Release | **v1.5.2 已发布**（2026-11-26）：https://github.com/Yi-One1/ShadeBar/releases/tag/v1.5.2 ；附件 `ShadeBar_v1.5.2.exe` |

`dist/` **永远保留历史版本 exe**（用户要求，便于回滚）；exe 不进仓库，只走 Releases。

---

## 2. 已完成事项

### 2.1 v1.5.2（2026-11-26，已发布）

| 内容 | 验证 |
|---|---|
| **冷启动整条链路失效**：第二次 `Advise` 必返 `E_UNEXPECTED`，先退订会拆掉唯一有效订阅，连时间/日期一起失效。彻底删除 `ReplayTree`、全部触发点、会话重建、预热与退避；只订阅一次，依赖 `Add/Remove` 增量 | explorer 重启真机 C1–C7 全 PASS、`[Replay]` 0 次；新开计算器指示条 4→5 并入树着色 |

### 2.2 v1.5.1（2026-11-25，已发布）

- `9e4ea24`：冷启动指示条不着色（预热 + 退避 + 会话重建）。**已被 v1.5.2 取代**：
  该方案的重放本身就是整条链路失效的根因。

### 2.3 v1.5.0（2026-11-24，已发布）

- `8203671` G1：亮/暗端预览语义修正 + 时间日期取色浮层不越界。
- `7c49805` G2：配色与判定粒度解耦，统一模式也按四项独立取色。
- `6a70087` G3：指示条状态切换改为 UI 线程 250 ms 轮询 `ActualWidth`，延迟 2 s → 0.14–0.48 s。
- 可见设置 30 → 28 项；配置仍为 49 项编号。

---

## 3. 关键决策（含理由，别重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 分区配色粒度 | **8 色 = 4 内容 × 2 背景态** + 独立不透明度 | 用户拍板；4 项天然分属三区 |
| 统一 / 分区关系 | **解耦**：判定粒度只决定明暗判断方式，配色始终用 8 色矩阵 | 用户拍板 |
| S-14 / S-15 | 仅保留配置兼容，不参与取色，UI 收起 | 解耦后无消费方；删编号会破坏旧配置 |
| 亮端 / 暗端语义 | **亮端 = 浅色文字 → 深色任务栏**，预览与引擎一致 | 需求 S-14/S-15 定义 |
| 指示条状态检测 | UI 线程每 250 ms（空闲 1 s）轮询 `ActualWidth` | `OnElementStateChanged` 对指示条不触发 |
| 正常指示条透明度 | S-60（默认 45%），两种模式都生效 | 保留系统半透明手感且可调 |
| 单击/双击共存 | 双击非「无操作」时单击挂起 `GetDoubleClickTime` | legacy 托盘先送单击 |
| S-22 语义 | **任务栏高度百分比，上下对称内缩** | 引擎本来就这么算 |
| 指示条两态 | 颜色可不同，写入路径不同 | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color` |
| 正常态一致性 | 所有正常态共用一个画刷 | 换色同一刻生效 |
| 托盘字号定位 | 只认 `AdaptiveTextBlock` 的 `Base` 层 `InnerTextBlock` | 其余两层透明/强调覆盖 |
| 元素树更新 | **只订阅一次** + `Add/Remove` 增量；**禁止任何重放** | 第二次 `Advise` 必返 `E_UNEXPECTED` 并破坏连接；实测新窗口会送来 Add |
| 托盘补挂 | 响应 `TaskbarCreated`，先 MODIFY 探测再 ADD | 无条件 `NIM_ADD` 会撞 uID 返回 `E_FAIL` |
| 开机自启 | HKCU Run，不注册 schtasks | 零 UAC |
| 设置布局 | 四页签 + 任务栏直控预览；配色卡始终 4×2 矩阵 | 用户选定方案 1 |
| 版本 | 严格 SemVer 2.0.0：修 bug = PATCH，新功能 = MINOR | 用户长期规则 |
| 验收方式 | 一律 `1. 一键启动预览.bat`，不覆盖 dist | 用户明确要求 |
| 打包产物 | 新 exe 只新增，不删旧 exe | 便于回滚与对比 |

---

## 4. 未完成的待办

### 4.1 v1.5.2 发布收尾
- [x] 版本 1.5.2（`Cargo.toml` / `Cargo.lock` / README / CHANGELOG / 本文件）。
- [x] `cargo test --release` 全绿（133 + 97 + 2，另 1 ignored）+ `build.ps1` 五道门禁全绿。
- [x] 提交文档与版本变更、打 tag `v1.5.2`、发布 GitHub Release 与附件。

### 4.2 用户回归验证（v1.5.2）
- [ ] 用 `dist/随影ShadeBar_v1.5.2.exe` 作为开机自启，**冷开机**确认时间/日期/托盘/指示条全部着色。
- [ ] 若仍有异常，取 `%LOCALAPPDATA%\ShadeBar\payload.log`（带时间戳）。

### 4.3 长期缺口（用户没催，不要主动扩范围）
- 悬停高亮块（S-32 / S-34~S-37）：生产载荷尚未实现。
- 多显示器（副屏任务栏元素契约未用探针确认）。
- 崩溃恢复（`RegisterApplicationRestart` 仅出现在注释里，未接线）。
- 日志轮转（7 天轮转未实现，当前按天切分）。
- 载荷副本累积：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll` 无清理机制（有意设计）。
- 自启项跟随"最后运行的那个 exe"：从 `target\` 启动过就会指回 `target\`。

---

## 5. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| 新版本没修好旧问题 | explorer 同时驻留多版载荷，旧副本在生效 | 注入测试收尾必须清场；日志格式与源码对不上 = 跑的不是这份代码 |
| **开机后指示条不着色** | 注入早于任务栏 XAML 建完；首拍 `ind=0` 立即重放，`Advise` 返回 `E_UNEXPECTED`，订阅永久失效 | v1.5.1：预热 10 s + 树静止 2 s + 退避 + 连败 3 次重建会话 |
| 指示条选中/正常切换色滞后 ~2 s | `OnElementStateChanged` 对指示条不触发 | v1.5：UI 线程 250 ms 轮询 `ActualWidth` 置脏置急 |
| 托盘图标"改了没变" | 按 `AdaptiveTextBlock` 匹配，只有 `Base` 的 `InnerTextBlock` 可见 | 按父级名收 `Base` 层子 TextBlock |
| 指示条两态颜色无法区分 | 载荷选中态误用 `indicatorNormalColor` | 改读 `indicatorSelectedColor` |
| "改了颜色却没变化" | 纯黑纯白策略下，白色目标与系统默认同色 | 先用红/绿等辨识色复现 |
| 滑条只在某种模式能拖 | 命中检测写在 `per_region()` 分支内 | 命中检测覆盖所有显示该控件的模式 |
| 取色浮层最右列点不中 | 浮层按锚点向右展开，超出窗口被裁 | x 夹紧在窗口内 |
| 语言表键值重复 | 追加脚本跑两次；`required_keys` 只查存在不查唯一 | 改语言表后检查重复键 |
| 软件在跑、托盘没图标 | 不响应 `TaskbarCreated`；`added` 失败不清零 | 广播补挂 + 失败归零 |
| 重复 `NIM_ADD` 返回 `E_FAIL` | 图标已存在时撞同一 uID | 先 `NIM_MODIFY` 探测 |
| **开机后全部元素不着色** | 第二次 `Advise` 返回 `E_UNEXPECTED`；先退订会拆掉唯一订阅，建新会话也救不回 | v1.5.2：彻底移除重放，只订阅一次 + Add/Remove 增量 |
| 新窗口指示条不出现 | v1.3.1 误判为"没有 Add 事件" | 实测新开应用会送来 Add；重放反而破坏连接 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完回读画刷 `Color` |
| `build.ps1` 报 ps1 encoding guard | 脚本要求纯 ASCII，混入中文注释 | 注释只用 ASCII；需中文正则的脚本存 UTF-8 带 BOM |
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
```

`payload/build/`、`dist/verify/`、`dist/compare/` 是构建中间物，不进仓库。

### 6.2 本机硬限制（踩过）
- 工具调用起的 GUI 进程会被回收：启动与断言必须写在同一命令里。
- PowerShell `Add-Type` 在工具调用内被拦；截图/断言用 Python + `ctypes` + Pillow。
- 截图必须先征得用户同意，同意有时限。
- 工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- 本机**没有 `gh` CLI**：Release 走 GitHub REST API（环境有 `GH_TOKEN`）；`git push` 走 SSH。
- dist 版进程名是 `随影ShadeBar_v<ver>`，`target` 版是 `shadebar-app`；单实例互斥会让后者
  启动即退出——测新代码前先按 PID 停旧实例。

### 6.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（**49 项编号**；缺失尾部按默认补）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷，带时间戳）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%

### 6.4 关键参数（改代码前必看）
- IPC 结构 `ShadeBarIpc` 80 字节；`reserved[0]` 是元素掩码（TIME/DATE/TRAY/INDICATOR）。
- 队列位序：`SettingKey as usize`，**新增项只能追加在末尾**（`SETTING_COUNT` 当前 49）。
- 分项色：S-52/53 正常条、S-54/55 选中条、S-56/57 托盘、S-58/59 时间日期
  （成对为「背景偏暗 / 偏亮」），S-60 正常条不透明度（0–100，默认 45）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块。
- 采样必须用屏幕 DC，不能用窗口 DC。
- 体积：单 exe 约 674 KB；不得引入新 crate（windows 锁 `=0.58.0`）。

---

## 7. 用户偏好（长期有效，别违反）

- 语言：简体中文。
- 截图前必须询问并取得同意，同意有时限。
- 安装应用必须先问、由用户选路径。
- 删除文件先列清单、说明影响，得到授权后才删。
- 版本号严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- 每组必须真机验收，他说「没问题」才继续；汇报要带任务清单与进度。
- 不要替他做"这个不重要"的判断；发现问题先说清根因，再给方案。

---

## 8. 新对话冒烟验证（3 分钟）

```bash
# 1) 清场
powershell -c "Get-Process *ShadeBar*,shadebar-app -ErrorAction SilentlyContinue | Stop-Process -Force"

# 2) 构建 + 启动（启动与断言同一命令）
cargo build --release

# 3) 测试与载荷回归
cargo test --release
python tools/check_payload_sync.py

# 4) 看四处：时间/日期、托盘文字图标、正常/选中指示条是否按设置着色
```
