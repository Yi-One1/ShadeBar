#!/usr/bin/env python3
"""QA independent PE audit for the three ShadeBar artifacts.

No third-party modules: hand-parses DOS/COFF/Optional headers, import table,
and the resource tree (RT_MANIFEST / RT_VERSION / RT_GROUP_ICON / RT_ICON).
"""
import hashlib
import os
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ARTIFACTS = [
    ("formal", os.path.join(ROOT, "dist", "\u968f\u5f71_ShadeBar_v1.0.0.exe")),
    ("verify", os.path.join(ROOT, "dist", "verify", "\u968f\u5f71_ShadeBar_v1.0.0_verify.exe")),
    ("no-en", os.path.join(ROOT, "dist", "compare", "\u968f\u5f71_ShadeBar_v1.0.0_no-en.exe")),
]
RT_ICON, RT_GROUP_ICON, RT_VERSION, RT_MANIFEST = 3, 14, 16, 24


def u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def parse(path):
    d = open(path, "rb").read()
    out = {"path": path, "size": len(d), "sha256": hashlib.sha256(d).hexdigest()}
    lf = u32(d, 0x3C)
    assert d[lf:lf + 4] == b"PE\0\0", "not a PE file"
    coff = lf + 4
    machine = u16(d, coff)
    nsec = u16(d, coff + 2)
    size_opt = u16(d, coff + 16)
    opt = coff + 20
    magic = u16(d, opt)
    out["machine"] = hex(machine)
    out["pe"] = "PE32+" if magic == 0x20B else "PE32"
    dd_off = opt + (112 if magic == 0x20B else 96)
    nrva = u32(d, opt + (108 if magic == 0x20B else 92))
    dd = [(u32(d, dd_off + 8 * i), u32(d, dd_off + 8 * i + 4)) for i in range(nrva)]
    sec_off = opt + size_opt
    sections = []
    for i in range(nsec):
        o = sec_off + 40 * i
        sections.append((
            d[o:o + 8].rstrip(b"\0").decode("ascii", "replace"),
            u32(d, o + 12), u32(d, o + 8), u32(d, o + 20), u32(d, o + 16),
        ))
    out["sections"] = [s[0] for s in sections]

    def rva2off(rva):
        for _name, va, vs, rp, rs in sections:
            if va <= rva < va + max(vs, rs):
                return rp + (rva - va)
        return None

    # --- import table (authoritative DLL dependency list) ---
    imports = []
    if len(dd) > 1 and dd[1][0]:
        off = rva2off(dd[1][0])
        i = 0
        while off is not None and off + 20 * (i + 1) <= len(d):
            vals = struct.unpack_from("<IIIII", d, off + 20 * i)
            if vals == (0, 0, 0, 0, 0):
                break
            nr = rva2off(vals[3])
            if nr is None:
                break
            end = d.find(b"\0", nr)
            imports.append(d[nr:end].decode("ascii", "replace"))
            i += 1
    out["imports"] = imports
    forbidden = [x for x in imports if x.upper().startswith(("VCRUNTIME", "MSVCP"))]
    out["forbidden_crt"] = forbidden

    # --- resource tree ---
    resources = {}
    if len(dd) > 2 and dd[2][0]:
        base = dd[2][0]

        def walk(rva, ids):
            off = rva2off(rva)
            if off is None:
                return
            cnt = u16(d, off + 12) + u16(d, off + 14)
            for k in range(cnt):
                e = off + 16 + 8 * k
                name, otd = u32(d, e), u32(d, e + 4)
                if name & 0x80000000:
                    so = rva2off(base + (name & 0x7FFFFFFF))
                    ln = u16(d, so)
                    label = d[so + 2:so + 2 + 2 * ln].decode("utf-16le", "replace")
                else:
                    label = name
                if otd & 0x80000000:
                    walk(base + (otd & 0x7FFFFFFF), ids + [label])
                else:
                    lo = rva2off(base + otd)
                    if lo is None:
                        continue
                    drva, size = u32(d, lo), u32(d, lo + 4)
                    do = rva2off(drva)
                    if do is None:
                        continue
                    resources.setdefault(tuple(ids), []).append((label, d[do:do + size]))

        walk(base, [])

    def by_type(t):
        blobs = []
        for key, entries in resources.items():
            if key and key[0] == t:
                blobs.extend(entries)
        return blobs

    out["resource_types"] = sorted({str(k[0]) for k in resources})
    # manifest
    mans = by_type(RT_MANIFEST)
    out["manifest_count"] = len(mans)
    mtext = ""
    for _lbl, blob in mans:
        mtext += blob.decode("utf-8", "replace")
    out["manifest_asInvoker"] = ('level="asInvoker"' in mtext)
    out["manifest_requestedExecutionLevel"] = "requestedExecutionLevel" in mtext
    out["manifest_PerMonitorV2"] = "PerMonitorV2" in mtext
    out["manifest_activeCodePage"] = "UTF-8" in mtext
    out["manifest_runas_comment"] = "runas" in mtext
    out["manifest_text"] = mtext
    # version strings (UTF-16LE key/value pairs)
    vers = by_type(RT_VERSION)
    vtext = ""
    for _lbl, blob in vers:
        vtext += blob.decode("utf-16-le", "replace")
    out["version_strings"] = {}
    for key in ("FileVersion", "ProductVersion", "ProductName", "FileDescription",
                "OriginalFilename", "CompanyName", "InternalName"):
        idx = vtext.find(key)
        if idx >= 0:
            tail = vtext[idx + len(key):idx + len(key) + 160].strip("\x00 \r\n\t")
            out["version_strings"][key] = tail.split("\x00")[0][:80]
    out["version_has_VS_VERSION_INFO"] = "VS_VERSION_INFO" in vtext
    # icons
    icons = []
    for _lbl, blob in by_type(RT_GROUP_ICON):
        if len(blob) < 6:
            continue
        count = u16(blob, 4)
        for i in range(count):
            o = 6 + 14 * i
            if o + 14 > len(blob):
                break
            w = blob[o] or 256
            h = blob[o + 1] or 256
            bits = u16(blob, o + 6)
            icons.append((w, h, bits))
    out["icon_entries"] = icons
    out["icon_raw_count"] = len(by_type(RT_ICON))
    # raw string probes
    ascii_txt = d.decode("latin-1")
    out["has_UI_Xaml"] = "UI_Xaml" in ascii_txt
    out["has_ShellExecuteExW"] = "ShellExecuteExW" in ascii_txt
    out["has_runas_string"] = "runas" in ascii_txt.lower()
    return out


