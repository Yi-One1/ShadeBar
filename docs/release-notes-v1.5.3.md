# 随影 ShadeBar v1.5.3

本次是缺陷修复版本（PATCH）：解决**一部分运行中应用的正常态指示条永远不亮**
（重启软件无效，只有重启 explorer 才恢复）。功能与设置项与 v1.5.2 完全一致。

## 修复

- 根因是**重复注入**：宿主每次启动都会向同一个 explorer 再注入一份载荷。
  XAML 诊断一条连接只允许成功订阅一次，第二次订阅拿到的是失效连接——
  能写的那部分元素正常，其余指示条永远进不了树。
- 现在宿主注入前先连读两次载荷活跃计数（IPC `reserved[6]`，载荷主循环每轮 +1，
  与总开关无关）；计数在动说明已有载荷存活，直接接管颜色并**跳过注入**。
- 实测：重启软件后日志为 `payload alive: explorer pid=..., skip inject`，
  7 条指示条保持 7/7 着色；explorer 重启后同样 7/7。

## 验证

- `check_payload_sync.py` C1–C7 全 PASS（指示条 20/20、时间/日期/托盘 20/20、0 次重放）。
- 单元测试 232 项通过（app 133 / core 97 / doc 2）。
- 打包五道门禁全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 / 图标）。

## 兼容性与已知限制

- 已在 Windows 11 26H2 Build 26300.9711 实测；Windows 10 未验证。
- 升级后请重启一次 explorer（或电脑），让旧版载荷退出、加载 v1.5.3。
- 托盘"显示隐藏的图标"箭头（`SystemTray.ChevronIconView`）仍在定位中，
  本版未处理；图片型托盘图标仍无法染色。
- 多显示器仍只处理含任务栏的主屏。

## 下载

- GitHub 附件：`ShadeBar_v1.5.3.exe`（671.5 KB / 687,616 B）
- 本地打包产物：`dist/随影ShadeBar_v1.5.3.exe`
- SHA-256：`0ACEAC2A9A55AD7923FA3E5C5E247AE19D563BE3CA4BCCA5C94427EA148F24FF`

完整变更见 [CHANGELOG.md](../CHANGELOG.md)。
