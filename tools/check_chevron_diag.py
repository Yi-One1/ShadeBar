#!/usr/bin/env python3
"""托盘箭头（ChevronIconView）着色取证分析（第 1 组，v1.5.5）。

背景：v1.5.4 声称"给箭头直接写 Foreground"已修复冷开机箭头不着色，但用户真机
反复遇到。旧日志的 `[Tray+] chevron Foreground ok idx=876` 只证明"Foreground 属性槽
存在"，不能证明"WinComp 内部绘制的箭头真的取了这个颜色"，也不能证明"箭头进了树"。

本脚本配合载荷新增的三类 `[Diag]` 日志，把一次冷开机会话判定到具体根因分支：
  - ADD    事件：chevron 是否进树；REMOVE：是否中途被移出
  - RB     记录：写后回读的实际颜色 vs 目标颜色（区分"写没写进去"与"写了不生效"）
  - 载荷启动 build：确认跑的是带诊断的版本，不是 explorer 里的旧载荷副本

判定分支（--json 输出 root_cause 字段）：
  NO_SESSION         日志里没有本次诊断版载荷启动记录
  NOT_IN_TREE        会话内没有 chevron ADD —— Add 事件没到，元素没进树（假设 A）
  REMOVED            chevron 进过树但之后被 REMOVE 且未再 ADD
  WRITE_MISMATCH     回读颜色与目标色不一致 —— 写进了但被覆盖/WinComp 不读（假设 B）
  WRITE_OK           ADD 存在且旧版 RB 回读颜色一致（v1.5.4 诊断版）
  FIX_OK             v1.5.5+ 修复版：改画刷对象 Color 后回读颜色一致（修复生效判据）
  FIX_FAILED         v1.5.5+ 修复版：改画刷后回读仍不符（修复未生效，需继续查）

用法：
  python tools/check_chevron_diag.py
  python tools/check_chevron_diag.py --log "%LOCALAPPDATA%\\ShadeBar\\payload.log"
  python tools/check_chevron_diag.py --json
  python tools/check_chevron_diag.py --self-test     # 内置 fixture 自测，不读真实日志
"""

from __future__ import annotations

import argparse
import os
import re
import sys

# build 是完整时间戳（形如 "Nov 27 2026 15:09:16"），中间有空格，不能用 \S+
START_RE = re.compile(r"==== ShadeBar 载荷启动 build=(?P<build>.+?) pid=(?P<pid>\d+) ====")
ADD_RE = re.compile(r"\[Diag\] chevron ADD #(?P<n>\d+) h=0x(?P<h>[0-9A-Fa-f]+)")
REMOVE_RE = re.compile(r"\[Diag\] chevron REMOVE #(?P<n>\d+) h=0x(?P<h>[0-9A-Fa-f]+)")
RB_RE = re.compile(
    r"\[Diag\] chevron RB want=(?P<want>[0-9A-Fa-f]{8}) got=(?P<got>[0-9A-Fa-f]{8}) "
    r"rb=(?P<rb>[01]) adds=(?P<adds>\d+) rems=(?P<rems>\d+)"
)
# v1.5.5 修复后的写后回读日志：改画刷对象 Color 之后读回的实际颜色
FIX_RE = re.compile(
    r"\[Diag\] chevron FIX want=(?P<want>[0-9A-Fa-f]{8}) got=(?P<got>[0-9A-Fa-f]{8}) "
    r"brush=(?P<brush>[01]) rb=(?P<rb>[01])"
)


def default_log() -> str:
    base = os.environ.get("LOCALAPPDATA")
    if base:
        return os.path.join(base, "ShadeBar", "payload.log")
    return os.path.expanduser("~/ShadeBar/payload.log")


