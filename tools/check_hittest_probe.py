#!/usr/bin/env python3
"""HitTest 只读探针判定（冷开机 P1-P4 取证）。

载荷侧 UiProbeProc 每次触发会写一组 `[Probe]` 日志；本脚本按 BEGIN/END 切成轮次，
逐轮判定四项，再给出整体验证结论：

  P1  存在性：HitTest hr=S_OK(0) 且 count>0
  P2  坐标系：三种 RECT 中至少一种命中数>0
  P3  可用性：当场读到 name=RunningIndicator 的句柄
  P4  有效性：第 2 轮（开新窗口后）indCount 比第 1 轮增加

结论分支：
  READY_TO_IMPLEMENT  四项全中，可把主动重扫并入 PollIndicatorState
  PARTIAL             P1/P3 中但 P4 不中（或只触发了一轮，无法比较）
  FAILED              P1 不通或 P3 读不到 RunningIndicator

用法：
  python tools/check_hittest_probe.py --self-test
  python tools/check_hittest_probe.py --log "%LOCALAPPDATA%\\ShadeBar\\payload.log"
"""

from __future__ import annotations

import argparse
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

BEGIN_RE = re.compile(r"\[Probe\] ==== BEGIN tid=\d+ hwnd=0x[0-9A-Fa-f]+ ====")
END_RE = re.compile(r"\[Probe\] ==== END ====")
TRIGGER_RE = re.compile(r"\[Probe\] TRIGGER posted hr=0x([0-9A-Fa-f]+)")
P1_RE = re.compile(r"\[Probe\] P1 rect=(\d+) hr=0x([0-9A-Fa-f]+) count=(\d+)")
P2_RE = re.compile(
    r"\[Probe\] P2 rect=(\d+) label=(\S+) RECT=\((-?\d+),(-?\d+),(-?\d+),(-?\d+)\) count=(\d+)"
)
P3_RE = re.compile(r"\[Probe\] P3 rect=(\d+) scanned=(\d+) readOk=(\d+) hitLogged=(\d+)")
HIT_RE = re.compile(r"\[Probe\] HIT h=0x([0-9A-Fa-f]+) name=(\S+) type=(\S+)")
SUMMARY_RE = re.compile(r"\[Probe\] SUMMARY indCount=(\d+)")
UIA_RE = re.compile(r"\[Probe\] UIA hr=0x([0-9A-Fa-f]+) count=(-?\d+)")


def default_log() -> str:
    base = os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "ShadeBar", "payload.log")
    return os.path.expanduser("~/ShadeBar/payload.log")


def new_round() -> dict:
    return {"p1": [], "p2": [], "p3": [], "hits": [], "summary": [], "uia": [], "trigger": []}


def parse_rounds(lines: list[str]) -> list[dict]:
    """只取最后一段会话里的探针轮次；更早会话的日志不参与判定。"""
    rounds: list[dict] = []
    cur: dict | None = None
    for line in lines:
        if "==== ShadeBar 载荷启动 build=" in line:
            rounds.clear()
            cur = None
            continue
        if BEGIN_RE.search(line):
            cur = new_round()
            continue
        if cur is None:
            continue
        if END_RE.search(line):
            rounds.append(cur)
            cur = None
            continue
        m = TRIGGER_RE.search(line)
        if m:
            cur["trigger"].append(int(m.group(1), 16))
            continue
        m = P1_RE.search(line)
        if m:
            cur["p1"].append((int(m.group(1)), int(m.group(2), 16), int(m.group(3))))
            continue
        m = P2_RE.search(line)
        if m:
            cur["p2"].append((int(m.group(1)), m.group(2), int(m.group(7))))
            continue
        m = P3_RE.search(line)
        if m:
            cur["p3"].append(tuple(int(m.group(i)) for i in (1, 2, 3, 4)))
            continue
        m = HIT_RE.search(line)
        if m:
            cur["hits"].append((m.group(2), m.group(3)))
            continue
        m = SUMMARY_RE.search(line)
        if m:
            cur["summary"].append(int(m.group(1)))
            continue
        m = UIA_RE.search(line)
        if m:
            cur["uia"].append((int(m.group(1), 16), int(m.group(2))))
    return rounds


