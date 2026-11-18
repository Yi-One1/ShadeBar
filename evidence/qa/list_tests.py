#!/usr/bin/env python3
"""List every unit-test fn inside #[cfg(test)] regions, for boundary-path coverage review."""
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CRATES = os.path.join(ROOT, "crates")


def test_region_marks(lines):
    marks = [False] * len(lines)
    i = 0
    pending = False
    while i < len(lines):
        s = lines[i].strip()
        if s.startswith("#[") and "cfg" in s and re.search(r"\btest\b", s):
            pending = True
            i += 1
            continue
        if s.startswith("#[test]"):
            pending = True
            i += 1
            continue
        if pending:
            if s.startswith("#[") or s == "" or s.startswith("//"):
                i += 1
                continue
            marks[i] = True
            depth = lines[i].count("{") - lines[i].count("}")
            i += 1
            pending = False
            while i < len(lines) and depth > 0:
                marks[i] = True
                depth += lines[i].count("{") - lines[i].count("}")
                i += 1
            continue
        i += 1
    return marks


def main():
    all_tests = []
    for base, _dirs, names in os.walk(CRATES):
        if "target" in base.split(os.sep):
            continue
        for n in sorted(names):
            if not n.endswith(".rs"):
                continue
            p = os.path.join(base, n)
            rel = os.path.relpath(p, ROOT).replace("\\", "/")
            lines = open(p, "r", encoding="utf-8", errors="replace").read().splitlines()
            marks = test_region_marks(lines)
            for idx, line in enumerate(lines):
                if not marks[idx]:
                    continue
                m = re.match(r"\s*(?:pub\s+)?fn\s+(\w+)\s*\(", line)
                if m:
                    all_tests.append((rel, idx + 1, m.group(1)))
    print("TOTAL test fns:", len(all_tests))
    for rel, ln, name in all_tests:
        print("%-62s %5d  %s" % (rel, ln, name))
    return 0


if __name__ == "__main__":
    main()
