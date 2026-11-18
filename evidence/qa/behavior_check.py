#!/usr/bin/env python3
"""QA behavior checks: safe CLI-only invocations (never starts the tray/message loop).

Formal build must reject --verify with EXIT_USAGE(2); verify build must accept
--version normally. No invocation here opens a window or starts sampling.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FORMAL = os.path.join(ROOT, "dist", "\u968f\u5f71_ShadeBar_v1.0.0.exe")
VERIFY = os.path.join(ROOT, "dist", "verify", "\u968f\u5f71_ShadeBar_v1.0.0_verify.exe")
NOEN = os.path.join(ROOT, "dist", "compare", "\u968f\u5f71_ShadeBar_v1.0.0_no-en.exe")

CASES = [
    ("formal --version", FORMAL, ["--version"], 0),
    ("formal -V", FORMAL, ["-V"], 0),
    ("formal --help", FORMAL, ["--help"], 0),
    ("formal -h", FORMAL, ["-h"], 0),
    ("formal /?", FORMAL, ["/?"], 0),
    ("formal --verify fast (branch NOT compiled)", FORMAL, ["--verify", "fast"], 2),
    ("formal --bogus (unknown arg)", FORMAL, ["--bogus"], 2),
    ("verify --version", VERIFY, ["--version"], 0),
    ("verify --help", VERIFY, ["--help"], 0),
    ("verify --verify <bad profile>", VERIFY, ["--verify", "not-a-profile"], 2),
    ("verify --verify <no profile>", VERIFY, ["--verify"], 2),
    ("no-en --version", NOEN, ["--version"], 0),
]


def main():
    failures = 0
    for label, exe, args, want in CASES:
        if not os.path.isfile(exe):
            print("MISSING EXE:", label, exe)
            failures += 1
            continue
        try:
            r = subprocess.run([exe] + args, capture_output=True, timeout=20)
            out = (r.stdout + r.stderr).decode("utf-8", "replace").strip().replace("\r", "")
            got = r.returncode
            ok = got == want
            print("[%s] %s" % ("PASS" if ok else "FAIL", label))
            print("    exit=%d (want %d)  stdout=%r" % (got, want, out[:220]))
            if not ok:
                failures += 1
        except subprocess.TimeoutExpired:
            print("[FAIL] %s : TIMEOUT (process did not exit - would indicate a resident loop)" % label)
            failures += 1
    print("")
    print("BEHAVIOR RESULT: %d/%d passed" % (len(CASES) - failures, len(CASES)))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
