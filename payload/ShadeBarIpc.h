// 随影 ShadeBar — 宿主 ⇄ 载荷 共享内存协议
//
// 宿主（Rust）与载荷（C++）必须严格保持同一二进制布局。
// 任何字段增删都必须同时提升 SHADEBAR_IPC_VERSION，并在宿主侧做版本校验。

#ifndef SHADEBAR_IPC_H
#define SHADEBAR_IPC_H

#include <windows.h>

#define SHADEBAR_IPC_NAME    L"Local\\ShadeBar.Ipc.v1"
#define SHADEBAR_IPC_MAGIC   0x53425231u   /* "SBR1" */
#define SHADEBAR_IPC_VERSION 1u

#define SHADEBAR_STATUS_IDLE    0
#define SHADEBAR_STATUS_READY   1
#define SHADEBAR_STATUS_FAILED  2

#pragma pack(push, 4)
struct ShadeBarIpc {
    unsigned int magic;              /* 0  */
    unsigned int version;            /* 4  */
    volatile LONG seq;               /* 8    宿主每次下发 +1 */
    volatile LONG enabled;           /* 12   1=应用配色 0=还原 */
    /* shutdown: 1=还原并退出载荷（偏移 16）。
     * 不变量：任何写 shutdown=1 的一方必须先写 enabled=0（宿主 Ipc::shutdown()
     * 已按此顺序：enabled=0 → seq+1 → shutdown=1）。载荷据此区分正常退出与陈旧值：
     * 因正常退出必 enabled=0，故 enabled=1 && shutdown=1 只能是旧线程的陈旧值/竞态残留，
     * 载荷忽略之并清 shutdown=0（见 ShadeBarTap.cpp::IsStaleShutdown）。
     * 破坏此顺序会导致新宿主已接管、enabled=1 时旧 shutdown=1 使载荷永久待命、颜色不更新。 */
    volatile LONG shutdown;          /* 16   1=还原并退出载荷 */
    volatile LONG heartbeat;         /* 20   载荷每轮 +1（存活心跳） */
    volatile LONG status;            /* 24   载荷回报状态 */
    volatile LONG tapSeq;            /* 28   载荷已应用的 seq */
    unsigned int timeColor;          /* 32   0xAARRGGBB */
    unsigned int dateColor;          /* 36 */
    unsigned int trayGlyphColor;     /* 40 */
    unsigned int indicatorNormalColor;   /* 44 */
    unsigned int indicatorSelectedColor; /* 48 */
    /* reserved[0] = 元素掩码：bit0 时间 / bit1 日期 / bit2 托盘字形 / bit3 指示条。
     * 未置位的元素会被写回系统原色（不是清空），从而让设置开关即时生效。
     * 其余保留，保持 80 字节对齐。 */
    unsigned int reserved[7];        /* 52..79 */
};
#pragma pack(pop)

#endif /* SHADEBAR_IPC_H */
