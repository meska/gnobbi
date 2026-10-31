#!/usr/bin/env python3
"""Checks that every packaged icon exists with the right size (stdlib only). Run from anywhere."""
import struct, pathlib, sys, zlib
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
# macOS Big Sur template: alpha, transparent corners, ~100/1024 margin. Pure-Python PNG decode, no iconutil.
def alpha_plane(b):
    """Alpha rows of an 8-bit non-interlaced RGBA/GA PNG, or None when there is no alpha."""
    w, h, depth, ct, _, _, il = struct.unpack(">IIBBBBB", b[16:29])
    if ct not in (4, 6) or depth != 8 or il: return None
    bpp = 4 if ct == 6 else 2; raw = bytearray(); i = 8
    while i < len(b):
        n, t = struct.unpack(">I4s", b[i:i + 8])
        if t == b"IDAT": raw += b[i + 8:i + 8 + n]
        i += 12 + n
    raw = zlib.decompress(bytes(raw)); st = w * bpp; prev = bytearray(st); rows = []
    for y in range(h):
        f = raw[y * (st + 1)]; r = bytearray(raw[y * (st + 1) + 1:(y + 1) * (st + 1)])
        for x in range(st):
            a = r[x - bpp] if x >= bpp else 0; c = prev[x - bpp] if x >= bpp else 0
            r[x] = (r[x] + (0, a, prev[x], (a + prev[x]) // 2, None)[f] if f < 4 else
                    r[x] + min((a, prev[x], c), key=lambda v: abs(a + prev[x] - c - v))) & 255
        rows.append(r[bpp - 1::bpp]); prev = r
    return rows
def mac_png(label, b, size):
    if b[:8] != b"\x89PNG\r\n\x1a\n": return bad.append(f"{label}: not a PNG")
    if struct.unpack(">II", b[16:24]) != (size, size): return bad.append(f"{label}: not {size}x{size}")
    a = alpha_plane(b)
    if a is None: return bad.append(f"{label}: no alpha channel (full-bleed icon, expected Big Sur template)")
    if any(a[y][x] for y in (0, size - 1) for x in (0, size - 1)): bad.append(f"{label}: corner pixels not transparent")
    ys = [y for y in range(size) if max(a[y]) > 16]  # >16 skips the faint drop shadow
    xs = [x for x in range(size) if any(a[y][x] > 16 for y in ys)]
    if not xs: return bad.append(f"{label}: fully transparent")
    want = size * 100 / 1024; tol = max(2, size * 0.02)
    box = (xs[0], ys[0], size - 1 - xs[-1], size - 1 - ys[-1])  # left, top, right, bottom margins
    if any(abs(m - want) > tol for m in box):
        bad.append(f"{label}: alpha margins l/t/r/b {box}, expected ~{want:.1f}px (+-{tol:.1f}) at {size}px")
try:
    b = (R / "macos/Gnobbi.icns").read_bytes(); i = 8; seen = set()
    while i + 8 <= len(b):
        t, n = struct.unpack(">4sI", b[i:i + 8]); d = b[i + 8:i + n]; i += n
        if d[:4] == b"\x89PNG": w = struct.unpack(">I", d[16:20])[0]; seen.add(w); mac_png(f"macos/Gnobbi.icns[{t.decode()}]", d, w)
    if len(seen) < 6: bad.append(f"macos/Gnobbi.icns: only PNG sizes {sorted(seen)}")
except OSError: pass  # already reported above
for n in ("segno-minimo", "gnomo", "mostriciattolo"):
    try: mac_png(f"matrix-desktop/assets/icon-{n}-mac-512.png", (R.parent / f"crates/matrix-desktop/assets/icon-{n}-mac-512.png").read_bytes(), 512)
    except OSError: bad.append(f"icon-{n}-mac-512.png: missing")
# Source-derived sets live in assets/ (not versioned): checked only when present.
for n in ("segno-minimo", "gnomo", "mostriciattolo"):
    if (R.parent / "assets/icons" / n).is_dir():
        for s in (16, 32, 64, 128, 256, 512, 1024): png(f"../assets/icons/{n}/icon-{s}.png", s, s)
        for s in (16, 32, 64, 128, 256, 512, 1024):
            if (R.parent / f"assets/icons/{n}/mac-{s}.png").exists(): mac_png(f"assets/icons/{n}/mac-{s}.png", (R.parent / f"assets/icons/{n}/mac-{s}.png").read_bytes(), s)
print("\n".join(bad) or "icons OK"); sys.exit(bool(bad))
