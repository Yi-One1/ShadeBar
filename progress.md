# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-23 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：**v1.4.0 已打包、门禁全绿**，产物 `dist/随影ShadeBar_v1.4.0.exe`
> （689,152 B，测试 **229** 项全绿）；**待提交推送、打 tag、建 GitHub Release**。
> v1.4.0 内容：分区独立配色（8 色 + 不透明度）、左键双击托盘、托盘菜单四项、
> 滑轨统一、采样带内缩改百分比，以及托盘字形 / 选中态颜色两处真机缺陷修复。

---

## 0. 新对话先做这三件事（五分钟上手）

1. `git log -8 --oneline` —— 应能看到 `277ca11`（托盘字形）到 `b80e6b0`（滑轨统一）的一串提交。
2. `cargo test --release` —— 应得 **130 + 97 + 2 = 229 通过 / 1 ignored / 0 失败**；
   `cargo check --release --all-targets` 零警告。
3. 需要重新体验：双击 `1. 一键启动预览.bat`（重新编译并启动 `target\` 最新版，
   **不覆盖 dist**）。重新打包：`powershell -ExecutionPolicy Bypass -File build.ps1`。

> **验收闸门**：用户明确要求——每组做完必须由他真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。

---

## 1. 当前版本与产物

| 项 | 值 |
|---|---|
| 版本 | **1.4.0**（MINOR：分区配色 / 双击 / 滑轨统一等） |
| 版本唯一来源 | `Cargo.toml` → `[workspace.package] version`（资源 / 关于页 / 文件名自动跟随） |
| 发布产物 | `dist/随影ShadeBar_v1.4.0.exe`（689,152 B，673 KB） |
| SHA-256 | `28FCF39D1BA52E0C569C841318ABC47D2D7CE79BEC83D80288DB614FF6C334D4` |
| FileVersion | `1.4.0.0`（`--version` 输出 `随影 ShadeBar v1.4.0`） |
| 门禁 | 5/5 全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 PerMonitorV2+asInvoker / 图标）；双语差 9 KB（预算 40 KB） |
| git tag | `v1.4.0` **待创建**；历史：`v1.3.2` → `9c9d850`、`v1.3.1` → `d2bed34`、`v1.3.0` → `93c6402` |
| GitHub Release | **待发布**（附件命名 `ShadeBar_v1.4.0.exe`，上传后中文前缀会被去掉，属既成口径） |

`dist/` **永远保留历史版本 exe**（用户要求，便于回滚）；exe 不进仓库，只走 Releases。

---

## 2. v1.4.0 完成事项（2026-11-23）

| 提交 | 内容 | 验证 |
|---|---|---|
| `da1a5de` | 文档：开机自启口径统一为 HKCU Run | 文档对齐代码 |
| `277ca11` | **托盘字形改染 Base 层文本块**（问题 6） | 红/绿真机截图 + `check_payload_sync.py` 新增 C6 |
| `99c624c` | 阈值滑条圆帽 + 采样带内缩预览按百分比（问题 2/3） | 2 个几何回归测试 + 截图 |
| `e190075` | 托盘菜单四项 + 左键双击（问题 5/7） | 菜单截图 + `click_plan` 单测 |
| `4b2ac2a` | 分区独立配色 8 色 + 不透明度 + 选中态独立色（问题 1） | 真机四色截图 + 6 个单测 |
| `e1575ef` | 不透明度滑条与阈值滑条统一外观 | 截图 |
| `2d2850f` | 配色卡标签间距、S-60 改名、语言表去重 | 截图 + 229 测试 |
| `b80e6b0` | 全部滑轨统一 + 修复统一模式不透明度不可拖 | 截图 + 3 个回归测试 |

### 2.1 问题 6 的真相（两个独立原因）

1. **时间/日期、指示条其实一直在写**。用户配置 `S-13=0`（纯黑白）+ 深色背景 →
   目标色就是纯白，与系统默认同色，看起来"没改"。红/绿高辨识色实测全部生效。
2. **托盘字形确实有 bug**：载荷按类型 `AdaptiveTextBlock` 匹配，写到
   `Underlay/Base/AccentOverlay` 三层，其中只有 `Base` 的 `InnerTextBlock` 是可见字形。
   现按「父级名为 `Base` 的 `InnerTextBlock`」收集，还原路径同步。

### 2.2 七项需求对照

| # | 需求 | 状态 |
|---|---|---|
| 1 | 分区独立配色（4 内容 × 2 背景态 + 不透明度） | 完成（S-52~S-60） |
| 2 | 阈值滑条左端两个黑点 | 完成 |
| 3 | 采样带偏移 28~45 预览不动 | 完成；语义改为百分比、上下对称内缩 |
| 4 | EWMA α 解释 | 已解释（未改代码） |
| 5 | 左键双击托盘图标 | 完成（S-51） |
| 6 | 部分元素不改色 | 完成（见 §2.1） |
| 7 | 托盘菜单只留 4 项 | 完成 |

---

## 3. 关键决策（含理由，别重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 分区配色粒度 | **8 色 = 4 内容 × 2 背景态** + 独立不透明度 | 用户拍板；4 项天然分属三区，无需 24 色 |
| 统一 / 分区关系 | **各管一种模式**：统一用 S-14/S-15，分区用 8 色 | 避免两套颜色互相覆盖 |
| 正常指示条透明度 | 独立滑条 S-60（默认 45%），两种模式都生效 | 保持原系统半透明手感，同时可调 |
| 单击/双击共存 | 双击非「无操作」时单击挂起 `GetDoubleClickTime` | legacy 托盘先送单击；不挂起会多触发一次 |
| S-22 语义 | **任务栏高度百分比，上下对称内缩** | 引擎本来就这么算；预览与文案对齐 |
| 指示条两态 | 颜色可不同，写入路径不同 | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color` |
| 正常态一致性 | 所有正常态共用一个画刷 | 换色同一刻生效，避免"改几条剩几条" |
| 托盘字号定位 | 只认 `AdaptiveTextBlock` 的 `Base` 层 `InnerTextBlock` | 其余两层透明 / 强调覆盖，写了也不可见 |
| 元素树更新 | 重放（Unadvise+Advise），有输入 5 s / 空闲 30 s | 无遍历 API，新窗口不产生可用 Add |
| 托盘补挂 | 响应 `TaskbarCreated`，先 MODIFY 探测再 ADD | 无条件 `NIM_ADD` 会撞 uID 返回 `E_FAIL` |
| 开机自启 | HKCU Run（v1.1 起），不注册 schtasks | 零 UAC；需求/架构中的 schtasks 仅作历史 |
| 设置布局 | 四页签 + 任务栏直控预览；配色卡按模式切换 | 用户选定方案 1；统一模式保留两块端色 |
| 版本 | 严格 SemVer 2.0.0：修 bug = PATCH，新功能 = MINOR | 用户长期规则 |
| 验收方式 | 一律 `1. 一键启动预览.bat`，不覆盖 dist | 用户明确要求 |
| 打包产物 | 新 exe 只新增，不删旧 exe | 便于回滚与对比 |

