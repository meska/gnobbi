#!/usr/bin/env python3
"""Checks that every packaged icon exists with the right size (stdlib only). Run from anywhere."""
import struct, pathlib, sys
R = pathlib.Path(__file__).resolve().parent
bad = []
def png(p, w, h, opaque=False):
    try: b = (R / p).read_bytes()
    except OSError: return bad.append(f"{p}: missing")
    if b[:8] != b"\x89PNG\r\n\x1a\n": return bad.append(f"{p}: not a PNG")
    got = struct.unpack(">II", b[16:24]); ct = b[25]
    if got != (w, h): bad.append(f"{p}: {got} != {(w, h)}")
    if opaque and ct in (4, 6): bad.append(f"{p}: has alpha channel")
for s in (16, 32, 64, 128, 256, 512): png(f"linux/icon-{s}.png", s, s)
png("store/apple-1024.png", 1024, 1024, opaque=True); png("store/play-512.png", 512, 512, opaque=True)
try:
    b = (R / "windows/gnobbi.ico").read_bytes(); n = struct.unpack("<H", b[4:6])[0]
    sizes = sorted((b[6 + 16 * i] or 256) for i in range(n))
    if b[:4] != b"\0\0\1\0" or sizes != [16, 32, 64, 128, 256]: bad.append(f"windows/gnobbi.ico: sizes {sizes}")
except OSError: bad.append("windows/gnobbi.ico: missing")
try:
    b = (R / "macos/Gnobbi.icns").read_bytes()
    if b[:4] != b"icns" or struct.unpack(">I", b[4:8])[0] != len(b): bad.append("macos/Gnobbi.icns: bad header")
    for t in (b"ic07", b"ic08", b"ic09", b"ic10", b"ic11", b"ic12", b"ic13", b"ic14"):
        if t not in b: bad.append(f"macos/Gnobbi.icns: missing {t.decode()}")
except OSError: bad.append("macos/Gnobbi.icns: missing")
# Source-derived sets live in assets/ (not versioned): checked only when present.
for n in ("segno-minimo", "gnomo", "mostriciattolo"):
    if (R.parent / "assets/icons" / n).is_dir():
        for s in (16, 32, 64, 128, 256, 512, 1024): png(f"../assets/icons/{n}/icon-{s}.png", s, s)
print("\n".join(bad) or "icons OK"); sys.exit(bool(bad))
