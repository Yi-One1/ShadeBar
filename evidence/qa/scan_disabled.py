#!/usr/bin/env python3
"""QA disabled-construct scan.

Strips every `#[cfg(test)]` / `#[cfg(any(test, ...))]` / `#[test]` region before
matching, then reports TODO/FIXME/panic!/unwrap()/expect(/unimplemented!/
todo!/unreachable!/UI_Xaml. Comment-only hits are reported separately.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CRATES = os.path.join(ROOT, "crates")

PATTERNS = {
    "TODO": r"\bTODO\b",
    "FIXME": r"\bFIXME\b",
    "panic!": r"panic!\s*\(",
    "unwrap()": r"\.unwrap\s*\(\s*\)",
    "expect(": r"\.expect\s*\(",
    "unimplemented!": r"unimplemented!\s*\(",
    "todo!": r"(?<![\w])todo!\s*\(",
    "unreachable!": r"unreachable!\s*\(",
    "UI_Xaml": r"UI_Xaml",
}
COMPILED = {k: re.compile(v) for k, v in PATTERNS.items()}


def test_region_marks(lines):
    """Mark lines belonging to cfg(test) modules / #[test] functions."""
    marks = [False] * len(lines)
    i = 0
    pending_cfg = False
    pending_test_attr = False
    while i < len(lines):
        s = lines[i].strip()
        if s.startswith("#[") and "cfg" in s and re.search(r"\btest\b", s):
            pending_cfg = True
            i += 1
            continue
        if s.startswith("#[test]"):
            pending_test_attr = True
            i += 1
            continue
        if pending_cfg or pending_test_attr:
            # skip further attributes / blank / comment lines before declaration
            if s.startswith("#[") or s == "" or s.startswith("//"):
                i += 1
                continue
            marks[i] = True
            depth = lines[i].count("{") - lines[i].count("}")
            i += 1
            pending_cfg = False
            pending_test_attr = False
            while i < len(lines) and depth > 0:
                marks[i] = True
                depth += lines[i].count("{") - lines[i].count("}")
                i += 1
            continue
        i += 1
    return marks


def is_comment(s):
    t = s.strip()
    return t.startswith("//") or t.startswith("*") or t.startswith("/*")


def main():
    files = []
    for base, _dirs, names in os.walk(CRATES):
        if "target" in base.split(os.sep):
            continue
        for n in names:
            if n.endswith(".rs"):
                files.append(os.path.join(base, n))
    files.sort()

    totals = {k: {"all": 0, "test": 0, "comment": 0, "runtime": []} for k in PATTERNS}
    per_file = {}

    for path in files:
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
        marks = test_region_marks(lines)
        per_file[rel] = {"lines": len(lines), "test_lines": sum(marks)}
        for idx, line in enumerate(lines):
            for name, rx in COMPILED.items():
                if not rx.search(line):
                    continue
                totals[name]["all"] += 1
                if marks[idx]:
                    totals[name]["test"] += 1
                elif is_comment(line):
                    totals[name]["comment"] += 1
                else:
                    totals[name]["runtime"].append("%s:%d: %s" % (rel, idx + 1, line.strip()[:150]))

    print("Scanned .rs files:", len(files))
    print("Total source lines:", sum(v["lines"] for v in per_file.values()))
    print("Lines inside #[cfg(test)] regions:", sum(v["test_lines"] for v in per_file.values()))
    print("")
    print("%-16s %8s %8s %10s %10s" % ("PATTERN", "ALL", "TEST", "COMMENT", "RUNTIME"))
    for name in PATTERNS:
        t = totals[name]
        print("%-16s %8d %8d %10d %10d" % (name, t["all"], t["test"], t["comment"], len(t["runtime"])))
    print("")
    print("=== RUNTIME (non-test, non-comment) HITS ===")
    any_runtime = False
    for name in PATTERNS:
        for h in totals[name]["runtime"]:
            print("[%s] %s" % (name, h))
            any_runtime = True
    if not any_runtime:
        print("NONE")
    print("")
    print("=== Files whose entire content is test-only (for context) ===")
    for rel, v in per_file.items():
        if v["test_lines"] > 0:
            print("  %-70s lines=%4d test_lines=%4d" % (rel, v["lines"], v["test_lines"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
