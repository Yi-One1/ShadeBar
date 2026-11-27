#!/usr/bin/env python3
"""正常态指示条冷开机着色取证分析（真问题第 1 组）。

现象：冷开机后正常态指示条不改色，重启软件无效，重启 explorer 才恢复。
现有 [Ind] 日志每行都带写入目标色（-> AARRGGBB）和写后回读（rb=AARRGGBB），
足以把根因定位到三种之一，无需再加诊断日志：
  - NOT_LOADED    会话里指示条元素数长期为 0（元素没进树 / 载荷没注入）
  - WRONG_TARGET  有写入，但下发的目标色不是配置的正常态色（采样判定错）
  - WRITE_LOST    目标色对，但回读 rb != 目标色（写进属性槽被系统覆盖）
  - OK            目标色对、回读也对（载荷侧正常；若视觉仍不对，问题在别处）

只看最后一个载荷启动会话。用法：
  python tools/check_indicator_coldboot.py
  python tools/check_indicator_coldboot.py --log <path>
  python tools/check_indicator_coldboot.py --target CCFFFFFF   # 指定期望的正常态色
  python tools/check_indicator_coldboot.py --self-test
"""

from __future__ import annotations

import argparse
import os
import re
import sys

START_RE = re.compile(r"==== ShadeBar 载荷启动 build=(?P<build>.+?) pid=(?P<pid>\d+) ====")
APPLY_RE = re.compile(
    r"\[Apply\] seq=(?P<seq>-?\d+) mask=(?P<mask>[0-9A-Fa-f]+) "
    r"time=(?P<time>\d+) date=(?P<date>\d+) tray=(?P<tray>\d+) "
    r"ind=(?P<ind>\d+) undo=(?P<undo>\d+)"
)
# [Ind] h=0x... w=6.0 正常 b=0x... -> CCFFFFFF (elem=1 brush=0 all=0 rb=CCFFFFFF ok=1)
IND_RE = re.compile(
    r"\[Ind\] h=0x(?P<h>[0-9A-Fa-f]+) w=(?P<w>[\d.]+) (?P<state>正常|选中) "
    r"b=0x(?P<b>[0-9A-Fa-f]+) -> (?P<want>[0-9A-Fa-f]{8}) "
    r"\(elem=(?P<elem>\d+) brush=(?P<brush>\d+) all=(?P<all>\d+) "
    r"rb=(?P<rb>[0-9A-Fa-f]{8}) ok=(?P<ok>\d+)\)"
)
TREE_RE = re.compile(
    r"\[Tree\] n=(?P<n>\d+) time=(?P<time>\d+) date=(?P<date>\d+) "
    r"tray=(?P<tray>\d+) ind=(?P<ind>\d+) other=(?P<other>\d+)"
)

DEFAULT_TARGET = "CCFFFFFF"  # S-52 白色 + S-60 80% alpha（当前配置）


def default_log() -> str:
    base = os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "ShadeBar", "payload.log")
    return os.path.expanduser("~/ShadeBar/payload.log")


def analyze(lines: list[str], target: str) -> dict:
    sessions: list[dict] = []
    cur: dict | None = None
    for line in lines:
        m = START_RE.search(line)
        if m:
            cur = {"build": m.group("build"), "pid": m.group("pid"),
                   "normal": [], "apply_ind": [], "tree_ind": [], "indfail": 0}
            sessions.append(cur)
            continue
        if cur is None:
            continue
        m = IND_RE.search(line)
        if m and m.group("state") == "正常":
            cur["normal"].append({
                "want": m.group("want").upper(), "rb": m.group("rb").upper(),
                "ok": int(m.group("ok")), "w": float(m.group("w")),
            })
            continue
        m = APPLY_RE.search(line)
        if m:
            cur["apply_ind"].append(int(m.group("ind")))
            continue
        m = TREE_RE.search(line)
        if m:
            cur["tree_ind"].append(int(m.group("ind")))
            continue
        if "[IndFail]" in line:
            cur["indfail"] += 1
            continue

    if not sessions:
        return {"root_cause": "NO_SESSION", "detail": "日志里没有载荷启动记录"}

    s = sessions[-1]
    n = len(s["normal"])
    base = {"build": s["build"], "pid": s["pid"], "sessions": len(sessions),
            "normal_writes": n, "indfail": s["indfail"],
            "apply_ind_max": max(s["apply_ind"]) if s["apply_ind"] else 0,
            "tree_ind_max": max(s["tree_ind"]) if s["tree_ind"] else 0,
            "target": target.upper()}

    if n == 0 or (s["apply_ind"] and max(s["apply_ind"]) == 0):
        return {**base, "root_cause": "NOT_LOADED",
                "detail": "会话内正常态指示条写入数为 0：元素没进树 / 载荷没生效"}

    tgt = target.upper()
    want_ok = [r for r in s["normal"] if r["want"] == tgt]
    want_bad = [r for r in s["normal"] if r["want"] != tgt]
    if not want_ok and want_bad:
        from collections import Counter
        c = Counter(r["want"] for r in want_bad)
        return {**base, "root_cause": "WRONG_TARGET",
                "detail": f"下发目标色不是期望的 {tgt}：实际 {dict(c)}（采样判定错）",
                "sample": want_bad[0]}

    lost = [r for r in want_ok if r["rb"] != r["want"]]
    if lost:
        return {**base, "root_cause": "WRITE_LOST",
                "detail": f"目标色对但回读不符：{lost[0]['want']} -> {lost[0]['rb']}（写后被覆盖）",
                "sample": lost[0]}

    return {**base, "root_cause": "OK",
            "detail": f"正常态指示条目标色与回读均正确（{len(want_ok)}/{n} 拍）",
            "sample": want_ok[0]}


