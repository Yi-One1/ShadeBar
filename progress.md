# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-22 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：**v1.3.1 已发布**（2026-11-22，GitHub Releases，tag `v1.3.1`）。
> 两组真机缺陷（指示条漏改 / 整片失效、指示条颜色不统一）已修并验收；
> 产物 `dist/随影ShadeBar_v1.3.1.exe` 678,400 B，测试 215 项全绿、五道门禁全绿。
> 下一版从 §4.1 末尾的发版链路接着走。

---

## 0. 新对话先做这三件事（五分钟上手）

1. `git log -8 --oneline` —— 最近提交应能看到 v1.3.1 的两组修复与文档收口。
2. `cargo test --release` —— 应得 **116 + 97 + 2 = 215 通过 / 1 ignored / 0 失败**；
   `cargo check --release --all-targets` 零警告。
3. 需要重新体验：双击 `1. 一键启动预览.bat`（重新编译并启动 `target\` 最新版，
   **不覆盖 dist**）。重新打包发版：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户明确要求——每组做完必须由他真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本 | **1.3.1**（PATCH：两组都是缺陷修复） |
| 版本唯一来源 | `Cargo.toml` → `[workspace.package] version` |
| 发布产物 | `dist/随影ShadeBar_v1.3.1.exe` |
| 体积 | 678,400 B（662.5 KB；NFR-01 软目标 400 KB / 硬上限 4096 KB） |
| SHA-256 | `17D3D53C45C3FBC7F7B7E40F49717B9AC3C40166098ED4E7D726DA3B169C7E73` |
| FileVersion | `1.3.1.0`（`--version` 输出 `随影 ShadeBar v1.3.1`） |
| 门禁 | 5/5 全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标）；verify 与 no-en 双产物同样全绿，双语资源差 8 KB（预算 40 KB） |
| git tag | `v1.3.1` → `d2bed34`（annotated，已推）；`v1.3.0` → `93c6402`（此前由网页建，已同步到本地） |
| GitHub Release | **v1.3.1 已发布**（2026-11-22）；附件名 `ShadeBar_v1.3.1.exe`，663 KB，哈希 `17d3d53c…` 与本地一致 |

发布资产命名：本地是 `随影ShadeBar_v1.3.1.exe`，**上传到 GitHub 后中文前缀会被去掉**，
变成 `ShadeBar_v1.3.1.exe`（v1.3.0 同样如此，属既成口径，不必纠正）。
构建产物命名规则：`<中文名><英文名>_v<版本>.exe`（`build.ps1` 的 `$productBase`，
2026-11-22 去掉了中文与英文之间多余的下划线）。

**dist/ 永远保留历史版本 exe**（用户明确要求）：`dist/` 里会有多个版本并存，
新版本只新增、不删除旧版本，便于回滚与对比；`dist/*.exe` 不进仓库，只走 Releases。

---

## 2. v1.3.1：两组真机缺陷（均已验收）

**触发**：用户真机反馈两条——①部分【正常态指示条】不改色（选中态 / 新开窗口等）；
②偶尔整片不着色（时间 / 日期 / 托盘 / 指示条全都不变），重启软件也无法完全恢复。

### 2.1 G1（`42ed030`）：指示条漏改与整片失效

| 根因 | 证据 | 修法 |
|---|---|---|
| 载荷只在订阅时拿到一次元素树，之后只认"带名字的 Add" | 打开新窗口时日志 **0 条 `[Add]`**，`ind` 不增 | 宿主重连 / 指示条 0 命中 / 兜底轮询时 `Unadvise+Advise` 全量重放 |
| 宿主重启不会重新订阅（DLL 驻留，`DllMain` 不再执行） | 重启软件后仍漏改 | 重放由载荷自己发起，不依赖重新注入 |
| 旧副本退位不退订；新副本 `Advise` 拿 `E_UNEXPECTED` 后放弃；初始化重试 60 次后线程退出 | `[Loop] 代际 N 已被 M 取代，退位` + `Advise hr=0x8000FFFF` | 退位前先退订；初始化无限重试（60 次后 10 s 退避） |

- 重放节奏自适应：用户 15 s 内有键鼠输入 → 5 s；空闲 → 30 s（重放实测中位 4.2 ms、
  p95 5.5 ms，后期 1.4–2.7 ms；5 s 节奏约 0.08% 单核，贴 NFR-03 的 0.1% 预算）。
- 回归检查：`python tools/check_payload_sync.py`，只读 `payload.log`，不改系统。
- 真机：新开设置窗口 `ind 7→8`；强杀宿主还原 25 个元素（选中态画刷回系统强调色），
  重启后着色恢复。

### 2.2 G2（`59f3251`）：正常态指示条共用一个画刷

- 用户选定 **B：结构统一**——`CreateInstance` 出一个 `SolidColorBrush`，逐条
  `SetProperty` 赋给正常态指示条的 `Fill`；换色只改该对象 `Color`，全部同刻变色。
- 选中态仍走"元素 `Fill` + 画刷 `Color`"双写 + 逐层染色（视觉状态会覆盖 `Fill`）。
- 树重放后 / 还原后丢弃画刷句柄（元素重建会让句柄失效），下一拍重建。
- 日志：`[Shared] b=.. color=.. assigned=N`、`[Ind] ... b=..`；检查脚本加 **C5**
  （同一拍内正常态画刷只能有 1 个句柄）。
- 真机：C1–C5 全 PASS（`2D7D4178` → 重放后 `2D90EDE8`，重建属预期）；强杀 `restored=22`。

### 2.3 本轮顺带修掉的工程问题

| 问题 | 处置 |
|---|---|
| `payload/build.sh` 写死别人机器的 MSVC `14.44` 路径 | 改为自动探测 VS / SDK（`MSVC_ROOT` / `SDK_ROOT` / `SDK_VER` 可覆盖）；依赖检查用 `dumpbin`；链接补 `user32` |
| `inject` 失败完全静默（只把 `injected_pid` 归零） | 记 `inject failed: …`（同错只记一次）与 `inject ok: explorer pid=` |
| `build.ps1` 里混进一行中文注释，违反它自己的"纯 ASCII"约定，Gate 0 直接拦死构建 | 注释改英文；`tools/verify-indicators.ps1` 需中文正则，改为带 UTF-8 BOM |
| `resources/app.manifest` 注释仍称"注册计划任务需 `ShellExecuteExW(runas)` 提权" | 改为事实口径：全生命周期不提权，自启写 HKCU Run |

**本轮不处理**：悬停高亮块（生产载荷零处 `BackgroundElement`）、副屏任务栏、
崩溃恢复、日志轮转。

---

## 3. 历史里程碑

| 版本 | 内容 |
|---|---|
| v1.0.0 | 只采样不落地。曾判定"任务栏改色不可用"，后被证伪（见 §6） |
| v1.1.0 | 改用官方 `IXamlDiagnostics` + `IVisualTreeService`，在 XAML UI 线程改画刷，跑通 |
| v1.1.2 | 常驻内存 92 MB → 8–15 MB |
| v1.1.3 | 补上指示条**选中态**（改画刷 `Color`，穿透 VisualState 优先级槽） |
| v1.2.0 | 判定粒度（分区 / 全任务栏）、诊断面板可折叠、颜色任意取（色板 + HEX + 系统调色板） |
| v1.3.0 | 设置窗口重做为四页签（720×560）+ 任务栏直控预览；可见设置收敛到 20 项（39 项编号保留） |
| **v1.3.1** | **指示条漏改 / 整片失效（G1）+ 指示条共用一个画刷（G2）** |

---

## 4. 待办

### 4.1 发布收尾（**已全部完成**，2026-11-22）
- [x] 版本升 **1.3.1**（`Cargo.toml` / `Cargo.lock` / README / CHANGELOG / 本文件）。
- [x] `build.ps1` 打包，五道门禁全绿，产物 `dist/随影ShadeBar_v1.3.1.exe`。
- [x] 仓库体检：无用户名 / 家目录 / 邮箱 / 令牌；`.gitignore` 覆盖 `target`、`dist/*.exe`、`.workbuddy`、载荷中间物。
- [x] `git push origin main`（`93c6402 → d2bed34`）。
- [x] `git tag -a v1.3.1` 并推送（指向 `d2bed34`）。
- [x] GitHub Releases 发布 `v1.3.1`，附件 `ShadeBar_v1.3.1.exe`（哈希与本地一致）。

下一版发版照抄这条链路：`build.ps1` → 提交 → `git tag -a vX.Y.Z` → push main+tag →
网页建 Release（正文写进 `docs/release-notes-vX.Y.Z.md`）。本机无 `gh` CLI、无 token，
Release 只能网页手动建。

### 4.2 长期缺口（用户没催，不要主动扩范围）
- 悬停高亮块（S-34~S-37）：**生产载荷尚未实现**（`payload/` 零处 `BackgroundElement`），
  需先实现再做 60 s 稳定性验证，通过后才能放回 UI。
- 多显示器（副屏任务栏元素契约未用探针确认）。
- 崩溃恢复（`RegisterApplicationRestart` 仅出现在注释里，未接线）。
- 日志轮转（7 天轮转未实现，当前按天切分）。
- 载荷副本累积：`%LOCALAPPDATA%\ShadeBar\` 下每次载荷内容变化留一份
  `ShadeBarTap_v<ver>_<指纹>.dll`，目前无清理机制（防 explorer 复用旧模块，属有意设计）。

---

## 5. 关键决策（含理由，别重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 指示条两态 | **目标色一致、写入路径不同** | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color`；正常态画刷被 29 处共享，不能逐层染 |
| 正常态指示条一致性 | **结构统一：共用一个画刷**（v1.3.1 G2） | 用户要求"统一、同时修改"；逐条写只能保证最终一致，共享对象才能同刻变 |
| 元素树更新 | **重放（Unadvise+Advise）而不是增量** | XAML 诊断接口没有遍历整棵树的 API，新窗口也不产生可用的 Add 事件 |
| 重放节奏 | 有输入 5 s / 空闲 30 s | 5 s 重放约 4 ms，恒定 5 s 会吃掉 NFR-03 的 0.1% CPU 预算 |
| 颜色选择 | 色板 + HEX + 系统调色板 | 用户三选一拍板；端色 alpha 恒 `0xFF` |
| 改色联动 | 改 S-14/S-15 时自动把 S-13 切「自定义」并提示 | 双门控会让选色完全无效；用户在 A/B 方案中选 A（自动切） |
| 图标 | 九套预览 → 选 A·对分；三色；去白点 | 用户先要彩色、再要自由发挥，最终仍选 A；但要求轮廓清晰、不要点 |
| 设置布局 | 四页签 + 任务栏直控预览，无左侧导航 / 无分档 | 用户从四套预览选定方案 1；原痛点是"排版混乱、显示区与交互区不一致" |
| 打包产物 | 新 exe 只新增，**不删除旧 exe** | 用户要求保留历史版本，便于回滚与对比 |
| 三区顺序 | **图标 → 托盘 → 时钟** | 用户明确纠正；预览、卡片、命中三处同序 |
| 验收方式 | 一律用 `1. 一键启动预览.bat`，**不覆盖 dist** | 用户明确要求；dist 只在发布时更新 |
| 版本 | 修 bug = PATCH，新功能 = MINOR（严格 SemVer 2.0.0） | 用户长期规则 |

---

## 6. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| "新版本没修好旧问题" | explorer 同时驻留多版载荷，**旧副本在生效** | 注入类测试收尾必须清场；**日志格式与源码对不上 = 跑的不是这份代码** |
| 一部分指示条永久不改色 | 订阅时只给一次全量树，之后只认带名字的 Add；新窗口 0 条 Add | v1.3.1：定时 / 触发式全量重放 |
| 整片不着色且只有重启 explorer 能恢复 | 旧副本退位不退订 → 新副本 `Advise` 拿 `E_UNEXPECTED` 后放弃；初始化重试耗尽后线程退出但 DLL 仍驻留 | v1.3.1：退位前退订 + 无限重试 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完**回读**画刷 `Color`；`tools/verify-indicators.ps1` 做断言 |
| `build.ps1` 直接 `BUILD FAILED: ps1 encoding guard` | 文件里混入中文注释，而脚本要求纯 ASCII（PS 5.1 无 BOM 时会按 ANSI 解析整份脚本） | 脚本注释只用 ASCII；确实需要中文正则的脚本存成 UTF-8 **带 BOM** |
| 图标"模糊" | 白半融进白桌面、黑半融进深色栏，细浅灰边撑不住 | 轮廓改 `#595959`、16 px 框 1 px / 缝 2 px |
| exe 图标改了没变 | Windows 图标缓存 | `ie4uinit -show` + 重启 explorer；用二进制搜 PNG 签名确认资源已更新 |
| 界面能看见但点不中 | 绘制/命中两套坐标（内容 vs 客户区） | 统一坐标语义 + 纯 frame 函数；**用变异测试证明断言有效** |
| 输入 `"颜颜"` 崩溃 | 自写 `parse_hex` 缺 ASCII 守卫，在非 char 边界切片 | 收敛到 `shadebar_core::color::Rgba::parse_hex` |
| 指示条漏改（v1.2 已修） | 正常↔选中不产生 Add 事件，`OnElementStateChanged` 空实现不置脏 | 置脏 + 250 ms 节流 |
| 还原留色 | 应用逐层染、还原只改一层 | 还原与应用对称（`SetAllBrushColorsOfProp`） |

---

## 7. 环境、命令与限制

### 7.1 构建与测试
```bash
cd "<repo>"

# 载荷（C++，必须先编，宿主编译期内嵌它）
cd payload && bash build.sh && cd ..

# 宿主（release，增量约 7–10 s）
cargo build --release

# 测试（215 通过 / 1 ignored）
cargo test --release

# 打包发版（五道门禁 + verify/no-en 双产物对比）
powershell -ExecutionPolicy Bypass -File build.ps1

# 一键启动预览（验收用；先 build 再启动 target 最新版，不动 dist）
cmd //c "1. 一键启动预览.bat"
```

`dist/verify/`、`dist/compare/` 是门禁用的对照产物，每次打包都会重新生成，
**不进仓库**，可在打包后删掉。

### 7.2 本机环境的硬限制（踩过）
- **GUI 进程活不过一次命令调用**：`Start-Process` / `&` 起的随影会在工具调用结束时被收掉。
  需要真机观察时让用户自己双击 bat。
- **PowerShell 的 `Add-Type` 被安全策略拦截**（工具调用内）：截图与
  `resources/gen-app-ico.ps1` 都跑不了，所以 ICO 改用纯 Python 标准库生成。
  （`build.ps1` 里 Gate 5 的 `Add-Type` 在它自己的 powershell 子进程里可正常执行。）
- **Bash 里不要调 PowerShell**（会被拦）；用 PowerShell 工具或纯 Python。
- **截图必须先征得用户同意**，且同意有时限，过期需重新问。
- 本机工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- 本机**没有 `gh` CLI**，GitHub Release 只能网页手动建；`git push` 走 SSH（已可用）。

### 7.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（39 项编号；缺失尾部按默认补，不判损坏）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%（`CLOCK_SPAN / TRAY_SPAN / ICONS_SPAN_MAX`）

### 7.4 关键技术参数（改代码前必看）
- IPC 结构 `ShadeBarIpc` 80 字节；`reserved[0]` 是元素掩码（TIME/DATE/TRAY/INDICATOR）。
- 队列位序：`SettingKey as usize`，**新增项只能追加在末尾**（`SETTING_COUNT` 当前 39）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块；注入失败要收手，不要反复重试。
- 采样必须用屏幕 DC，不能用窗口 DC（TranslucentTB 模糊结果拿不到）。
- 体积：单 exe 约 660 KB；不得引入新 crate（windows 锁 `=0.58.0`）。

---

## 8. 用户偏好（长期有效，别违反）

- 语言：简体中文。
- **截图**：任何截图前必须询问并取得同意，同意有时限，过期需重新问。
- **安装应用**：必须先问、由用户选路径。
- **删除文件**：先列清单、说明影响，得到授权后才删。
- **版本号**：严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- **验收**：每组必须真机验收；他说「没问题」才继续。汇报要带任务清单与进度。
- 不要替他做"这个不重要"的判断；发现问题先说清根因，再给方案。

---

## 9. 新对话冒烟验证（3 分钟）

```bash
# 1) 清场：确保没有残留实例
powershell -c "Get-Process *ShadeBar* -ErrorAction SilentlyContinue | Stop-Process -Force"

# 2) 确认 explorer 里只有当前版本载荷（多份 = 旧代码可能在生效）
#    日志若出现源码里已删除的字符串，说明跑的不是这份代码

# 3) 构建 + 启动
cargo build --release
cmd //c "1. 一键启动预览.bat"

# 4) 看三处：时间/日期颜色是否跟随背景、所有指示条是否同色、托盘图标是否为 A 图标
# 5) 日志侧回归：python tools/check_payload_sync.py
```
