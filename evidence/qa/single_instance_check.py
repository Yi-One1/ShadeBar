#!/usr/bin/env python3
"""QA controlled single-instance (A-01) real-machine check.

Launches the FORMAL exe twice with no args. The 2nd must exit <=1 s with code 0
after posting WM_SHADEBAR_ACTIVATE to the 1st. Then gracefully closes the 1st
via WM_COMMAND / MENU_EXIT (its own tray-menu exit path) and verifies no
ShadeBar process remains. Never touches explorer / TranslucentTB / user windows.
"""
import ctypes
import os
import subprocess
import sys
import time
from ctypes import wintypes

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FORMAL = os.path.join(ROOT, "dist", "\u968f\u5f71_ShadeBar_v1.0.0.exe")
WM_COMMAND, MENU_EXIT = 0x0111, 0x5105
WND_CLASS = "ShadeBar.MainWindow"

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.FindWindowW.restype = wintypes.HWND
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.PostMessageW.restype = wintypes.BOOL


def shadebar_pids():
    r = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True)
    txt = r.stdout.decode("gbk", "replace")
    out = []
    for line in txt.splitlines():
        if "shadebar" in line.lower():
            parts = line.replace('"', "").split(",")
            if len(parts) > 1:
                try:
                    out.append(int(parts[1]))
                except ValueError:
                    pass
    return out


def main():
    if not os.path.isfile(FORMAL):
        print("ABORT: formal exe missing")
        return 2
    before = shadebar_pids()
    if before:
        print("ABORT: pre-existing ShadeBar processes:", before)
        return 2
    print("pre-check: no ShadeBar process")

    p1 = subprocess.Popen([FORMAL], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print("launched 1st instance pid=%d" % p1.pid)
    time.sleep(2.0)
    alive1 = p1.poll() is None
    after1 = shadebar_pids()
    print("1st alive=%s  shadebar_pids=%s" % (alive1, after1))

    t0 = time.time()
    timed_out = False
    rc2 = None
    try:
        r2 = subprocess.run([FORMAL], capture_output=True, timeout=8)
        rc2 = r2.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
    elapsed2 = time.time() - t0
    after2 = shadebar_pids()
    print("2nd: exit=%s elapsed=%.3fs timed_out=%s  shadebar_pids=%s" % (rc2, elapsed2, timed_out, after2))

    hwnd = user32.FindWindowW(WND_CLASS, None)
    print("FindWindowW(%s) hwnd=%s" % (WND_CLASS, hwnd))
    posted = False
    if hwnd:
        posted = bool(user32.PostMessageW(hwnd, WM_COMMAND, MENU_EXIT, 0))
    print("posted MENU_EXIT=%s" % posted)

    for _ in range(60):
        if p1.poll() is not None:
            break
        time.sleep(0.1)
    print("1st exit code=%s (None means still alive)" % p1.poll())

    # hard cleanup of anything we started, if graceful exit failed
    remaining = shadebar_pids()
    for pid in remaining:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
    time.sleep(0.3)
    final = shadebar_pids()
    print("remaining ShadeBar pids after cleanup:", final)

    ok = (alive1 and rc2 == 0 and not timed_out and elapsed2 <= 1.0
          and len(after2) == 1 and p1.poll() is not None and not final)
    print("")
    print("A-01 SINGLE INSTANCE: %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