SELF_FIXTURES: list[tuple[str, list[str], str, str]] = [
    ("无会话", ["一行日志"], DEFAULT_TARGET, "NO_SESSION"),
    ("没进树", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Apply] seq=1 mask=F time=1 date=1 tray=5 ind=0 undo=0",
    ], DEFAULT_TARGET, "NOT_LOADED"),
    ("下发色错", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Ind] h=0x1 w=6.0 正常 b=0x2 -> CC000000 (elem=1 brush=0 all=0 rb=CC000000 ok=1)",
        "[Apply] seq=1 mask=F time=1 date=1 tray=5 ind=1 undo=0",
    ], DEFAULT_TARGET, "WRONG_TARGET"),
    ("写后丢失", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Ind] h=0x1 w=6.0 正常 b=0x2 -> CCFFFFFF (elem=1 brush=0 all=0 rb=CC000000 ok=1)",
        "[Apply] seq=1 mask=F time=1 date=1 tray=5 ind=1 undo=0",
    ], DEFAULT_TARGET, "WRITE_LOST"),
    ("正常", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Ind] h=0x1 w=6.0 正常 b=0x2 -> CCFFFFFF (elem=1 brush=0 all=0 rb=CCFFFFFF ok=1)",
        "[Apply] seq=1 mask=F time=1 date=1 tray=5 ind=1 undo=0",
    ], DEFAULT_TARGET, "OK"),
    ("多会话取最后", [
        "==== ShadeBar 载荷启动 build=OLD pid=1 ====",
        "[Ind] h=0x1 w=6.0 正常 b=0x2 -> CC000000 (elem=1 brush=0 all=0 rb=CC000000 ok=1)",
        "==== ShadeBar 载荷启动 build=NEW pid=2 ====",
        "[Ind] h=0x1 w=6.0 正常 b=0x2 -> CCFFFFFF (elem=1 brush=0 all=0 rb=CCFFFFFF ok=1)",
    ], DEFAULT_TARGET, "OK"),
]


def self_test() -> int:
    ok = True
    for name, lines, target, expect in SELF_FIXTURES:
        got = analyze(lines, target)["root_cause"]
        flag = "PASS" if got == expect else "FAIL"
        if got != expect:
            ok = False
        print(f"[{flag}] {name}: expect={expect} got={got}")
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", default=default_log())
    ap.add_argument("--target", default=DEFAULT_TARGET)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if not os.path.isfile(args.log):
        print(f"[FAIL] 日志不存在：{args.log}")
        return 1
    with open(args.log, encoding="utf-8", errors="replace") as fh:
        result = analyze(fh.readlines(), args.target)

    if args.json:
        import json
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(f"root_cause    : {result['root_cause']}")
        print(f"build         : {result.get('build')}")
        print(f"pid           : {result.get('pid')}")
        print(f"正常态写入拍数: {result.get('normal_writes')}")
        print(f"IndFail 次数  : {result.get('indfail')}")
        print(f"apply_ind max : {result.get('apply_ind_max')}")
        print(f"tree_ind max  : {result.get('tree_ind_max')}")
        print(f"目标色        : {result.get('target')}")
        print(f"detail        : {result['detail']}")
        if "sample" in result:
            print(f"sample        : {result['sample']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