---

## 4. 待办

### 4.1 本次发布收尾
- [x] 版本 1.4.0（`Cargo.toml` / `Cargo.lock` / README / CHANGELOG / 本文件）。
- [x] `build.ps1` 打包，五道门禁全绿，产物与哈希见 §1。
- [ ] 提交文档与版本变更 → `git push origin main`。
- [ ] `git tag -a v1.4.0` 并推送。
- [ ] 网页建 GitHub Release `v1.4.0`，上传 `ShadeBar_v1.4.0.exe`，
      Release notes 用 `docs/release-notes-v1.4.0.md`。
- [ ] 建完 Release 后，把 tag / Release 状态补回本文件与 README。

### 4.2 长期缺口（用户没催，不要主动扩范围）
- 悬停高亮块（S-34~S-37）：生产载荷尚未实现（`payload/` 零处 `BackgroundElement`）。
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
| 托盘图标"改了没变" | 按 `AdaptiveTextBlock` 匹配，写到的三层里只有 `Base` 的 `InnerTextBlock` 可见 | 按父级名收 `Base` 层子 TextBlock（v1.4.0） |
| 指示条两态颜色无法区分 | 载荷选中态误用 `indicatorNormalColor` | 改读 `indicatorSelectedColor`，还原兜底认两种色（v1.4.0） |
| "改了颜色却没变化" | 纯黑纯白策略下，白色目标与系统默认同色 | 排查时先用红/绿等辨识色复现 |
| 滑条只在某种模式能拖 | 命中检测写在 `per_region()` 分支内 | 命中检测必须覆盖所有显示该控件的模式（v1.4.0） |
| 语言表键值重复 | 追加脚本跑两次；`required_keys` 只查"存在"不查"唯一" | 改语言表后检查重复键 |
| 软件在跑、托盘没图标 | 不响应 `TaskbarCreated`；`added` 失败不清零 + 缓存短路 | v1.3.2：广播补挂 + 失败归零 |
| 重复 `NIM_ADD` 返回 `E_FAIL` | 图标已存在时再 `NIM_ADD` 撞同一 uID | 先 `NIM_MODIFY` 探测 |
| 一部分指示条永久不改色 | 订阅只给一次全量树 | v1.3.1：定时 / 触发式全量重放 |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完回读画刷 `Color` |
| `build.ps1` 报 ps1 encoding guard | 脚本要求纯 ASCII，混入中文注释 | 注释只用 ASCII；需中文正则的脚本存 UTF-8 带 BOM |
| 界面能看见但点不中 | 绘制/命中两套坐标 | 统一坐标语义 + 纯 frame 函数 + 变异测试 |
| 还原留色 | 应用逐层染、还原只改一层 | 还原与应用对称（`SetAllBrushColorsOfProp`） |

