#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成随影 ShadeBar 的应用图标（方案 A · 对分）。

权威几何：scratch/icon-concepts.html 的 `#iconA` 对角黑白方砖。
真机返工（G4 验收）：原 #8A8A8A 细边在白底/深底上都会"融进去"，改为
**深灰 #595959 加粗轮廓**；状态点按用户决定彻底去掉。三色结构不变：
#1A1A1A / #FFFFFF / #595959。

轮廓按目标尺寸取整像素（不在小尺寸上硬等比例缩细）：
    外框  16/24/32 → 1 px，48 → 2 px，256 → 8 px
    中缝  16/24/32 → 2 px，48 → 3 px，256 → 12 px
圆角恒为边长 22%（四舍五入取整像素）。外框与内裁剪共用圆角中心，
所以灰边等宽。中缝按 45° 的垂直宽度计：x+y 方向的间隔 = seam_px×√2。

只用 Python 标准库（zlib / struct）。输出：
    resources/app.ico             16/24/32/48/256 PNG 压缩 ICO（覆盖生成）
    scratch/icon-a-preview.png    五个尺寸并排（浅底 / 深底两行）
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

# ---- 几何（与 crates/shadebar-app/src/tray.rs::icon_pixel 同一套数字） ----
FRAME_PX = {16: 1, 24: 1, 32: 1, 48: 2, 256: 8}
SEAM_PX = {16: 2, 24: 2, 32: 2, 48: 3, 256: 12}

DARK = (0x1A, 0x1A, 0x1A, 0xFF)
WHITE = (0xFF, 0xFF, 0xFF, 0xFF)
GRAY = (0x59, 0x59, 0x59, 0xFF)  # 深灰轮廓 + 中缝，same color
CLEAR = (0x00, 0x00, 0x00, 0x00)

SIZES = tuple(FRAME_PX)


def in_rounded(u: float, v: float, lo: float, hi: float, r: float) -> bool:
    """点是否落在圆角矩形内；角心用 clamp 求，避免四角被判成尖角。"""
    cx = min(max(u, lo + r), hi - r)
    cy = min(max(v, lo + r), hi - r)
    return (u - cx) ** 2 + (v - cy) ** 2 <= r**2


def pixel(x: int, y: int, size: int) -> tuple[int, int, int, int]:
    """目标尺寸的像素中心取色（RGBA）。与 Rust 的 icon_pixel 逐像素等价。"""
    frame = FRAME_PX[size]
    radius = round(size * 0.22)  # 圆角 = 边长 22%
    u, v = x + 0.5, y + 0.5
    if not in_rounded(u, v, 0.0, float(size), float(radius)):
        return CLEAR
    if not in_rounded(u, v, frame, float(size - frame), float(radius - frame)):
        return GRAY
    center = float(size)
    half = SEAM_PX[size] * (2**0.5) / 2.0
    total = u + v
    if total <= center - half:
        return DARK
    if total >= center + half:
        return WHITE
    return GRAY  # 中缝：与外框同色


def render(size: int) -> list[list[tuple[int, int, int, int]]]:
    """按目标尺寸在像素中心采样，得到 1:1 的 RGBA 栅格。"""
    return [[pixel(x, y, size) for x in range(size)] for y in range(size)]


# ---- 纯标准库 PNG 编码 ----------------------------------------------------

def _chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def png_bytes(rows: list[list[tuple[int, int, int, int]]]) -> bytes:
    """8-bit RGBA PNG；每行 filter=0，不用外部图像库。"""
    height = len(rows)
    width = len(rows[0])
    raw = bytearray()
    for row in rows:
        raw.append(0)
        for r, g, b, a in row:
            raw.extend((r, g, b, a))
    return (
        b"\x89PNG\r\n\x1a\n"
        + _chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + _chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + _chunk(b"IEND", b"")
    )


def write_ico(path: Path) -> None:
    images = [(n, png_bytes(render(n))) for n in SIZES]
    out = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + 16 * len(images)
    for n, blob in images:
        # ICO 目录里 256 用 0 表示，否则写不进 1 字节宽度字段。
        out.extend(
            struct.pack(
                "<BBBBHHII",
                0 if n == 256 else n,
                0 if n == 256 else n,
                0,
                0,
                1,
                32,
                len(blob),
                offset,
            )
        )
        offset += len(blob)
    for _, blob in images:
        out.extend(blob)
    path.write_bytes(bytes(out))


def write_preview(path: Path) -> None:
    """五个尺寸并排，浅底 / 深底各一行；原生 1:1，不放大。"""
    cell_w, cell_h = 300, 320
    width, height = cell_w * len(SIZES), cell_h * 2
    canvas = [[(0xF3, 0xF3, 0xF3, 0xFF)] * width for _ in range(cell_h)]
    canvas += [[(0x20, 0x20, 0x20, 0xFF)] * width for _ in range(cell_h)]
    for col, size in enumerate(SIZES):
        icon = render(size)
        ox = cell_w * col + (cell_w - size) // 2
        oy = (cell_h - size) // 2
        for bg_y in (0, cell_h):
            for y in range(size):
                for x in range(size):
                    r, g, b, a = icon[y][x]
                    if a == 0:
                        continue
                    canvas[bg_y + oy + y][ox + x] = (r, g, b, 0xFF)
    path.write_bytes(png_bytes(canvas))


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    ico = root / "resources" / "app.ico"
    preview = root / "scratch" / "icon-a-preview.png"
    write_ico(ico)
    write_preview(preview)
    print(f"wrote {ico} ({ico.stat().st_size} bytes)")
    print(f"wrote {preview} ({preview.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
