#!/usr/bin/env python3
"""载荷着色链路检查（G1 回归项）

用途：判断"改色整片失效 / 指示条漏改"是否还存在。只看运行日志，不改任何东西。

判据（任一不满足即退出码 1）：
  C1 最近 20 条 [Apply] 中，掩码要求指示条时必须真的写到（ind > 0）
  C2 订阅成功（[Tap] Advise hr == 0）
  C3 最近 20 条 [Apply] 不得三区同时为 0（整片不着色）
  C4 订阅成功后确实写过指示条
  C5 所有正常态指示条共用一个画刷
  C6 掩码要求时间/日期/托盘时，必须真的写到对应元素
  C7 不发起自动重放（v1.5.2 起只订阅一次；出现 [Replay] 说明跑的是旧载荷）

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

MASK_TIME = 0x1
MASK_DATE = 0x2
MASK_TRAY = 0x4
MASK_INDICATOR = 0x8
WINDOW = 20
# 载荷日志行首时间戳：[HH:MM:SS.mmm]（v1.5.1 起）
TS_RE = re.compile(r"^\[(\d{2}):(\d{2}):(\d{2})\.(\d{3})\]")


def ts_ms(line: str) -> int | None:
    m = TS_RE.match(line)
    if not m:
        return None
    h, mi, s, ms = (int(g) for g in m.groups())
    return ((h * 60 + mi) * 60 + s) * 1000 + ms


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
    replay_count = 0
    replay_ms: list[float] = []
    last_advise = None
    # 订阅成功那一刻的 apply 序号；C4 用它判断"之后有没有真的着色"
    advise_mark: int | None = None
    apply_seq: list[tuple[int, int]] = []
    # 共享画刷检查：每个 [Apply] 之前的 [Ind] 块里，正常态应当只有一个画刷句柄
    block_brushes: set[str] = set()
    last_block: dict | None = None

    with open(args.log, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            # 只看本次载荷会话：更早的会话日志不能参与判定
            if "载荷启动" in line:
                applies.clear()
                apply_seq.clear()
                replay_ms.clear()
                replay_count = 0
                last_advise = None
                advise_mark = None
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
                if m.group("us"):
                    replay_ms.append(int(m.group("us")) / 1000.0)
                elif m.group("ms"):
                    replay_ms.append(float(m.group("ms")))
                replay_count += 1
                continue
            m = ADVISE_RE.search(line)
            if m:
                last_advise = int(m.group("hr"), 16)
                if last_advise == 0:
                    advise_mark = apply_seq[-1][0] if apply_seq else 0

    if not applies:
        print(f"[FAIL] 日志里没有 [Apply] 记录，无法判定（{args.log}）")
        return 1

    results: list[tuple[str, bool, str]] = []

    want_ind = [a for a in applies if a["mask"] & MASK_INDICATOR]
    hits = [a for a in want_ind if a["ind"] > 0]
    ratio = (len(hits) / len(want_ind)) if want_ind else 1.0
    results.append(("C1 指示条有写入", want_ind and ratio >= 0.9,
                    f"{len(hits)}/{len(want_ind)} 拍写过 ind>0（{ratio:.0%}）"))

    results.append(("C2 订阅成功", last_advise == 0,
                    "无 [Tap] Advise 记录" if last_advise is None else
                    f"最后 Advise hr=0x{last_advise:08X}"))

    dead = [a for a in applies if a["time"] == 0 and a["date"] == 0 and a["tray"] == 0]
    results.append(("C3 未整片失效", not dead, f"整片为 0 的拍数：{len(dead)}"))

    # C4：订阅成功之后必须真的把指示条写进任务栏。
    if advise_mark is None:
        results.append(("C4 订阅后确实着色", False, "没有成功的订阅记录"))
    else:
        tail = [ind for idx, ind in apply_seq if idx > advise_mark]
        ind_after = len([i for i in tail if i > 0])
        results.append(("C4 订阅后确实着色", ind_after > 0,
                        f"订阅后 {ind_after}/{len(tail)} 拍写过指示条"))

    # C6：掩码要求时间/日期/托盘时，必须真的写到对应元素。
    # 这条专门拦"元素没进树"——v1.3.2 的托盘元素曾被误认成 AdaptiveTextBlock，
    # 写得进但染不到任何可见字形，日志里只看 [Apply] 全是成功，掩盖了问题。
    c6_parts = []
    c6_ok = True
    for bit, key, label in ((MASK_TIME, "time", "时间"), (MASK_DATE, "date", "日期"),
                            (MASK_TRAY, "tray", "托盘")):
        want = [a for a in applies if a["mask"] & bit]
        hit = [a for a in want if a[key] > 0]
        ratio = (len(hit) / len(want)) if want else 1.0
        c6_ok = c6_ok and ratio >= 0.9
        c6_parts.append(f"{label} {len(hit)}/{len(want)}")
    results.append(("C6 时间/日期/托盘有写入", c6_ok, "，".join(c6_parts)))

    # C5：所有正常态指示条必须指向同一个画刷对象（方案 B 的结构性证据）
    if last_block is None:
        results.append(("C5 指示条共用画刷", False, "没有 ind>0 的 Apply 记录"))
    else:
        nb = last_block["brushes"]
        results.append(("C5 指示条共用画刷", len(nb) == 1,
                        f"seq={last_block['seq']} 正常态画刷 {len(nb)} 个：{', '.join(sorted(nb))[:60]}"))

    # C7：v1.5.2 起只订阅一次，不再自动重放。日志里出现 [Replay] 说明
    # explorer 里跑的还是旧载荷（或代码回退），必须当作失败处理。
    results.append(("C7 不发起自动重放", replay_count == 0,
                    f"本次会话 [Replay] {replay_count} 次（期望 0）"))

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
        print(f"[INFO] 本次会话未发生重放（v1.5.2 设计如此），[Replay] 计数 {replay_count}")
    else:
        warn = "  <-- p95 超过 50 ms，说明跑的是旧载荷" if (p95_ms or 0) > 50 else ""
        print(f"[INFO] 本次会话重放 {replay_count} 次：中位 {median_ms:.1f} ms，p95 {p95_ms:.1f} ms{warn}")

    if last_advise is not None and last_advise != 0:
        print(f"[FAIL] 最近一次 Advise 返回 0x{last_advise:08X}")
        ok_all = False

    print("RESULT:", "PASS" if ok_all else "FAIL")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