def judge(rounds: list[dict]) -> tuple[str, list[str]]:
    """返回 (结论, 逐项说明)。结论只看 P1-P4，UIA 是对照组不参与。"""
    if not rounds:
        return "FAILED", ["没有完整的 [Probe] BEGIN/END 轮次：探针未被触发或载荷未加载"]

    notes: list[str] = []
    p1_ok_rounds = 0
    p3_ok_rounds = 0
    p2_any = False
    ind_counts: list[int] = []

    for idx, r in enumerate(rounds, 1):
        p1 = [x for x in r["p1"] if x[1] == 0 and x[2] > 0]
        p1_ok = bool(p1)
        p2_hit = [x for x in r["p2"] if x[2] > 0]
        p3_hit = [h for h in r["hits"] if h[0] == "RunningIndicator"]
        p3_ok = bool(p3_hit)
        cnt = r["summary"][-1] if r["summary"] else None

        p1_ok_rounds += int(p1_ok)
        p3_ok_rounds += int(p3_ok)
        p2_any |= bool(p2_hit)
        if cnt is not None:
            ind_counts.append(cnt)

        best = p1[0] if p1 else (r["p1"][0] if r["p1"] else (0, 1, 0))
        notes.append(
            f"ROUND {idx}: P1={'PASS' if p1_ok else 'FAIL'} hr=0x{best[1]:08X} count={best[2]}; "
            f"P2={'PASS' if p2_hit else 'FAIL'} rects={[x[0] for x in p2_hit]}; "
            f"P3={'PASS' if p3_ok else 'FAIL'} RunningIndicatorHits={len(p3_hit)}; "
            f"P4 indCount={cnt}; "
            f"UIA={r['uia'][-1][1] if r['uia'] else 'n/a'}"
        )

    p4_ok = len(ind_counts) >= 2 and ind_counts[-1] > ind_counts[0]
    if len(ind_counts) < 2:
        notes.append("P4=UNKNOWN：只触发了一轮，需冷开基线一次 + 开新窗口后一次")
    else:
        notes.append(f"P4={'PASS' if p4_ok else 'FAIL'}：indCount {ind_counts[0]} -> {ind_counts[-1]}")

    all_p1_p3 = p1_ok_rounds == len(rounds) and p3_ok_rounds == len(rounds)
    if all_p1_p3 and p2_any and p4_ok:
        verdict = "READY_TO_IMPLEMENT"
    elif all_p1_p3:
        verdict = "PARTIAL"
    else:
        verdict = "FAILED"
    return verdict, notes


def verify_source_contract() -> tuple[bool, str]:
    """核对 C/Rust 两侧的探针契约未漂移（只检查，不修改）。"""
    try:
        with open(os.path.join(ROOT, "payload", "ShadeBarTap.cpp"), encoding="utf-8", errors="replace") as fh:
            cpp = fh.read()
        with open(os.path.join(ROOT, "payload", "ShadeBarIpc.h"), encoding="utf-8", errors="replace") as fh:
            hdr = fh.read()
        with open(
            os.path.join(ROOT, "crates", "shadebar-app", "src", "recolor", "mod.rs"),
            encoding="utf-8", errors="replace",
        ) as fh:
            rs = fh.read()
    except OSError as e:
        return False, f"源码读取失败：{e}"

    checks = [
        ("C++ UiProbe 枚举值", "UiProbe = 4" in cpp),
        ("C++ 使用 reserved[2] 触发", "&g_ipc->reserved[2]" in cpp),
        ("C++ 调用 HitTest", "->HitTest(rects[ri]" in cpp),
        ("C++ 只读：探针函数无 SetProperty/ClearProperty",
         "SetProperty" not in cpp[cpp.index("static void UiProbeProc()"):cpp.index("static void UiPollProc()")]
         if "static void UiProbeProc()" in cpp else False),
        ("IPC 头声明 reserved[2]", "reserved[2] = HitTest" in hdr),
        ("Rust reserved 下标为 2", "PROBE_TRIGGER_INDEX: usize = 2" in rs),
        ("Rust 有 request_probe", "pub fn request_probe" in rs),
        ("Rust 哨兵文件名一致", 'PROBE_TRIGGER_FILE: &str = "probe.trigger"' in rs),
    ]
    bad = [name for name, ok in checks if not ok]
    if bad:
        return False, "契约漂移：" + "、".join(bad)
    return True, f"契约一致（{len(checks)} 项）"