---

## 6. 环境、命令与限制

### 6.1 构建与测试
```bash
cd "<repo>"

# 载荷（C++，必须先编，宿主编译期内嵌它）
cd payload && bash build.sh && cd ..

# 宿主 / 测试 / 打包
cargo build --release
cargo test --release          # 229 通过 / 1 ignored
powershell -ExecutionPolicy Bypass -File build.ps1
cmd //c "1. 一键启动预览.bat"
```

`dist/verify/`、`dist/compare/` 是门禁对照产物，每次打包重建，不进仓库。

### 6.2 本机硬限制（踩过）
- 工具调用起的 GUI 进程会被回收：启动与断言必须写在同一命令里。
- PowerShell `Add-Type` 在工具调用内被拦；截图/断言用 Python + `ctypes` + Pillow。
- 截图必须先征得用户同意，同意有时限。
- 工具链：VS `D:\APP\VS2026`（MSVC 14.52.37025）、Windows SDK 10.0.26100.0。
- 本机**没有 `gh` CLI**：GitHub Release 只能网页手动建；`git push` 走 SSH（可用）。

### 6.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（**49 项编号**；缺失尾部按默认补）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%

### 6.4 关键参数（改代码前必看）
- IPC 结构 `ShadeBarIpc` 80 字节；`reserved[0]` 是元素掩码（TIME/DATE/TRAY/INDICATOR）。
- 队列位序：`SettingKey as usize`，**新增项只能追加在末尾**（`SETTING_COUNT` 当前 49）。
- 分项色：S-52/53 正常条、S-54/55 选中条、S-56/57 托盘、S-58/59 时间日期
  （成对为「背景偏暗 / 偏亮」），S-60 正常条不透明度（0–100，默认 45）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块。
- 采样必须用屏幕 DC，不能用窗口 DC。
- 体积：单 exe 约 673 KB；不得引入新 crate（windows 锁 `=0.58.0`）。

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
powershell -c "Get-Process *ShadeBar* -ErrorAction SilentlyContinue | Stop-Process -Force"

# 2) 构建 + 启动（启动与断言同一命令）
cargo build --release

# 3) 测试与载荷回归
cargo test --release
python tools/check_payload_sync.py

# 4) 看四处：时间/日期、托盘文字图标、正常/选中指示条是否按设置着色
```
