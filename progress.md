# 随影 ShadeBar · 项目进展快照

> **最后更新**：2026-11-20 21:56 (GMT+8)
> **用途**：跨对话续接。新对话看不到聊天记录，本文件是唯一状态依据。
>
> **状态一句话**：G1–G4 已提交并验收；G5 四页签设置窗口已提交（`6647ec5`），
> 用户验收后进入 UI 细节修复阶段：组 2 托盘 `^`（已验收）、组 3 预览清晰度
> （已完成待验收）、组 4 数值范围标注、组 5 导出诊断包。每组单独测试 + 提交 + 验收。
>
> 本文档 2026-11-20 整体重写：删掉了 v1.1.x 阶段的过时结论与已修缺陷的重复条目，
> 只保留**仍然有效**的状态、决策、坑与参数。历史溯源见 CHANGELOG 与 `docs/03-recolor-mechanism.md`。

---

## 0. 新对话先做这三件事（五分钟上手）

1. `git log -3 --oneline` —— 设置窗口重做与各组修复各自一条提交（见 §2.2）。
2. `cargo test --release` —— 应得 **113 + 97 + 2 = 212 通过 / 1 ignored / 0 失败**；
   `cargo check --release --all-targets` 零警告。
3. 让用户双击 `1. 一键启动预览.bat` 验收 G5；确认后 **提交 → 更新 dist → tag/push**。

> **验收闸门**：用户明确要求——每组做完必须由他真机验收，他说「没问题」之后才允许
> 更新文档并进入下一组。不要跳过，也不要替他判断"应该没问题"。

---

## 1. 已完成（均已提交，用户已验收）

