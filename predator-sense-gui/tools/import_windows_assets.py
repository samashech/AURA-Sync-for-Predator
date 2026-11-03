#!/usr/bin/env python3
"""Copy the Windows PredatorSense app's own artwork into a local folder.

For personal use on a machine that also has the official app installed (for
example a dual-boot Windows partition). Nothing Acer ships is stored in this
repository: this script reads the app's `app.asar` straight from that
installation and writes the background, the Predator font and the icons to
`~/.local/share/predator-sense/acer-assets/`, where the GUI picks them up at
runtime and falls back to its own drawn design when they are absent.

Usage:
    python3 -I tools/import_windows_assets.py /path/to/app.asar

The asar is the Electron archive under
`Program Files/WindowsApps/ULICTekInc.PredatorSenseforNotebook_*/app/resources/`.
Files are only read from the archive, never executed. Each SVG is also
rendered to a 2x PNG beside it with `rsvg-convert` (librsvg), so the GUI
only ever has to load PNGs, which GTK decodes natively.
"""

import json
import os
import re
import shutil
import struct
import subprocess
import sys

DEST = os.path.join(os.path.expanduser("~"), ".local/share/predator-sense/acer-assets")
# Everything the UI needs sits under the Angular bundle's output folder.
PREFIX = "dist/widgets/main/"
# Content-hashed build names (`Predator-Bold.261cd725a32fe532.otf`) are saved
# under their plain names so the GUI does not depend on one app build.
HASHED = re.compile(r"^(?P<stem>.+)\.[0-9a-f]{16}(?P<ext>\.[a-zA-Z0-9]+)$")
WANTED_EXT = {".png", ".jpg", ".svg", ".otf", ".ttf", ".webp"}


def read_header(data):
    # Chromium pickle: u32 header-pickle size, then the pickle holding a u32
    # payload size, a u32 string length and the JSON index.
    header_size = struct.unpack_from("<I", data, 4)[0]
    json_len = struct.unpack_from("<I", data, 12)[0]
    return json.loads(data[16 : 16 + json_len]), 8 + header_size


def walk(node, path=""):
    for name, entry in node.get("files", {}).items():
        if name in ("..", ".") or "/" in name or "\\" in name:
            continue
        child = f"{path}{name}"
        if "files" in entry:
            yield from walk(entry, child + "/")
        elif "offset" in entry and not entry.get("unpacked"):
            yield child, entry


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    with open(sys.argv[1], "rb") as f:
        data = f.read()
    header, base = read_header(data)
    count = 0
    svgs = []
    for path, entry in walk(header):
        if not path.startswith(PREFIX):
            continue
        rel = path[len(PREFIX) :]
        if rel.startswith("assets/"):
            rel = rel[len("assets/") :]
        else:
            # Top-level build outputs: keep only the hashed copies of
            # images/fonts the stylesheet references, under plain names.
            m = HASHED.match(rel)
            if "/" in rel or not m:
                continue
            rel = os.path.join("build", m["stem"] + m["ext"])
        if os.path.splitext(rel)[1].lower() not in WANTED_EXT:
            continue
        out = os.path.normpath(os.path.join(DEST, rel))
        if not out.startswith(DEST + os.sep):
            continue
        os.makedirs(os.path.dirname(out), exist_ok=True)
        off = base + int(entry["offset"])
        with open(out, "wb") as f:
            f.write(data[off : off + entry["size"]])
        count += 1
        if out.endswith(".svg"):
            svgs.append(out)
    print(f"copied {count} files to {DEST}")

    rsvg = shutil.which("rsvg-convert")
    if not rsvg:
        print("rsvg-convert not found - SVGs left unrendered (install librsvg)")
        return
    failed = 0
    for svg in svgs:
        result = subprocess.run(
            [rsvg, "--zoom=2", "--output", svg[: -len(".svg")] + ".png", svg],
            stderr=subprocess.DEVNULL,
        )
        failed += result.returncode != 0
    print(f"rendered {len(svgs) - failed} SVGs to PNG ({failed} failed)")


if __name__ == "__main__":
    main()
