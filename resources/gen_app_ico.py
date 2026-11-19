#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成随影 ShadeBar 的应用图标（方案 A · 对分）。

权威几何：scratch/icon-concepts.html 的 `#iconA`：
    viewBox 0 0 32 32
    <rect x=1 y=1 w=30 h=30 rx=7 fill=#8A8A8A>        外框
    clipPath: <rect x=3 y=3 w=26 h=26 rx=5>           内部裁剪
    <path d="M3 3 L29 3 L3 29 Z" fill=#1A1A1A>        左上深色三角
    <path d="M29 29 L6 29 L29 6 Z" fill=#FFFFFF>      右下浅色三角
两条斜边的界分别是 x+y=32 与 x+y=35，中间 3 个逻辑单位是灰色中缝；
16 px 下正好留 1 行像素的中缝。外框与内裁剪的圆角中心同为 (8,8)/(24,8)/
(8,24)/(24,24)，所以灰边等宽 2 逻辑单位 = 16 px 下 1 px（取整像素）。

状态点：中心 (7,20)（左下深色半区），直径 6/32 ≈ 边长的 18%（r=3）；
正常画白圆点，降级画白方点。只用 #1A1A1A / #FFFFFF / #8A8A8A 三色。

只用 Python 标准库（zlib / struct）。输出：
    resources/app.ico             16/24/32/48/256 PNG 压缩 ICO（覆盖生成）
    scratch/icon-a-preview.png    五个尺寸并排（浅底 / 深底两行）
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

# ---- 几何（与 crates/shadebar-app/src/tray.rs::icon_pixel 同一套数字） ----
UNIT = 32.0
OUTER_LO, OUTER_HI, OUTER_R = 1.0, 31.0, 7.0
INNER_LO, INNER_HI, INNER_R = 3.0, 29.0, 5.0
DARK_SUM, WHITE_SUM = 32.0, 35.0
DOT_X, DOT_Y, DOT_R2 = 7.0, 20.0, 3.0 * 3.0

DARK = (0x1A, 0x1A, 0x1A, 0xFF)
WHITE = (0xFF, 0xFF, 0xFF, 0xFF)
GRAY = (0x8A, 0x8A, 0x8A, 0xFF)
CLEAR = (0x00, 0x00, 0x00, 0x00)

SIZES = (16, 24, 32, 48, 256)


def in_rounded(u: float, v: float, lo: float, hi: float, r: float) -> bool:
    """点是否落在圆角矩形内；角心用 clamp 求，避免四角被判成尖角。"""
    cx = min(max(u, lo + r), hi - r)
    cy = min(max(v, lo + r), hi - r)
    return (u - cx) ** 2 + (v - cy) ** 2 <= r**2


def pixel(u: float, v: float, show_dot: bool, degraded: bool) -> tuple[int, int, int, int]:
    """逻辑 32 坐标取色（RGBA）。与 Rust 的 icon_pixel 逐像素等价。"""
    if not in_rounded(u, v, OUTER_LO, OUTER_HI, OUTER_R):
        return CLEAR
    if not in_rounded(u, v, INNER_LO, INNER_HI, INNER_R):
        return GRAY
    total = u + v
    if total <= DARK_SUM:
        base = DARK
    elif total >= WHITE_SUM:
        base = WHITE
    else:
        base = GRAY  # 中缝：外框同色，非黑白主体
    if show_dot and base is DARK:
        dx, dy = u - DOT_X, v - DOT_Y
        # 圆点差掉四角，方点保留；16 px 下两者一眼可辨。
        inside = (dx * dx + dy * dy <= DOT_R2) if not degraded else (
            abs(dx) <= 3.0 and abs(dy) <= 3.0
        )
        if inside:
            return WHITE
    return base


def render(size: int, show_dot: bool = True, degraded: bool = False) -> list[tuple[int, int, int, int]]:
    """按目标尺寸在像素中心采样，得到 1:1 的 RGBA 栅格。"""
    scale = UNIT / size
    rows = []
    for y in range(size):
        v = (y + 0.5) * scale
        rows.append([pixel((x + 0.5) * scale, v, show_dot, degraded) for x in range(size)])
    return rows


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