# 冷开机两轮 fixture：开新窗口后 indCount 1 -> 2，四项全中。
FIXTURE_READY = """\
[16:54:48.100] [Probe] TRIGGER posted hr=0x00000000
[16:54:48.101] [Probe] ==== BEGIN tid=9012 hwnd=0x00000000000A0123 ====
[16:54:48.102] [Probe] P1 rect=0 hr=0x00000000 count=247
[16:54:48.103] [Probe] P2 rect=0 label=window RECT=(0,1030,1920,1080) count=247
[16:54:48.104] [Probe] P2 rect=1 label=client RECT=(0,0,1920,50) count=0
[16:54:48.105] [Probe] P2 rect=2 label=icons RECT=(0,1032,960,1078) count=63
[16:54:48.106] [Probe] P3 rect=0 scanned=247 readOk=200 hitLogged=1
[16:54:48.107] [Probe] HIT h=0x19DF0010 name=RunningIndicator type=Windows.UI.Xaml.Shapes.Rectangle
[16:54:48.108] [Probe] P3 rect=1 scanned=0 readOk=0 hitLogged=0
[16:54:48.109] [Probe] P3 rect=2 scanned=63 readOk=50 hitLogged=0
[16:54:48.110] [Probe] SUMMARY indCount=1
[16:54:48.111] [Probe] UIA hr=0x00000000 count=0
[16:54:48.112] [Probe] ==== END ====
[16:55:10.100] [Probe] TRIGGER posted hr=0x00000000
[16:55:10.101] [Probe] ==== BEGIN tid=9012 hwnd=0x00000000000A0123 ====
[16:55:10.102] [Probe] P1 rect=0 hr=0x00000000 count=251
[16:55:10.103] [Probe] P2 rect=0 label=window RECT=(0,1030,1920,1080) count=251
[16:55:10.104] [Probe] HIT h=0x19DF0010 name=RunningIndicator type=Windows.UI.Xaml.Shapes.Rectangle
[16:55:10.105] [Probe] HIT h=0x19DF0020 name=RunningIndicator type=Windows.UI.Xaml.Shapes.Rectangle
[16:55:10.106] [Probe] P3 rect=0 scanned=251 readOk=203 hitLogged=2
[16:55:10.107] [Probe] SUMMARY indCount=2
[16:55:10.108] [Probe] UIA hr=0x00000000 count=0
[16:55:10.109] [Probe] ==== END ====
"""

# P1/P3 成立但数量不增：只读到旧树快照，按架构 §4.2 应转 GetCollectionElements。
FIXTURE_PARTIAL = """\
[16:54:48.100] [Probe] ==== BEGIN tid=9012 hwnd=0x00000000000A0123 ====
[16:54:48.102] [Probe] P1 rect=0 hr=0x00000000 count=247
[16:54:48.103] [Probe] P2 rect=0 label=window RECT=(0,1030,1920,1080) count=247
[16:54:48.107] [Probe] HIT h=0x19DF0010 name=RunningIndicator type=Windows.UI.Xaml.Shapes.Rectangle
[16:54:48.110] [Probe] SUMMARY indCount=1
[16:54:48.112] [Probe] ==== END ====
[16:55:10.101] [Probe] ==== BEGIN tid=9012 hwnd=0x00000000000A0123 ====
[16:55:10.102] [Probe] P1 rect=0 hr=0x00000000 count=247
[16:55:10.103] [Probe] HIT h=0x19DF0010 name=RunningIndicator type=Windows.UI.Xaml.Shapes.Rectangle
[16:55:10.107] [Probe] SUMMARY indCount=1
[16:55:10.109] [Probe] ==== END ====
"""

# HitTest 本身不通：hr 失败 / count=0 / 读不到 RunningIndicator。
FIXTURE_FAILED = """\
[16:54:48.101] [Probe] ==== BEGIN tid=9012 hwnd=0x00000000000A0123 ====
[16:54:48.102] [Probe] P1 rect=0 hr=0x80004005 count=0
[16:54:48.103] [Probe] P2 rect=0 label=window RECT=(0,1030,1920,1080) count=0
[16:54:48.107] [Probe] P3 rect=0 scanned=0 readOk=0 hitLogged=0
[16:54:48.110] [Probe] SUMMARY indCount=0
[16:54:48.112] [Probe] ==== END ====
"""


def self_test() -> int:
    ok = True
    cases = [
        ("READY_TO_IMPLEMENT", FIXTURE_READY),
        ("PARTIAL", FIXTURE_PARTIAL),
        ("FAILED", FIXTURE_FAILED),
    ]
    for expect, fixture in cases:
        got, notes = judge(parse_rounds(fixture.splitlines()))
        flag = "PASS" if got == expect else "FAIL"
        ok &= got == expect
        print(f"[{flag}] fixture 期望={expect} 实际={got}（{len(notes)} 项说明）")
        for n in notes:
            print(f"       {n}")

    src_ok, detail = verify_source_contract()
    print(f"[{'PASS' if src_ok else 'FAIL'}] 源码契约: {detail}")
    ok &= src_ok
    print(f"RESULT: {'PASS' if ok else 'FAIL'} ({len(cases)}/{len(cases)} fixture)")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", default=default_log(), help="载荷日志路径")
    ap.add_argument("--self-test", action="store_true", help="运行内置 fixture（不读真实日志）")
    args = ap.parse_args()
    if args.self_test:
        return self_test()

    if not os.path.isfile(args.log):
        print(f"[FAIL] 日志不存在：{args.log}")
        print("提示：在 %LOCALAPPDATA%\\ShadeBar\\ 创建 probe.trigger 触发一次探针后再看日志。")
        return 1
    with open(args.log, encoding="utf-8", errors="replace") as fh:
        rounds = parse_rounds(fh.read().splitlines())
    verdict, notes = judge(rounds)
    for n in notes:
        print(n)
    print(f"RESULT: {verdict} ({len(rounds)} 轮探针)")
    return 0 if verdict == "READY_TO_IMPLEMENT" else 1


if __name__ == "__main__":
    sys.exit(main())