| 组 | 覆盖需求 | 提交 | 关键产出 |
|---|---|---|---|
| G1 | #1 指示条一致改色 / #7 深色壁纸漏改 | `0a4de88` | 状态切换置脏（250 ms 节流）、选中态逐层染色、还原对称、**写完回读**；`tools/verify-indicators.ps1` |
| G2 | #4 分区独立开关（兼答 #5） | `7125d8a` | 新增 S-48/49/50 三区开关；`mask = 元素开关 AND 分区开关`；图标区采样上限卡到托盘起点 84% |
| G3 | #6 任意颜色 | `08ce53e` + 文档 `dddd002` | 三组 18 色推荐色板（带对比度实测）、HEX 输入、comdlg32 系统调色板、S-13 自动切自定义、alpha 语义修正、消除 `"颜颜"` panic |
| G4 | #3 图标重设计 | `7b6c672` + 返工 `f531f87` + 文档 `2a13a91` | 九套预览选 **A·对分**；三色对角方砖；托盘 / exe / 窗口标题三处统一；去白点、`#595959` 深灰轮廓；`resources/gen_app_ico.py` 纯标准库生成 ICO |
| — | 一键启动预览脚本 | 含在 `f531f87` | `1. 一键启动预览.bat`：`cargo build --release` 后启动 `target\` 最新版，**不覆盖 dist** |

**当前能力**（v1.2.0 + G1–G4，真机验证过）：
- 时间 / 日期 / 托盘字形图标 / 指示条（正常态 + 选中态）随背景明暗自适应着色；
- 三区（时钟 93–99% / 托盘 84–93% / 图标区 0–84%）各自判定，可单独开关；
- 亮端色 / 暗端色任意取色（色板 + HEX + 系统调色板）；
- 常驻工作集 8–15 MB、CPU 约 0.052%（v1.2.0 实测，G5 后未复测）；
- 退出 / 被强杀都能完整还原系统原色。

---

## 2. 进行中：G5 设置窗口重做（四页签版，代码+文档完成，待真机验收）

### 2.1 方案与范围
用户从四套外观预览里选定 **1 · 任务栏直控**，并要求元素开关**合并为三区**、
顺序为 **图标 → 托盘 → 时钟**。选型稿：`deliverables/settings-redesign-2026-11-20.html`。

- 720×560 dip，**无自绘标题栏**（系统标题栏 + Mica Alt），头部 **48**、内容区手工
  排布**不滚动**、底栏 **48**，行高 **44**。
- 头部：4 个页签（任务栏 / 采样 / 托盘 / 诊断）+ Y 读数 + 状态 chip + 着色开关 + 暂停。
- **任务栏页**：150 px 实时预览（开始按钮 → 应用图标 → 托盘 → 竖分隔 → 时钟，
  时钟最右）+ 三区卡片（S-50 图标 / S-49 托盘 / S-48 时钟）+ 颜色与判定卡
  （S-14 亮端色 / S-15 暗端色、S-47 判定粒度、S-12 阈值 32 段条）。
- **采样页**：采样带（160）+ 间隔 / 平滑 / 起点 / 宽度 / 偏移（S-16/18/20/21/22）。
- **托盘页**：左键行为 / 关闭行为 / 开机自启 / 启动最小化 / 语言（S-40/41/02/03/04）。
- **诊断页**：实时判定摘要 + 迷你趋势 + 日志级别（S-45）+ 版本 + 导出诊断包。
- 可见设置 **20 项**（39 项编号与 `config.json` 下标不变）；S-43 继续不在 UI。

### 2.2 已提交的文件（`6647ec5`）
```
Cargo.toml  Cargo.lock                         # 1.3.0
crates/shadebar-app/Cargo.toml                 # + Win32_System_SystemInformation / UI_Input_KeyboardAndMouse
crates/shadebar-app/src/app.rs
crates/shadebar-app/src/config.rs              # ui_visible 20 项、删 Tier
crates/shadebar-app/src/render.rs              # 新文本角色 / 右对齐
crates/shadebar-app/src/ui/theme.rs            # 预览色、尺寸常量
crates/shadebar-app/src/ui/layout.rs           # Rect + 文本度量（滚动/导航已删）
crates/shadebar-app/src/ui/mod.rs              # 四页签视图（整体重写）
crates/shadebar-app/src/ui/pages/mod.rs        # 页面归属表
crates/shadebar-app/src/ui/pages/diagnostics.rs
crates/shadebar-app/src/ui/pages/region.rs
crates/shadebar-app/src/ui/widgets.rs          # 仅留 VK / 浮层类型
crates/shadebar-app/src/ui/pages/{behavior,color,elements,general}.rs   # 删除
crates/shadebar-app/src/window.rs
resources/lang/en-US.tsv  resources/lang/zh-CN.tsv
README.md  CHANGELOG.md  progress.md           # 收口文档
deliverables/settings-redesign-2026-11-20.html # 新增（选型稿）
```

### 2.3 这一组踩过的坑（已修，别回退）
1. **三区顺序**：用户指出正确顺序是 **图标 → 托盘 → 时钟**；预览 DOM、卡片数组、
   命中映射三处必须同序，否则"看得见、点得着、但改错区"。
2. **windows 0.58 路径**：`GetLocalTime` 在 `Win32::System::SystemInformation`、
   `SYSTEMTIME` 在 `Win32::Foundation`，`SetCapture/ReleaseCapture` 在
   `Win32::UI::Input::KeyboardAndMouse`；`Win32::System::Time` 不存在。
3. **测试与默认值**：S-14 默认就是白色，点白色色块不会产生写入；阈值条测试要点在
   轨道内端点，点轨道外不属于命中区。

### 2.4 质量证据
- `cargo check --release --all-targets` 零警告。
- `cargo test --release` = 109 + 97 + 2 = **208 通过 / 1 ignored / 0 失败**。
- `cargo build --release` 成功，产物 668672 B（653 KB）。
- 真机效果仍以用户验收为准。

### 2.5 提交前还要做
- [x] README / CHANGELOG / 本文件按四页签现状收口（20 项可见、1.3.0、653 KB）。
- [x] `cargo test --release` 212 通过 / 1 ignored / 0 失败；零警告。
- [x] `cargo build --release` 成功，target 产物 668672 B（653 KB）。
- [ ] 用 `1. 一键启动预览.bat` 让用户真机验收：4 页签、预览与三区、底栏四按钮、
      颜色 / 下拉浮层位置是否与点击一致。
- [ ] 通过后：提交上述文件 → 更新 `dist/随影ShadeBar_v1.3.0.exe` →
  `git tag v1.3.0` + push。

---

## 3. 关键决策（含理由，别重新发明）

| 决策 | 结论 | 理由 |
|---|---|---|
| 协作方式 | G3 起小队（架构/设计/前端/测试），lead 编排不代写 | 用户明确要求团队协作；单兵只用于 G1/G2 那种单点改动 |
| 指示条两态 | **目标色一致、写入路径不同** | 选中态 `Fill` 被 VisualState 压住，必须改画刷 `Color`；正常态画刷被 29 处共享，不能逐层染 |
| 颜色选择 | 色板 + HEX + 系统调色板 | 用户三选一拍板；端色 alpha 恒 `0xFF`（`argb_to_rgba` 本就丢 alpha） |
| 改色联动 | 改 S-14/S-15 时自动把 S-13 切「自定义」并提示 | 双门控会让选色完全无效；用户在 A/B 方案中选 A（自动切） |
| 图标 | 九套预览 → 最终选 A·对分；三色；去白点 | 用户先要彩色、再要自由发挥，最终仍选 A；但要求轮廓清晰、不要点 |
| 设置布局 | 四页签 + 任务栏直控预览，无左侧导航 / 无分档 | 用户从四套预览选定方案 1；原痛点是"排版混乱、显示区与交互区不一致" |
| 三区顺序 | **图标 → 托盘 → 时钟** | 用户明确纠正；预览、卡片、命中三处同序 |
| S-35/S-37 | 继续隐藏 | 悬停高亮块从未做 60 s 稳定性验证，不把未验证能力摆到用户面前 |
| 验收方式 | 一律用 `1. 一键启动预览.bat`，**不覆盖 dist** | 用户明确要求；dist 只在发布时更新 |
| 版本 | 每组提交不改号，全部完成后统一 **1.3.0（MINOR）** | 严格 SemVer：新功能 MINOR；修 bug 是 PATCH |
| 分支 | 直接提交本地 `main`，全部完成后再推送 | 用户未要求中间推送 |

---

## 4. 未完成的待办

### 4.1 立刻要做（G5 收尾）
- [x] 更新 `README.md`：20 项可见 / 39 编号、四页签、配置导入导出、S-43 移除、1.3.0。
- [x] 更新 `CHANGELOG.md`：1.3.0 条目改为四页签。
- [x] 更新本文件，G5 标为"代码+文档完成，待验收"。
- [x] **全量测试 + 构建**：`cargo test --release` 208 通过 / 1 ignored / 0 失败；
  `cargo check --release --all-targets` 零警告；`cargo build --release` 成功。
- [ ] 用户用 `1. 一键启动预览.bat` 验收 G5，确认布局与点击一致。
- [ ] 验收通过后提交 §2.2 的文件（别漏 `resources/lang/*.tsv`、`Cargo.lock`、删除的旧页面）。

### 4.2 发布收尾
- [x] 版本升 **1.3.0**：`Cargo.toml`、`Cargo.lock`、README 横幅、CHANGELOG、本文件。
- [ ] 验收通过后更新 `dist/随影ShadeBar_v1.3.0.exe`（此时才动 dist）。
- [ ] `git tag v1.3.0` + `git push`。
- [ ] 用户终验签字。

### 4.3 长期缺口（用户没催，不要主动扩范围）
- 悬停高亮块（S-34~S-37）：**生产载荷尚未实现**（`payload/` 零处 `BackgroundElement`），
  需先实现再做 60 s 稳定性验证，通过后才能放回 UI。
- 多显示器（副屏任务栏元素契约未用探针确认）。
- 深色壁纸对照证据归档。

---

## 5. 踩坑清单（本项目最贵的资产）

| 症状 | 真因 | 处置 |
|---|---|---|
| "新版本没修好旧问题" | explorer 同时驻留多版载荷，**旧副本在生效** | 注入类测试收尾必须清场；**日志格式与源码对不上 = 跑的不是这份代码** |
| 以为写入成功就生效 | `SetProperty` 返回 `S_OK` 只代表写进去 | 写完**回读**画刷 `Color`；`tools/verify-indicators.ps1` 做断言 |
| 图标"模糊" | 白半融进白桌面、黑半融进深色栏，细浅灰边撑不住 | 轮廓改 `#595959`、16 px 框 1 px / 缝 2 px |
| exe 图标改了没变 | Windows 图标缓存 | `ie4uinit -show` + 重启 explorer；用二进制搜 PNG 签名确认资源已更新 |
| 界面能看见但点不中 | 绘制/命中两套坐标（内容 vs 客户区） | 统一坐标语义 + 纯 frame 函数；**用变异测试证明断言有效** |
| 输入 `"颜颜"` 崩溃 | 自写 `parse_hex` 缺 ASCII 守卫，在非 char 边界切片 | 收敛到 `shadebar_core::color::Rgba::parse_hex` |
| 指示条漏改 | 正常↔选中不产生 Add 事件，`OnElementStateChanged` 空实现不置脏 | 置脏 + 250 ms 节流 |
| 还原留色 | 应用逐层染、还原只改一层 | 还原与应用对称（`SetAllBrushColorsOfProp`） |

---

## 6. 环境、命令与限制

### 6.1 构建与测试
```bash
cd "E:/AI工作空间/随影ShadeBar"

# 载荷（C++，必须先编，宿主编译期内嵌它）
cd payload && bash build.sh && cd ..

# 宿主（release，~8 s 增量）
cargo build --release

# 测试（当前 253 项）
cargo test --release

# 一键启动预览（用户验收用；先 build 再启动 target 最新版）
cmd //c "1. 一键启动预览.bat"
```

### 6.2 本机环境的硬限制（踩过）
- **GUI 进程活不过一次命令调用**：`Start-Process` / `&` 起的随影会在工具调用结束时被收掉。
  需要真机观察时让用户自己双击 bat；单次 Bash 内可用 `./exe & ... kill $!` 抓一张图。
- **PowerShell 的 `Add-Type` 被安全策略拦截**：`resources/gen-app-ico.ps1` 跑不了，
  所以 ICO 改用纯 Python 标准库 `resources/gen_app_ico.py` 生成。
- **Bash 里不要调 PowerShell**（会被拦）；用 PowerShell 工具或纯 Python。
- **截图必须先征得用户同意**，且同意有时限，过期需重新问。
- `dist/` 里的 exe 只有发布时才更新，日常验收走 bat。

### 6.3 数据与路径
- 配置：`%LOCALAPPDATA%\ShadeBar\config.json`（39 项编号；缺失尾部按默认补，不判损坏）
- 日志：`%LOCALAPPDATA%\ShadeBar\payload.log`（载荷）、`shadebar.log`（宿主）
- 载荷副本：`%LOCALAPPDATA%\ShadeBar\ShadeBarTap_v<ver>_<指纹>.dll`
- 采样分区：时钟 93–99%、托盘 84–93%、图标区 0–84%（`CLOCK_SPAN / TRAY_SPAN / ICONS_SPAN_MAX`）

### 6.4 关键技术参数（改代码前必看）
- IPC 结构 `ShadeBarIpc` 80 字节；`reserved[0]` 是元素掩码（TIME/DATE/TRAY/INDICATOR）。
- 队列位序：`SettingKey as usize`，**新增项只能追加在末尾**（`SETTING_COUNT` 当前 39）。
- 载荷文件名带内容指纹，避免 explorer 复用旧模块；注入失败要收手，不要反复重试。
- 采样必须用屏幕 DC，不能用窗口 DC（TranslucentTB 模糊结果拿不到）。
- 体积门禁：单 exe 约 650 KB 量级；不得引入新 crate（windows 锁 `=0.58.0`）。

---

## 7. 用户偏好（长期有效，别违反）

- 语言：简体中文。
- **截图**：任何截图前必须询问并取得同意，同意有时限，过期需重新问。
- **安装应用**：必须先问、由用户选路径。
- **版本号**：严格 SemVer 2.0.0；修 bug 是 PATCH，新功能是 MINOR。
- **验收**：每组必须真机验收；他说「没问题」才继续。汇报要带任务清单与进度。
- 不要替他做"这个不重要"的判断；发现问题先说清根因，再给方案。

---

## 8. 团队协作（会话级）

上一轮用专家团「MVP开发专家团」的小队模式（架构师 / 设计师 / 前端 / 测试），
团队名 `shadebar-g3-color`。**团队是会话级的**，新对话不会继承：

- 若继续用小队：重新建团队、重新派单；成员不记得上一轮上下文，施工单必须写全。
- 若单干：G5 只剩"提交 + 文档 + 版本收尾"，一个人更快，不必拉人。

**上一轮有效的做法**（值得保留）：
1. 施工单写死文件、行号、期望值，不复述需求。
2. 每个成员交付必须带"改了哪些行 + 测试数字 + 风险"。
3. 测试员做**变异测试**（故意改坏一处，看测试是否 FAIL），专治假断言。
4. 复核不通过就定点返工，不整组重做。

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

# 4) 看三处：时间/日期颜色是否跟随背景、指示条正常态与选中态是否同色、托盘图标是否为新 A 图标
```
