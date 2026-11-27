#!/usr/bin/env python3
"""重连夺回事件席位的不变量检查（真问题第 1 组）。

背景：冷开机时"最后连接"XAML 诊断的工具会独占实时 Add 事件流，先连接的随影
只剩连接瞬间的快照，表现为"开机后新开程序指示条不变色"。修复方式是主动建立
**新连接**夺回席位（本组由哨兵文件手动触发）。这类修复最容易在后续改动中退化：

  R1 重连必须在新线程里做（XAML 诊断初始化每线程只能成功一次）
  R2 重连线程必须真的调用 InitTap
  R3 每次尝试都在新线程里，连接名按槽位 1..60 生成（失败过的线程不能复用）
  R4 SetSite 的新连接路径不得 Unadvise 旧连接（v1.5.2 实测会损坏诊断连接）
  R5 新连接必须重新订阅，并置"清树 + 重新着色"标志
  R6 首个 Add 到达时必须丢弃旧树与失效句柄
  R7 丢弃句柄时必须同时重置共享画刷 / 箭头 / 选中态画刷缓存
  R8 哨兵 reconnect.req 必须"检测 → 删除 → 触发重连"
  R9 "清树"标志必须在 Advise 之前置位（快照在 Advise 内同步投递），且失败要回滚
  R10 自动重连：首连后 10 s 一次，且检测到 ExplorerTAP 新加载时提前到 +2 s，只触发一次
  R11 重连串行化：自动触发与手动哨兵不得并发开两条连接

用法：
  python tools/check_reconnect_logic.py            # 检查真实源码
  python tools/check_reconnect_logic.py --self-test # 反向自测（故意改坏源码）
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from typing import Callable

CHECK_PREFIX = "R"


def cpp_source_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "..", "payload", "ShadeBarTap.cpp")


def _set_site_body(text: str) -> str:
    """取出 SetSite 函数体（按大括号配平）。SetSite 内没有带大括号的字符串字面量。"""
    i = text.find("SetSite(IUnknown* site)")
    if i < 0:
        return ""
    j = text.find("{", i)
    if j < 0:
        return ""
    depth = 0
    for k in range(j, len(text)):
        if text[k] == "{":
            depth += 1
        elif text[k] == "}":
            depth -= 1
            if depth == 0:
                return text[j:k + 1]
    return ""


def _segment(text: str, anchor: str, size: int) -> str:
    i = text.find(anchor)
    return text[i:i + size] if i >= 0 else ""


def r1_new_thread(text: str) -> bool:
    return ("static DWORD WINAPI ReconnectThread(LPVOID)" in text
            and "CreateThread(nullptr, 0, ReconnectThread" in text)


def r2_calls_init(text: str) -> bool:
    seg = _segment(text, "static DWORD WINAPI ReconnectThread(LPVOID)", 400)
    return "InitTap()" in seg


def r3_new_endpoint(text: str) -> bool:
    return ("CreateThread(nullptr, 0, InitAttemptThread" in text
            and 'L"VisualDiagConnection%d"' in text
            and "for (int slot = 1; slot <= 60; ++slot)" in text)


def r4_no_unadvise(text: str) -> bool:
    body = _set_site_body(text)
    return bool(body) and "UnadviseVisualTreeChange" not in body


def r5_recommit(text: str) -> bool:
    body = _set_site_body(text)
    return all(k in body for k in (
        "AdviseVisualTreeChange(this)",
        "InterlockedExchange(&g_advised, 1)",
        "InterlockedExchange(&g_clearOnAdd, 1)",
        "InterlockedExchange(&g_dirty, 1)",
    ))


def r6_clear_tree(text: str) -> bool:
    seg = _segment(text, "if (InterlockedExchange(&g_clearOnAdd, 0))", 400)
    return "g_tree.clear()" in seg and "ResetTreeHandles();" in seg


def r7_reset_handles(text: str) -> bool:
    seg = _segment(text, "static void ResetTreeHandles() {", 700)
    return all(k in seg for k in (
        "ResetSharedBrush();",
        "g_chevronHandle = 0;",
        "g_restore.indSelectedBrush = 0;",
    ))


def r8_sentinel(text: str) -> bool:
    return all(k in text for k in (
        'L"%s\\\\reconnect.req"',
        "GetFileAttributesW(req)",
        "DeleteFileW(req)",
        "TriggerReconnect();",
    ))


def r9_clear_before_advise(text: str) -> bool:
    """全量快照是在 AdviseVisualTreeChange 内同步投递的：清树标志晚置位会把
    整棵任务栏树误清（实测：树从 n=10 变 n=1），因此必须是"先置位、后 Advise"，
    且订阅失败要把标志回滚。"""
    body = _set_site_body(text)
    i = body.find("prevClear = InterlockedExchange(&g_clearOnAdd, 1)")
    if i < 0:
        return False
    j = body.find("AdviseVisualTreeChange(this)", i)
    k = body.find("InterlockedExchange(&g_clearOnAdd, prevClear)", j)
    return 0 <= i < j < k


def r10_auto_trigger(text: str) -> bool:
    return all(k in text for k in (
        "autoAt = now + 10000",
        'GetModuleHandleW(L"ExplorerTAP.dll")',
        "[Reconnect] 自动触发",
        "autoDone = true",
    ))


def r11_reconnect_serialized(text: str) -> bool:
    return ("InterlockedExchange(&g_reconnecting, 1)" in text
            and "InterlockedExchange(&g_reconnecting, 0)" in text)


# (编号, 名称, 判定函数, 反例改写（用于 --self-test 证明断言真的会失败）)
CHECKS: list[tuple[str, str, Callable[[str], bool], tuple[str, str]]] = [
    ("R1", "重连走新线程", r1_new_thread,
     ("CreateThread(nullptr, 0, ReconnectThread", "CreateThread(nullptr, 0, Worker")),
    ("R2", "重连调用 InitTap", r2_calls_init,
     ("const bool ok = InitTap();", "const bool ok = true;")),
    ("R3", "每次尝试新线程+独立连接名", r3_new_endpoint,
     ('L"VisualDiagConnection%d"', 'L"VisualDiagConnection1"')),
    ("R4", "新连接不退订旧连接", r4_no_unadvise,
     ("HRESULT hr = v3 ? v3->AdviseVisualTreeChange(this)",
      "UnadviseVisualTreeChange(this); HRESULT hr = v3 ? v3->AdviseVisualTreeChange(this)")),
    ("R5", "新连接重新订阅并置标志", r5_recommit,
     ("InterlockedExchange(&g_clearOnAdd, 1);", "")),
    ("R6", "首个 Add 清树并丢句柄", r6_clear_tree,
     ("if (InterlockedExchange(&g_clearOnAdd, 0)) {", "if (0) {")),
    ("R7", "重置全部句柄缓存", r7_reset_handles,
     ("g_restore.indSelectedBrush = 0;", "")),
    ("R8", "哨兵检测→删除→重连", r8_sentinel,
     ("DeleteFileW(req);", "")),
    ("R9", "清树先于 Advise 且失败回滚", r9_clear_before_advise,
     ("prevClear = InterlockedExchange(&g_clearOnAdd, 1)", "prevClear = 0")),
    ("R10", "首连后定时+TTB 检测自动重连", r10_auto_trigger,
     ("autoAt = now + 10000", "autoAt = now")),
    ("R11", "重连串行化", r11_reconnect_serialized,
     ("InterlockedExchange(&g_reconnecting, 1)", "")),
]


def run_checks(text: str, verbose: bool = True) -> bool:
    ok_all = True
    for code, name, fn, _mutation in CHECKS:
        ok = fn(text)
        ok_all = ok_all and ok
        if verbose:
            print(f"[{'PASS' if ok else 'FAIL'}] {code} {name}")
    return ok_all


def self_test(text: str) -> int:
    ok = True
    print("== 正向：真实源码必须全部通过 ==")
    if not run_checks(text):
        ok = False

    print("== 反向：逐个改坏，对应对断言必须失败 ==")
    for code, name, fn, (old, new) in CHECKS:
        if old not in text:
            print(f"[FAIL] {code} 反例锚点不存在：{old!r}")
            ok = False
            continue
        broken = text.replace(old, new, 1)
        got = fn(broken)
        flag = "PASS" if not got else "FAIL"
        if got:
            ok = False
        print(f"[{flag}] {code} {name}：改坏后检查={'通过（不应通过）' if got else '失败（符合预期）'}")

    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--source", default=cpp_source_path())
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if not os.path.isfile(args.source):
        print(f"[FAIL] 源码不存在：{args.source}")
        return 1
    with open(args.source, encoding="utf-8", errors="replace") as fh:
        text = fh.read()

    if args.self_test:
        return self_test(text)

    ok = run_checks(text)
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
