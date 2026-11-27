# 冷开机验证记录 2026-11-27

## 结论：问题复现；此前 shutdown 修复非根因

### 硬指标（16:57 实测）
- 系统真重启：是
- 载荷 build=Nov 27 2026 16:28:36（修复版确实加载）
- IPC: enabled=1 shutdown=0 status=1(READY) heartbeat 持续增长
- 记事本/浏览器指示条：**灰色**（用户报告，问题复现）

### 真根因（证据：after/payload.log 16:54:48 会话）
冷开机注入那一刻：
- 300+ Add 事件全量发放，含 1 个 RunningIndicator（QQ），Tree ind=1
之后用户开记事本、浏览器：
- 新增 <<IND>> Add：0 条
- [Add] 总数停在 201（全是注入瞬间）
- [Tree] ind 恒为 1；[Apply] ind 恒为 1
→ 新窗口指示条未进树，无法染色

对比 warm 会话（重启 explorer）：开记事本时 Add 正常到达（ind 6→7）。
差异：冷开机会话的 IVTS 增量事件通道不通，全量发放正常。

### 被推翻的假设
- 托盘箭头写不进色（箭头一直是白的，已回滚相关改动）
- shutdown=1 卡死导致新窗口不染色（冷开机 shutdown=0 且修复版已加载，问题仍在）

### 保留的修复（用户决定）
6bd7223 / 1dd539e / c776316：测试隔离 + 陈旧 shutdown 防护 + 日志归属。
无害、是独立 bug 修复，但与本问题根因无关。

### 下一步
派架构师论证非订阅式元素枚举接口（绝不重新 Advise——v1.5.2 会弄坏订阅），
产出探针方案后验证冷开机能否拿到新窗口元素。

### 证据文件
- before/payload.log, before/shadebar.log：重启前基线
- after/payload.log, after/shadebar.log：冷开机后完整会话
