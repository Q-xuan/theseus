#!/usr/bin/env python3
"""Generate the checked-in theseus-desktop icon set (PNG / ICO / ICNS). No extra deps."""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates" / "theseus-desktop" / "icons"

BG = (0x1A, 0x1A, 0x18, 255)
FG = (0xFA, 0xFA, 0xF8, 255)


def _blend(dst: list[int], x: int, y: int, w: int, h: int, cover: float) -> None:
    if not (0 <= x < w and 0 <= y < h) or cover <= 0:
        return
    i = (y * w + x) * 4
    a = max(0.0, min(1.0, cover))
    for c in range(3):
        dst[i + c] = int(dst[i + c] * (1 - a) + FG[c] * a)
    dst[i + 3] = 255


def _fill_rect(px: list[int], w: int, h: int, x0: float, y0: float, x1: float, y1: float) -> None:
    ix0, iy0 = int(x0), int(y0)
    ix1, iy1 = int(x1) + 1, int(y1) + 1
    for y in range(max(0, iy0), min(h, iy1)):
        for x in range(max(0, ix0), min(w, ix1)):
            cx0, cy0 = max(x0, x), max(y0, y)
            cx1, cy1 = min(x1, x + 1), min(y1, y + 1)
            if cx1 > cx0 and cy1 > cy0:
                _blend(px, x, y, w, h, (cx1 - cx0) * (cy1 - cy0))


def render(size: int) -> bytes:
    px = []
    for _ in range(size * size):
        px.extend(BG)
    # Geometric T for Theseus, padded like the desktop chrome.
    t = size / 32.0
    _fill_rect(px, size, size, 7 * t, 8.2 * t, 25 * t, 11.6 * t)
    _fill_rect(px, size, size, 14.2 * t, 10.6 * t, 17.8 * t, 23.8 * t)
    return bytes(px)


def png(size: int) -> bytes:
    raw = render(size)
    rows = b"".join(b"\x00" + raw[y * size * 4 : (y + 1) * size * 4] for y in range(size))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def ico(sizes: list[int]) -> bytes:
    images = [(s, png(s)) for s in sizes]
    offset = 6 + 16 * len(images)
    out = struct.pack("<HHH", 0, 1, len(images))
    blobs = b""
    for s, data in images:
        w = 0 if s >= 256 else s
        h = 0 if s >= 256 else s
        out += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return out + blobs


def icns(sizes: dict[bytes, int]) -> bytes:
    body = b""
    for ostype, size in sizes.items():
        data = png(size)
        body += ostype + struct.pack(">I", 8 + len(data)) + data
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "32x32.png").write_bytes(png(32))
    (OUT / "128x128.png").write_bytes(png(128))
    (OUT / "128x128@2x.png").write_bytes(png(256))
    (OUT / "icon.png").write_bytes(png(512))
    (OUT / "icon.ico").write_bytes(ico([16, 32, 48, 256]))
    (OUT / "icon.icns").write_bytes(
        icns(
            {
                b"icp4": 16,
                b"icp5": 32,
                b"icp6": 64,
                b"ic07": 128,
                b"ic08": 256,
                b"ic09": 512,
                b"ic11": 32,
                b"ic12": 64,
                b"ic13": 256,
            }
        )
    )
    print(f"wrote icons in {OUT}")


if __name__ == "__main__":
    main()
