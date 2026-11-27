#!/usr/bin/env python3
"""陈旧 shutdown 判定真值表自检（v1.5.5 shutdown 修复）。

背景：载荷 ShadeBarTap.cpp::IsStaleShutdown(enabled, shutdown) 决定"看到 shutdown=1
时是否忽略"。判据必须只看 enabled（宿主 shutdown() 必先写 enabled=0）——
enabled=1 && shutdown=1 只能是旧 recolor 线程的陈旧值/竞态残留。

一旦判据被改成"依赖心跳""依赖 staleLoops"或把 !=0 写成 ==1，冷开机新开窗口的
指示条就会永久不染色（2026-11-27 现场）。本脚本把判据锁进真值表，并核对 C++ 源码
表达式防止实现与被记录的行为漂移。

用法：
  python tools/check_shutdown_logic.py --self-test   # 6 组合真值表 + 源码一致性
  python tools/check_shutdown_logic.py               # 同上（默认即自检）
"""

from __future__ import annotations

import argparse
import os
import re
import sys

# 与 payload/ShadeBarTap.cpp::IsStaleShutdown 的 Python 镜像：判据只看 enabled/shutdown，
# 且用 != 0 而非 == 1（任何非 0 值都算"真"，与 C++ int 语义一致）。
def is_stale_shutdown(enabled: int, shutdown: int) -> bool:
    """返回 true 表示应忽略这次 shutdown（陈旧值）。"""
    return enabled != 0 and shutdown != 0


# 真值表：(名称, enabled, shutdown, 期望是否忽略)
# 前 4 条覆盖两个布尔量的全部组合；后 2 条专测 !=0 语义（防止被误写成 ==1）。
TRUTH_TABLE: list[tuple[str, int, int, bool]] = [
    ("(0,0) 无关", 0, 0, False),
    ("(1,0) 正常着色", 1, 0, False),
    ("(0,1) 正常退出→不忽略", 0, 1, False),
    ("(1,1) 陈旧→忽略", 1, 1, True),
    ("(2,1) enabled 非 1 仍算真→忽略", 2, 1, True),
    ("(1,-1) shutdown 非 0 仍算真→忽略", 1, -1, True),
]

# 期望的 C++ 判定式（去空白后比较），下面是源码里应出现的规范写法。
EXPECTED_CPP_EXPR = "enabled!=0&&shutdown!=0"
CPP_PREDICATE_RE = re.compile(
    r"static\s+bool\s+IsStaleShutdown\s*\([^)]*\)\s*\{"
    r"\s*return\s+(?P<expr>[^;]+);",
    re.S,
)


def cpp_source_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "..", "payload", "ShadeBarTap.cpp")


def verify_cpp_source(path: str) -> tuple[bool, str]:
    """核对 C++ 里 IsStaleShutdown 的返回式未漂移。"""
    if not os.path.isfile(path):
        return False, f"C++ 源码不存在：{path}"
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    m = CPP_PREDICATE_RE.search(text)
    if not m:
        return False, "未找到 IsStaleShutdown 的 return 表达式"
    expr = re.sub(r"\s+", "", m.group("expr"))
    if expr != EXPECTED_CPP_EXPR:
        return False, f"判定式已漂移：期望 {EXPECTED_CPP_EXPR}，实际 {expr}"
    return True, f"判定式一致：{expr}"


def self_test() -> int:
    ok = True
    for name, enabled, shutdown, expect in TRUTH_TABLE:
        got = is_stale_shutdown(enabled, shutdown)
        flag = "PASS" if got == expect else "FAIL"
        if got != expect:
            ok = False
        print(f"[{flag}] {name}: expect_ignore={expect} got_ignore={got}")

    src_ok, detail = verify_cpp_source(cpp_source_path())
    print(f"[{'PASS' if src_ok else 'FAIL'}] C++ 源码一致性: {detail}")
    if not src_ok:
        ok = False

    print(f"RESULT: {'PASS' if ok else 'FAIL'} ({len(TRUTH_TABLE)}/{len(TRUTH_TABLE)} 组合)")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true",
                    help="运行内置真值表 fixture（默认行为）")
    args = ap.parse_args()
    # 本脚本没有"读取真实日志"模式：判据是纯函数，唯一可验证对象就是真值表 + C++ 源码。
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
