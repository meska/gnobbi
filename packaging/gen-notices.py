#!/usr/bin/env python3
"""Regenerate THIRD_PARTY_NOTICES.md from the current Cargo.lock.

Run from the repo root (needs cargo and python3):
    python3 packaging/gen-notices.py

Uses `cargo metadata --locked --all-features`, which must resolve exactly the
packages in Cargo.lock (the script fails if it doesn't). Workspace members are
skipped (they are our own crates, MIT OR Apache-2.0). Vendored crates carry a
provenance annotation below instead of the plain upstream repository URL.
"""
import json
import subprocess
import sys

ANNOTATIONS = {
    # matrix-sdk is vendored and patched; see GNOBBI-PATCH.md in the vendor dir.
    ("matrix-sdk", "0.19.1"):
        "https://github.com/matrix-org/matrix-rust-sdk — vendored with local "
        "modifications (optional receive-time response body limit); provenance, "
        "modified files and license in vendor/matrix-sdk-0.19.1/GNOBBI-PATCH.md, "
        "LICENSE and vendor/matrix-sdk-0.19.1.patch.",
    # gpui is vendored with one Linux text-system patch; see GNOBBI-PATCH.md in the vendor dir.
    ("gpui", "0.2.2"):
        "https://github.com/zed-industries/zed — vendored with a local Linux "
        "text-system modification (emoji clusters use the bundled Noto Color Emoji); "
        "provenance and modified file in vendor/gpui-0.2.2/GNOBBI-PATCH.md.",
}

HEADER = """# Third-party notices

Gnobbi depends on the crates below (all targets and features, from Cargo.lock). License strings are the SPDX expressions declared by each crate; full texts ship with each crate source. Generated with cargo metadata; not a legal audit.
"""

FOOTER = """
## Bundled font: Noto Color Emoji

- Noto Color Emoji 2.051 (NotoColorEmoji.ttf, CBDT/CBLC colour bitmaps), Copyright 2013 Google LLC, SIL Open Font License 1.1.
- Source: https://github.com/googlefonts/noto-emoji/blob/v2.051/fonts/NotoColorEmoji.ttf (release tag v2.051, file unmodified).
- SHA-256: `72a635cb3d2f3524c51620cdde406b217204e8a6a06c6a096ff8ed4b5fd6e27b`
- File: `crates/matrix-desktop/assets/fonts/NotoColorEmoji.ttf`; license text next to it in `NotoColorEmoji-OFL.txt` (SHA-256 `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2`).
- The font is embedded in the Linux binary at build time (no network at run time, nothing extra in the AppImage); it is used for every emoji cluster, so the set is the same on every Linux system. macOS and Windows use the system emoji font.
- The OFL allows bundling and redistribution with software; the font is not sold by itself and is not renamed or modified.

## AppImage package

The Linux AppImage bundles only the application binary (which embeds the font above), its desktop entry, metadata and icons (own assets). No system libraries are bundled; GPU, Wayland and Vulkan libraries come from the host. The AppImage file embeds the upstream type 2 runtime from the AppImage project (MIT; it statically links squashfuse, BSD-2-Clause, and libfuse, LGPL-2.1-or-later): https://github.com/AppImage/type2-runtime

## App icons

The Gnobbi app icons (the default "minimal mark", the gnome and the monster variants) were generated with an AI image model and then only cropped and resized. They are offered under the project license (MIT OR Apache-2.0). They are not a registered trademark.
"""


def main() -> int:
    meta = subprocess.run(
        ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"],
        capture_output=True, text=True, check=True,
    ).stdout
    doc = json.loads(meta)
    members = set(doc["workspace_members"])

    lines = []
    seen = set()
    for p in sorted(doc["packages"], key=lambda p: (p["name"], p["version"])):
        if p["id"] in members:
            continue
        key = (p["name"], p["version"])
        if key in seen:  # same crate reachable from several sources
            continue
        seen.add(key)
        source = ANNOTATIONS.get(key) or p.get("repository") or p.get("homepage") or ""
        lines.append(f"- {p['name']} {p['version']} — {p.get('license', 'NO LICENSE FIELD')}"
                     + (f" — {source}" if source else ""))
    with open("THIRD_PARTY_NOTICES.md", "w") as f:
        f.write(HEADER + "\n" + "\n".join(lines) + "\n" + FOOTER)
    print(f"THIRD_PARTY_NOTICES.md: {len(lines)} crates")
    return 0


if __name__ == "__main__":
    sys.exit(main())
