# 随影 ShadeBar v1.5.5

本次为缺陷修复版本（PATCH）：根治**冷开机后新开程序的任务栏指示条保持灰色**。
功能与设置项与 v1.5.4 完全一致。

## 修复

- **根因**：Windows XAML 诊断在单个进程内只有**一个实时消费者**。后连接者独占
  `Add/Remove` 增量事件流，先连接者只保留连接瞬间的全量快照。冷开机时随影载荷
  先连接（17:56:01），TranslucentTB 的 `ExplorerTAP.dll` 后连接（17:56:03），
  之后新开程序的事件全部丢失；开机时已在的程序（如 QQ）不受影响。
- **修复**：载荷建立**新连接**重新夺回席位（同一连接 `Unadvise` 后再 `Advise`
  必返 `E_UNEXPECTED`，只能开新连接）。旧连接保持订阅不动，重建快照时丢弃全部
  失效句柄。
- **自动触发**：首连成功后 10 秒兜底重连；TranslucentTB **每次进程启动** +8 秒；
  `ExplorerTAP.dll` 在本次连接后加载 +2 秒；两次自动重连至少间隔 10 秒。
- **手动兜底**：创建 `%LOCALAPPDATA%\ShadeBar\reconnect.req` 可立即触发一次重连。

## 验证

- **真冷开机验收**：22:29:27 首连 → 22:29:31 TranslucentTB 启动 → 22:29:34 自动
  重连；之后新开记事本/画图均收到 `[Add] RunningIndicator`，指示条为白色，
  全程无人工操作。
- 单元测试 235 项通过（app 136 / core 97 / doc 2）。
- `check_reconnect_logic.py` R1–R12、`check_payload_sync.py` C1–C9 全 PASS。
- 打包五道门禁全绿（体积 / 无 VC++ 运行时 / 版本资源 / 清单 / 图标）。

## 兼容性与已知限制

- 已在 Windows 11 26H2 Build 26300.9711 实测；Windows 10 未验证。
- 升级后请重启一次 explorer（或电脑），让旧版载荷退出并加载 v1.5.5。
- 图片型托盘图标仍无法染色；多显示器仍只处理含任务栏的主屏。
- 非 TranslucentTB 的第三方诊断工具若在自动重连之后才连接，可用手动哨兵夺回。

## 下载

- GitHub 附件：`随影ShadeBar_v1.5.5.exe`（685.5 KB / 701,952 B）
- SHA-256：`A9DB6C464DCA3E14A3B857740BE64A31032047FA7A14A1732E9F8D94D62BCA1D`

完整变更见 [CHANGELOG.md](../CHANGELOG.md)。