def main():
    results = {}
    for label, path in ARTIFACTS:
        if not os.path.isfile(path):
            print("MISSING:", label, path)
            continue
        r = parse(path)
        results[label] = r
        print("=" * 72)
        print("ARTIFACT:", label, r["path"])
        print("  size          :", r["size"], "B =", round(r["size"] / 1024, 1), "KB")
        print("  sha256        :", r["sha256"])
        print("  machine / PE  :", r["machine"], r["pe"])
        print("  sections      :", ", ".join(r["sections"]))
        print("  imports       :", ", ".join(r["imports"]))
        print("  forbidden CRT :", r["forbidden_crt"] or "NONE")
        print("  resource types:", ", ".join(r["resource_types"]))
        print("  manifest      : count=%d asInvoker=%s requestedExecutionLevel=%s PerMonitorV2=%s UTF8=%s runas_comment=%s" % (
            r["manifest_count"], r["manifest_asInvoker"], r["manifest_requestedExecutionLevel"],
            r["manifest_PerMonitorV2"], r["manifest_activeCodePage"], r["manifest_runas_comment"]))
        print("  version       :", r["version_strings"], "VS_VERSION_INFO=", r["version_has_VS_VERSION_INFO"])
        print("  icons         :", "entries=", r["icon_entries"], "RT_ICON blobs=", r["icon_raw_count"])
        print("  strings       : UI_Xaml=%s ShellExecuteExW=%s runas=%s" % (
            r["has_UI_Xaml"], r["has_ShellExecuteExW"], r["has_runas_string"]))
    # cross checks
    print("=" * 72)
    print("CROSS CHECKS")
    if "formal" in results and "no-en" in results:
        delta = results["formal"]["size"] - results["no-en"]["size"]
        print("  A-26 delta formal - no-en =", delta, "B =", round(delta / 1024, 2), "KB (<= 40 KB)")
    dist = os.path.join(ROOT, "dist")
    root_exes = [f for f in os.listdir(dist) if f.lower().endswith(".exe")]
    print("  dist root exe files:", root_exes, "count=", len(root_exes))
    sidecars = [p for p in os.listdir(dist) if os.path.isdir(os.path.join(dist, p)) and p.lower() == "lang"]
    print("  dist root lang sidecars:", sidecars)
    for label, r in results.items():
        ok = (not r["forbidden_crt"] and r["manifest_asInvoker"] and r["manifest_PerMonitorV2"]
              and r["version_strings"].get("FileVersion") == "1.0.0.0"
              and any(w == 32 and h == 32 for w, h, _ in r["icon_entries"])
              and not r["has_UI_Xaml"])
        print("  GATE SUMMARY", label, ":", "PASS" if ok else "FAIL")
    return 0


if __name__ == "__main__":
    sys.exit(main())
