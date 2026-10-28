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
}

HEADER = """# Third-party notices

Gnobbi depends on the crates below (all targets and features, from Cargo.lock). License strings are the SPDX expressions declared by each crate; full texts ship with each crate source. Generated with cargo metadata; not a legal audit.
"""

FOOTER = """
## AppImage package

The Linux AppImage bundles only the application binary, its desktop entry, metadata and icons (own assets). No system libraries are bundled; GPU, Wayland and Vulkan libraries come from the host. The AppImage file embeds the upstream type 2 runtime from the AppImage project (MIT; it statically links squashfuse, BSD-2-Clause, and libfuse, LGPL-2.1-or-later): https://github.com/AppImage/type2-runtime

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
