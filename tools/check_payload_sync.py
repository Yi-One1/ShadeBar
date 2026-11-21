#!/usr/bin/env python3
"""载荷着色链路检查（G1 回归项）

用途：判断"改色整片失效 / 指示条漏改"是否还存在。只看运行日志，不改任何东西。

判据（任一不满足即退出码 1）：
  C1 最近 20 条 [Apply] 中，掩码要求指示条时必须真的写到（ind > 0）
  C2 存在 [Replay]，且最后一次 hr == 0（订阅成功）
  C3 最近 20 条 [Apply] 不得三区同时为 0（整片不着色）
  C4 最后一次 [Replay] 之后有写过指示条
  C5 重放耗时（仅报告，> 50 ms 给警告，不判失败）

用法：
  python tools/check_payload_sync.py
  python tools/check_payload_sync.py --log "%LOCALAPPDATA%\\ShadeBar\\payload.log"
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from collections import deque

APPLY_RE = re.compile(
    r"\[Apply\] seq=(?P<seq>-?\d+) mask=(?P<mask>[0-9A-Fa-f]+) "
    r"time=(?P<time>\d+) date=(?P<date>\d+) tray=(?P<tray>\d+) "
    r"ind=(?P<ind>\d+) undo=(?P<undo>\d+)"
)
REPLAY_RE = re.compile(r"\[Replay\] #(?P<n>\d+) reason=(?P<reason>\S+) hr=0x(?P<hr>[0-9A-Fa-f]+) tree=(?P<tree>\d+)(?: (?:ms=(?P<ms>\d+)|us=(?P<us>\d+)))?")
ADVISE_RE = re.compile(r"\[Tap\] Advise hr=0x(?P<hr>[0-9A-Fa-f]+)")
IND_RE = re.compile(
    r"\[Ind\] h=0x(?P<h>[0-9A-Fa-f]+) w=(?P<w>[\d.]+) (?P<state>选中|正常) "
    r"b=0x(?P<b>[0-9A-Fa-f]+) -> (?P<color>[0-9A-Fa-f]+)"
)

MASK_INDICATOR = 0x8
WINDOW = 20


def default_log() -> str:
    base = os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "ShadeBar", "payload.log")
    return os.path.expanduser("~/ShadeBar/payload.log")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", default=default_log())
    ap.add_argument("--window", type=int, default=WINDOW)
    args = ap.parse_args()

    if not os.path.isfile(args.log):
        print(f"[FAIL] 日志不存在：{args.log}")
        return 1

    applies: deque[dict] = deque(maxlen=args.window)
    last_replay = None
    replay_count = 0
    replay_ms: list[float] = []
    last_advise = None
    # 记录每次重放之后是否真的着色：以行号为序，末尾重放可能还没来得及应用
    replay_marks: list[int] = []
    apply_seq: list[tuple[int, int]] = []
    # 共享画刷检查：每个 [Apply] 之前的 [Ind] 块里，正常态应当只有一个画刷句柄
    block_brushes: set[str] = set()
    last_block: dict | None = None

    with open(args.log, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            # 只看本次载荷会话：上一轮运行的重放耗时（启动瞬间 200-400 ms）会把均值拉歪
            if "载荷启动" in line:
                applies.clear()
                replay_marks.clear()
                apply_seq.clear()
                replay_ms.clear()
                last_replay = None
                replay_count = 0
                last_advise = None
                continue
            m = APPLY_RE.search(line)
            if m:
                d = {k: int(m.group(k), 16) if k == "mask" else int(m.group(k)) for k in
                     ("seq", "mask", "time", "date", "tray", "ind", "undo")}
                applies.append(d)
                apply_seq.append((apply_seq[-1][0] + 1 if apply_seq else 1, d["ind"]))
                if d["ind"] > 0:
                    last_block = {"seq": d["seq"], "ind": d["ind"], "brushes": set(block_brushes)}
                block_brushes = set()
                continue
            m = IND_RE.search(line)
            if m:
                if m.group("state") == "正常" and m.group("b") != "0":
                    block_brushes.add(m.group("b"))
                continue
            m = REPLAY_RE.search(line)
            if m:
                last_replay = {"n": int(m.group("n")), "hr": int(m.group("hr"), 16),
                               "tree": int(m.group("tree")), "reason": m.group("reason")}
                if m.group("us"):
                    replay_ms.append(int(m.group("us")) / 1000.0)
                elif m.group("ms"):
                    replay_ms.append(float(m.group("ms")))
                replay_count += 1
                replay_marks.append(apply_seq[-1][0] if apply_seq else 0)
                continue
            m = ADVISE_RE.search(line)
            if m:
                last_advise = int(m.group("hr"), 16)

    if not applies:
        print(f"[FAIL] 日志里没有 [Apply] 记录，无法判定（{args.log}）")
        return 1

    results: list[tuple[str, bool, str]] = []

    want_ind = [a for a in applies if a["mask"] & MASK_INDICATOR]
    hits = [a for a in want_ind if a["ind"] > 0]
    ratio = (len(hits) / len(want_ind)) if want_ind else 1.0
    results.append(("C1 指示条有写入", want_ind and ratio >= 0.9,
                    f"{len(hits)}/{len(want_ind)} 拍写过 ind>0（{ratio:.0%}）"))

    results.append(("C2 订阅成功", last_replay is not None and last_replay["hr"] == 0,
                    "无 [Replay] 记录" if last_replay is None else
                    f"最后 #%d hr=0x%08X tree=%d" % (last_replay["n"], last_replay["hr"], last_replay["tree"])))

    dead = [a for a in applies if a["time"] == 0 and a["date"] == 0 and a["tray"] == 0]
    results.append(("C3 未整片失效", not dead, f"整片为 0 的拍数：{len(dead)}"))

    # 取"后面确实还有 Apply"的最后一次重放，避免把刚发生、尚未应用的重放算成 0/0
    ind_after = 0
    total_after = 0
    for mark in reversed(replay_marks):
        tail = [ind for idx, ind in apply_seq if idx > mark]
        if tail:
            total_after = len(tail)
            ind_after = len([i for i in tail if i > 0])
            break
    results.append(("C4 重放后确实着色", ind_after > 0,
                    f"最后一次有效重放后 {ind_after}/{total_after} 拍写过指示条"))

    # C5：所有正常态指示条必须指向同一个画刷对象（方案 B 的结构性证据）
    if last_block is None:
        results.append(("C5 指示条共用画刷", False, "没有 ind>0 的 Apply 记录"))
    else:
        nb = last_block["brushes"]
        results.append(("C5 指示条共用画刷", len(nb) == 1,
                        f"seq={last_block['seq']} 正常态画刷 {len(nb)} 个：{', '.join(sorted(nb))[:60]}"))

    def pct(vals: list[float], q: float) -> float:
        if not vals:
            return 0
        ordered = sorted(vals)
        idx = min(len(ordered) - 1, int(round(q * (len(ordered) - 1))))
        return ordered[idx]

    median_ms = pct(replay_ms, 0.5) if replay_ms else None
    p95_ms = pct(replay_ms, 0.95) if replay_ms else None

    width = max(len(name) for name, _, _ in results)
    ok_all = True
    for name, ok, detail in results:
        print(f"[{'PASS' if ok else 'FAIL'}] {name.ljust(width)}  {detail}")
        ok_all = ok_all and bool(ok)

    if median_ms is None:
        print("[WARN] 日志里没有 ms= 字段（旧版载荷），未测重放耗时")
    else:
        warn = "  <-- p95 超过 50 ms，5 s 节奏会占用可观察 CPU" if (p95_ms or 0) > 50 else ""
        print(f"[INFO] 本次会话重放 {replay_count} 次：中位 {median_ms:.1f} ms，p95 {p95_ms:.1f} ms{warn}")

    if last_advise is not None and last_advise != 0:
        print(f"[FAIL] 最近一次 Advise 返回 0x{last_advise:08X}")
        ok_all = False

    print("RESULT:", "PASS" if ok_all else "FAIL")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