def analyze(lines: list[str]) -> dict:
    """分析一个日志（可含多会话），只取最后一个载荷启动会话。"""
    sessions: list[dict] = []
    cur: dict | None = None
    for line in lines:
        m = START_RE.search(line)
        if m:
            cur = {"build": m.group("build"), "pid": m.group("pid"),
                   "adds": [], "removes": [], "rb": [], "fix": []}
            sessions.append(cur)
            continue
        if cur is None:
            continue
        m = ADD_RE.search(line)
        if m:
            cur["adds"].append(int(m.group("n")))
            continue
        m = REMOVE_RE.search(line)
        if m:
            cur["removes"].append(int(m.group("n")))
            continue
        m = RB_RE.search(line)
        if m:
            cur["rb"].append({
                "want": m.group("want").upper(),
                "got": m.group("got").upper(),
                "rb": int(m.group("rb")),
                "adds": int(m.group("adds")),
                "rems": int(m.group("rems")),
            })
            continue
        m = FIX_RE.search(line)
        if m:
            cur["fix"].append({
                "want": m.group("want").upper(),
                "got": m.group("got").upper(),
                "brush": int(m.group("brush")),
                "rb": int(m.group("rb")),
            })
            continue

    if not sessions:
        return {"root_cause": "NO_SESSION", "sessions": 0,
                "detail": "日志里没有载荷启动记录（可能没注入，或跑的是旧载荷）"}

    s = sessions[-1]
    base = {"sessions": len(sessions), "build": s["build"], "pid": s["pid"],
            "add_count": len(s["adds"]), "remove_count": len(s["removes"]),
            "rb_count": len(s["rb"]), "fix_count": len(s["fix"])}

    if not s["adds"]:
        return {**base, "root_cause": "NOT_IN_TREE",
                "detail": "本次会话没有 chevron ADD 事件：箭头未进诊断树（假设 A）"}

    if s["removes"] and len(s["removes"]) >= len(s["adds"]):
        return {**base, "root_cause": "REMOVED",
                "detail": "chevron 进树后被 REMOVE 且未再 ADD：句柄失效"}

    # 修复版（v1.5.5+）以 chevron FIX 回读为准：改画刷对象 Color 后读回必须 == 目标色。
    if s["fix"]:
        fix_good = [r for r in s["fix"] if r["rb"] == 1 and r["want"] == r["got"]]
        if fix_good:
            return {**base, "root_cause": "FIX_OK",
                    "detail": f"修复生效：改画刷后回读颜色一致（{len(fix_good)}/{len(s['fix'])} 拍）",
                    "sample": fix_good[0]}
        sample = s["fix"][0]
        return {**base, "root_cause": "FIX_FAILED",
                "detail": f"修复未生效：改画刷后回读仍不符 want={sample['want']} got={sample['got']}",
                "sample": sample}

    good = [r for r in s["rb"] if r["rb"] == 1 and r["want"] == r["got"]]
    bad = [r for r in s["rb"] if r["rb"] == 1 and r["want"] != r["got"]]
    if not s["rb"]:
        return {**base, "root_cause": "NO_RB",
                "detail": "chevron 已进树，但没有回读日志（载荷可能是旧版或诊断未编译进去）"}

    if bad and not good:
        sample = bad[0]
        return {**base, "root_cause": "WRITE_MISMATCH",
                "detail": f"回读颜色与目标色不一致：want={sample['want']} got={sample['got']}（假设 B）",
                "sample": sample}

    return {**base, "root_cause": "WRITE_OK",
            "detail": f"chevron 已进树且回读颜色一致（{len(good)}/{len(s['rb'])} 拍）",
            "sample": good[0] if good else s["rb"][-1]}


SELF_FIXTURES: list[tuple[str, list[str], str]] = [
    ("无会话", ["随便一行日志"], "NO_SESSION"),
    ("没进树", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Apply] seq=1 mask=F time=1 date=1 tray=5 ind=0 undo=0",
    ], "NOT_IN_TREE"),
    ("进树后被移除", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron REMOVE #1 h=0xABC",
    ], "REMOVED"),
    ("写后回读不一致", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron RB want=FFFFFFFF got=FF000000 rb=1 adds=1 rems=0",
    ], "WRITE_MISMATCH"),
    ("写入正常", [
        "==== ShadeBar 载荷启动 build=X pid=1 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron RB want=FFFFFFFF got=FFFFFFFF rb=1 adds=1 rems=0",
    ], "WRITE_OK"),
    ("多会话只取最后", [
        "==== ShadeBar 载荷启动 build=OLD pid=1 ====",
        "[Diag] chevron RB want=FFFFFFFF got=FF000000 rb=1 adds=1 rems=0",
        "==== ShadeBar 载荷启动 build=NEW pid=2 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron RB want=FFFFFFFF got=FFFFFFFF rb=1 adds=1 rems=0",
    ], "WRITE_OK"),
    ("修复生效", [
        "==== ShadeBar 载荷启动 build=NEW pid=3 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron FIX want=FFFFFFFF got=FFFFFFFF brush=1 rb=1",
    ], "FIX_OK"),
    ("修复未生效", [
        "==== ShadeBar 载荷启动 build=NEW pid=3 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron FIX want=FFFFFFFF got=00000000 brush=1 rb=1",
    ], "FIX_FAILED"),
    ("FIX 优先于 RB", [
        "==== ShadeBar 载荷启动 build=NEW pid=3 ====",
        "[Diag] chevron ADD #1 h=0xABC parent=0xDEF",
        "[Diag] chevron RB want=FFFFFFFF got=00000000 rb=1 adds=1 rems=0",
        "[Diag] chevron FIX want=FFFFFFFF got=FFFFFFFF brush=1 rb=1",
    ], "FIX_OK"),
]


def self_test() -> int:
    ok = True
    for name, lines, expect in SELF_FIXTURES:
        got = analyze(lines)["root_cause"]
        flag = "PASS" if got == expect else "FAIL"
        if got != expect:
            ok = False
        print(f"[{flag}] {name}: expect={expect} got={got}")
    print("RESULT:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", default=default_log())
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if not os.path.isfile(args.log):
        print(f"[FAIL] 日志不存在：{args.log}")
        return 1
    with open(args.log, encoding="utf-8", errors="replace") as fh:
        result = analyze(fh.readlines())

    if args.json:
        import json
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(f"root_cause : {result['root_cause']}")
        print(f"sessions   : {result.get('sessions')}")
        print(f"build      : {result.get('build')}")
        print(f"pid        : {result.get('pid')}")
        print(f"chevron ADD: {result.get('add_count')}  REMOVE: {result.get('remove_count')}")
        print(f"回读记录   : {result.get('rb_count')}")
        print(f"detail     : {result['detail']}")
        if "sample" in result:
            print(f"sample     : {result['sample']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
