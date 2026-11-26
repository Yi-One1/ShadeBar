# 随影 ShadeBar v1.5.4

本次是缺陷修复版本（PATCH）：解决**托盘「显示隐藏的图标」箭头在深色任务栏下
不着色**。功能与设置项与 v1.5.3 完全一致。

## 修复

- 根因：托盘展开箭头 `SystemTray.ChevronIconView` 在 XAML 诊断树里**没有任何
  XAML 子元素**，字形由 WinComp 内部绘制。此前按“子 `TextBlock` / `FontIcon`”
  匹配的规则永远命中不到它，箭头一直保持系统色。
- 现在把该控件**本身**归入托盘着色通道，直接给它写 `Foreground`，复用已有的
  着色 / 还原逻辑；元素移除时同步清理句柄。
- 注意不能用类型子串匹配 `TextBlock`——那会把 `SystemTray.AdaptiveTextBlock`
  误收进来（实测托盘计数从 5 膨胀到 30）。
- 实测：重启 explorer 后日志 `[Tray+] chevron Foreground ok idx=876`、
  `[Apply] … tray=6`，箭头像素 ≈ `#F2F2F2`，与托盘字形一致。

## 验证

- `check_payload_sync.py` C1–C8 全 PASS（新增 **C8**：箭头进树后必须能解析出
  `Foreground`，否则判失败）。
- 单元测试 232 项通过（app 133 / core 97 / doc 2）。
- 打包五道门禁全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 / 图标）。

## 兼容性与已知限制

- 已在 Windows 11 26H2 Build 26300.9711 实测；Windows 10 未验证。
- 升级后请重启一次 explorer（或电脑），让旧版载荷退出、加载 v1.5.4。
- 图片型托盘图标仍无法染色；多显示器仍只处理含任务栏的主屏。

## 下载

- GitHub 附件：`随影ShadeBar_v1.5.4.exe`（672.5 KB / 688,640 B）
- 本地打包产物：`dist/随影ShadeBar_v1.5.4.exe`
- SHA-256：`017AF671981B7527E5CB18EA9303975D3D63794C6B8DB455180E6104B8441DAC`

完整变更见 [CHANGELOG.md](../CHANGELOG.md)。
