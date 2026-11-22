# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-22 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：代码到 **v1.3.2**，产物已打包 `dist/随影ShadeBar_v1.3.2.exe`
> （679,424 B，SHA-256 `AC79C1EA…C8B726`），五道门禁全绿、测试 **216** 项全绿。
> **只剩发布动作**：`git push` + 打 tag `v1.3.2` + 网页建 GitHub Release
> （正文用 `docs/release-notes-v1.3.2.md`；本机无 `gh` CLI，只能手动建）。

---

## 0. 新对话先做这三件事（五分钟上手）

1. `git log -8 --oneline` —— 最近提交应能看到 v1.3.2 的托盘修复与文档收口。
2. `cargo test --release` —— 应得 **117 + 97 + 2 = 216 通过 / 1 ignored / 0 失败**；
   `cargo check --release --all-targets` 零警告。
3. 需要重新体验：双击 `1. 一键启动预览.bat`（重新编译并启动 `target\` 最新版，
   **不覆盖 dist**）。重新打包发版：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户明确要求——每组做完必须由他真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本 | **1.3.2**（PATCH：托盘图标缺陷修复） |
| 版本唯一来源 | `Cargo.toml` → `[workspace.package] version` |
| 发布产物 | `dist/随影ShadeBar_v1.3.2.exe` |
| 体积 | 679,424 B（663.5 KB；NFR-01 软目标 400 KB / 硬上限 4096 KB） |
| SHA-256 | `AC79C1EAE3445BAAD0422ABDD7E65D83A95193C94B58667856C91A2470C8B726` |
| FileVersion | `1.3.2.0`（`--version` 输出 `随影 ShadeBar v1.3.2`） |
| 门禁 | 5/5 全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标）；verify 769,024 B、no-en 671,232 B，双语差 8 KB（预算 40 KB） |
| git tag | `v1.3.2` 待打（`v1.3.1` → `d2bed34`、`v1.3.0` → `93c6402` 均已存在） |
| GitHub Release | v1.3.0 / v1.3.1 已发布；**v1.3.2 待发布** |

发布资产命名：本地是 `随影ShadeBar_v1.3.2.exe`，**上传到 GitHub 后中文前缀会被去掉**，
线上叫 `ShadeBar_v1.3.2.exe`（v1.3.0 / v1.3.1 都如此，属既成口径，不必纠正）。

**dist/ 永远保留历史版本 exe**（用户明确要求）：新版本只新增、不删旧版本，
便于回滚与对比；`dist/*.exe` 不进仓库，只走 Releases。

---

## 2. v1.3.2：托盘图标在 explorer 重启后消失

**触发**：用户真机反馈——软件在运行，但系统托盘（含隐藏区）里找不到图标，
只能重启软件。

### 2.1 根因（两条叠加，均有日志实证）

| # | 根因 | 证据 |
|---|---|---|
| 1 | 全项目**没有处理 `TaskbarCreated` 广播**。托盘图标挂在 explorer 上，资源管理器重启会清空所有图标；别的程序收到广播会重挂，随影不会 | explorer 08:30 重启、随影 07:36 已在跑；日志 `inject ok: explorer pid=9852` → `pid=33048`，着色链路自己重连了，托盘没有 |
| 2 | 心跳补挂条件 `first = !is_added()`，而 `added` **失败时从不清零**；`refresh` 里还有"已挂载 + 外观没变 → return"的短路 | 图标丢失后既不发 `NIM_ADD` 也不发 `NIM_MODIFY`，永久静默 |

### 2.2 修法

- `window.rs`：`RegisterWindowMessageW("TaskbarCreated")` + wndproc 分支，收到后调
  `tray.reattach()`，日志记 `tray reattach after TaskbarCreated: added=<bool>`。
- `tray.rs`：`reattach()` 先按"图标仍在"发 `NIM_MODIFY` 探测，**失败才** `NIM_ADD` 重挂。
  实测直接 `NIM_ADD` 会撞上已存在的 uID 返回 `E_FAIL (0x80004005)`，把状态误判成
  未挂载，心跳反而每秒重试失败。
- `tray.rs`：`refresh` 失败即 `added = false` + 清缓存键。
- `tray.rs`：短路条件抽成纯函数 `needs_shell_call(first, added, look_unchanged)` 并加单测。
- 失败日志带 `GetLastError()`，每进程最多 3 行（1 Hz 重试不刷屏）。

### 2.3 验证

- `cargo test --release`：**117 + 97 + 2 = 216 通过 / 1 ignored / 0 失败**，零警告。
- 真机：杀掉 explorer 并重建 → `tray reattach after TaskbarCreated: added=true`，图标自动回来。
- 伪造广播（图标仍在）→ `added=true`，不误判、不刷失败日志。
- 顺带修掉：开机自启项曾指向 `target\...\deps\shadebar_app-<hash>.exe`（`cargo clean`
  后会失效），现已改指 `dist\随影ShadeBar_v1.3.2.exe`。

---

## 3. v1.3.1：指示条漏改 / 整片失效 / 颜色不统一（已发布）

**触发**：①部分【正常态指示条】不改色（选中态 / 新开窗口等）；②偶尔整片不着色，
重启软件也无法完全恢复。

### 3.1 G1（`42ed030`）：指示条漏改与整片失效

| 根因 | 证据 | 修法 |
|---|---|---|
| 载荷只在订阅时拿到一次元素树，之后只认"带名字的 Add" | 打开新窗口时日志 **0 条 `[Add]`**，`ind` 不增 | 宿主重连 / 指示条 0 命中 / 兜底轮询时 `Unadvise+Advise` 全量重放 |
| 宿主重启不会重新订阅（DLL 驻留，`DllMain` 不再执行） | 重启软件后仍漏改 | 重放由载荷自己发起，不依赖重新注入 |
| 旧副本退位不退订；新副本 `Advise` 拿 `E_UNEXPECTED` 后放弃；初始化重试 60 次后线程退出 | `[Loop] 代际 N 已被 M 取代，退位` + `Advise hr=0x8000FFFF` | 退位前先退订；初始化无限重试（60 次后 10 s 退避） |

- 重放节奏自适应：15 s 内有键鼠输入 → 5 s；空闲 → 30 s（重放中位 4.2 ms / p95 5.5 ms）。
- 回归检查：`python tools/check_payload_sync.py`（C1–C5），只读 `payload.log`。

### 3.2 G2（`59f3251`）：正常态指示条共用一个画刷

- `CreateInstance` 出一个 `SolidColorBrush`，逐条赋给正常态指示条的 `Fill`；换色只改
  该对象 `Color`，全部同刻变色。选中态仍走"元素 `Fill` + 画刷 `Color`"双写。
- 树重放后 / 还原后丢弃画刷句柄（元素重建会让句柄失效）。

### 3.3 顺带修掉的工程问题

| 问题 | 处置 |
|---|---|
| `payload/build.sh` 写死别人机器的 MSVC `14.44` 路径 | 自动探测 VS / SDK；依赖检查用 `dumpbin`；链接补 `user32` |
| `inject` 失败完全静默 | 记 `inject failed: …` / `inject ok: explorer pid=` |
| `build.ps1` 混入中文注释，违反纯 ASCII 约定，Gate 0 拦死构建 | 注释改英文；`tools/verify-indicators.ps1` 需中文正则，改存 UTF-8 带 BOM |
| `resources/app.manifest` 注释仍称注册计划任务需 runas | 改为事实口径：全生命周期不提权，自启写 HKCU Run |

---

## 4. 历史里程碑

| 版本 | 内容 |
|---|---|
| v1.0.0 | 只采样不落地。曾判定"任务栏改色不可用"，后被证伪（见 §7） |
| v1.1.0 | 改用官方 `IXamlDiagnostics` + `IVisualTreeService`，在 XAML UI 线程改画刷，跑通 |
| v1.1.2 | 常驻内存 92 MB → 8–15 MB |
| v1.1.3 | 补上指示条**选中态**（改画刷 `Color`，穿透 VisualState 优先级槽） |
| v1.2.0 | 判定粒度（分区 / 全任务栏）、诊断面板可折叠、颜色任意取 |
| v1.3.0 | 设置窗口重做为四页签（720×560）+ 任务栏直控预览；可见设置收敛到 20 项 |
| v1.3.1 | 指示条漏改 / 整片失效（树重放 + 退位退订 + 无限重试）+ 指示条共用一个画刷 |
| **v1.3.2** | **托盘图标在 explorer 重启后自愈（TaskbarCreated + MODIFY→ADD 探测重挂）** |

---

## 5. 待办

### 5.1 发布收尾（v1.3.2）
- [x] 版本升 **1.3.2**（`Cargo.toml` / `Cargo.lock` / README / CHANGELOG / 本文件）。
- [x] `build.ps1` 打包，五道门禁全绿，产物 `dist/随影ShadeBar_v1.3.2.exe`。
- [x] 仓库体检：无用户名 / 家目录 / 邮箱 / 令牌；`.gitignore` 覆盖 `target`、`dist/`、`.workbuddy`、载荷中间物。
- [x] 自启项改指 dist 产物。
- [ ] `git push origin main`。
- [ ] `git tag -a v1.3.2` 并推送（指向发布代码状态）。
- [ ] 网页建 GitHub Release `v1.3.2`，附件 `随影ShadeBar_v1.3.2.exe`，
      正文用 `docs/release-notes-v1.3.2.md`。

下一版照抄：`build.ps1` → 提交 → `git tag -a vX.Y.Z` → push main+tag → 网页建 Release。

### 5.2 长期缺口（用户没催，不要主动扩范围）
- 悬停高亮块（S-34~S-37）：**生产载荷尚未实现**（`payload/` 零处 `BackgroundElement`），
  需先实现再做 60 s 稳定性验证，通过后才能放回 UI。
- 多显示器（副屏任务栏元素契约未用探针确认）。
- 崩溃恢复（`RegisterApplicationRestart` 仅出现在注释里，未接线）。
- 日志轮转（7 天轮转未实现，当前按天切分）。
- 载荷副本累积：`%LOCALAPPDATA%\ShadeBar\` 下每次载荷内容变化留一份
  `ShadeBarTap_v<ver>_<指纹>.dll`，无清理机制（防 explorer 复用旧模块，属有意设计）。
- 自启项跟随"最后运行的那个 exe"：从 `target\` 启动过就会指回 `target\`；
  想让自启稳定，最后用 `dist\` 里的 exe 启动一次即可。

---

## 6. 关键决策（含理由，别重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 指示条两态 | **目标色一致、写入路径不同** | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color`；正常态画刷被 29 处共享，不能逐层染 |
| 正常态指示条一致性 | **结构统一：共用一个画刷**（v1.3.1 G2） | 用户要求"统一、同时修改"；逐条写只能保证最终一致 |
| 元素树更新 | **重放（Unadvise+Advise）而不是增量** | XAML 诊断接口没有遍历整棵树的 API，新窗口也不产生可用的 Add 事件 |
| 重放节奏 | 有输入 5 s / 空闲 30 s | 5 s 重放约 4 ms，恒定 5 s 会吃掉 NFR-03 的 0.1% CPU 预算 |
| 托盘补挂 | **响应 `TaskbarCreated`，先 MODIFY 探测再 ADD** | 窗口不会自己知道 explorer 换了；无条件 `NIM_ADD` 会撞 uID 返回 `E_FAIL` |
| 颜色选择 | 色板 + HEX + 系统调色板 | 用户三选一拍板；端色 alpha 恒 `0xFF` |
| 图标 | 九套预览 → 选 A·对分；三色；去白点 | 用户要求轮廓清晰、不要点 |
| 设置布局 | 四页签 + 任务栏直控预览，无左侧导航 / 无分档 | 用户从四套预览选定方案 1 |
| 打包产物 | 新 exe 只新增，**不删除旧 exe** | 用户要求保留历史版本，便于回滚与对比 |
| 三区顺序 | **图标 → 托盘 → 时钟** | 用户明确纠正；预览、卡片、命中三处同序 |
| 验收方式 | 一律用 `1. 一键启动预览.bat`，**不覆盖 dist** | 用户明确要求；dist 只在发布时更新 |
| 版本 | 修 bug = PATCH，新功能 = MINOR（严格 SemVer 2.0.0） | 用户长期规则 |

---

## 7. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| "新版本没修好旧问题" | explorer 同时驻留多版载荷，**旧副本在生效** | 注入类测试收尾必须清场；**日志格式与源码对不上 = 跑的不是这份代码** |
| 软件在跑、托盘没图标 | 不响应 `TaskbarCreated`；`added` 失败不清零 + 缓存短路，永久静默 | v1.3.2：广播补挂 + 失败归零 + 纯函数单测 |
| 重复 `NIM_ADD` 返回 `E_FAIL` | 图标已存在时再 `NIM_ADD` 撞同一 uID | 先 `NIM_MODIFY` 探测，失败才 `NIM_ADD` |
| 一部分指示条永久不改色 | 订阅时只给一次全量树，之后只认带名字的 Add | v1.3.1：定时 / 触发式全量重放 |
| 整片不着色且只有重启 explorer 能恢复 | 旧副本退位不退订 → 新副本 `Advise` 拿 `E_UNEXPECTED`；重试耗尽后线程退出但 DLL 仍驻留 | v1.3.1：退位前退订 + 无限重试 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完**回读**画刷 `Color` |
| `build.ps1` 直接 `BUILD FAILED: ps1 encoding guard` | 文件里混入中文注释，而脚本要求纯 ASCII | 脚本注释只用 ASCII；需中文正则的脚本存 UTF-8 **带 BOM** |
| 图标"模糊" | 白半融进白桌面、黑半融进深色栏，细浅灰边撑不住 | 轮廓改 `#595959`、16 px 框 1 px / 缝 2 px |
| exe 图标改了没变 | Windows 图标缓存 | `ie4uinit -show` + 重启 explorer；用二进制搜 PNG 签名确认资源已更新 |
| 界面能看见但点不中 | 绘制/命中两套坐标 | 统一坐标语义 + 纯 frame 函数；**用变异测试证明断言有效** |
| 输入 `"颜颜"` 崩溃 | 自写 `parse_hex` 缺 ASCII 守卫 | 收敛到 `shadebar_core::color::Rgba::parse_hex` |
| 还原留色 | 应用逐层染、还原只改一层 | 还原与应用对称（`SetAllBrushColorsOfProp`） |

---

## 8. 环境、命令与限制

### 8.1 构建与测试
```bash
cd "<repo>"

# 载荷（C++，必须先编，宿主编译期内嵌它）
cd payload && bash build.sh && cd ..

# 宿主（release，增量约 7–10 s）
cargo build --release

# 测试（216 通过 / 1 ignored）
cargo test --release

# 打包发版（五道门禁 + verify/no-en 双产物对比）
powershell -ExecutionPolicy Bypass -File build.ps1

# 一键启动预览（验收用；先 build 再启动 target 最新版，不动 dist）
cmd //c "1. 一键启动预览.bat"
```

`dist/verify/`、`dist/compare/` 是门禁用的对照产物，每次打包重新生成，**不进仓库**
（已加进 `.gitignore`），可在打包后删掉。

### 8.2 本机环境的硬限制（踩过）
- **工具调用起的 GUI 进程会被回收**：`Start-Process` / `cmd /c start` 起的随影，
  在本次工具调用结束后会被杀。需要真机验证时，**启动与断言必须写在同一条命令里**；
  长时观察让用户自己双击 bat。
- **PowerShell 的 `Add-Type` 在工具调用内被拦**：截图、`gen-app-ico.ps1` 都跑不了。
  （`build.ps1` 自己的 powershell 子进程里 Gate 5 的 `Add-Type` 可正常执行。）
- **Bash 里不要调 PowerShell**（会被拦）；用 PowerShell 工具或纯 Python（`ctypes` 可用）。
- **截图必须先征得用户同意**，且同意有时限，过期需重新问。
- 本机工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- 本机**没有 `gh` CLI**，GitHub Release 只能网页手动建；`git push` 走 SSH（可用）。

### 8.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（39 项编号；缺失尾部按默认补）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%

### 8.4 关键技术参数（改代码前必看）
- IPC 结构 `ShadeBarIpc` 80 字节；`reserved[0]` 是元素掩码（TIME/DATE/TRAY/INDICATOR）。
- 队列位序：`SettingKey as usize`，**新增项只能追加在末尾**（`SETTING_COUNT` 当前 39）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块。
- 采样必须用屏幕 DC，不能用窗口 DC（TranslucentTB 模糊结果拿不到）。
- 体积：单 exe 约 663 KB；不得引入新 crate（windows 锁 `=0.58.0`）。

---

## 9. 用户偏好（长期有效，别违反）

- 语言：简体中文。
- **截图**：任何截图前必须询问并取得同意，同意有时限，过期需重新问。
- **安装应用**：必须先问、由用户选路径。
- **删除文件**：先列清单、说明影响，得到授权后才删。
- **版本号**：严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- **验收**：每组必须真机验收；他说「没问题」才继续。汇报要带任务清单与进度。
- 不要替他做"这个不重要"的判断；发现问题先说清根因，再给方案。

---

## 10. 新对话冒烟验证（3 分钟）

```bash
# 1) 清场：确保没有残留实例
powershell -c "Get-Process *ShadeBar* -ErrorAction SilentlyContinue | Stop-Process -Force"

# 2) 确认 explorer 里只有当前版本载荷（多份 = 旧代码可能在生效）

# 3) 构建 + 启动
cargo build --release
cmd //c "1. 一键启动预览.bat"

# 4) 看四处：时间/日期是否跟随背景、所有指示条是否同色、托盘图标是否在、
#    重启一次 explorer 后图标是否自己回来（v1.3.2）
# 5) 日志侧回归：python tools/check_payload_sync.py
```
