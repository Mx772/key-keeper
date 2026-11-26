#!/usr/bin/env python3
"""Write the updater manifest (latest.json) for a release.

The app reads it from https://github.com/<repo>/releases/latest/download/latest.json and
cargo-packager-updater expects this "static" shape:

    {"version", "notes", "pub_date", "platforms": {"<os>-<arch>": {"url", "signature", "format"}}}

with <os> one of windows / macos (not darwin) and <arch> x86_64 / aarch64. Windows updates install
the NSIS setup.exe ("nsis"); macOS updates replace the bundle from KeyKeeper.app.tar.gz ("app").
Each signature is the contents of the minisign .sig file cargo-packager wrote next to the artifact.

Fails (exit 1) unless every platform below has its artifact and a non-empty signature, so an
incomplete or unsigned manifest is never published.

Usage: make_latest_json.py --dir upload --tag v0.2.0 --repo owner/name --notes notes.md --out latest.json
"""

import argparse
import datetime
import json
import pathlib
import sys

# manifest key -> (file name pattern, format); {v} is the version without the leading "v".
PLATFORMS = {
    "windows-x86_64": ("KeyKeeper_{v}_x64-setup.exe", "nsis"),
    "macos-aarch64": ("KeyKeeper_{v}_aarch64.app.tar.gz", "app"),
    "macos-x86_64": ("KeyKeeper_{v}_x64.app.tar.gz", "app"),
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True, type=pathlib.Path)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--notes", type=pathlib.Path)
    ap.add_argument("--out", required=True, type=pathlib.Path)
    a = ap.parse_args()

    if not a.tag.startswith("v"):
        print(f"::error::tag {a.tag} does not start with v", file=sys.stderr)
        return 1
    version = a.tag[1:]
    notes = a.notes.read_text(encoding="utf-8").strip() if a.notes and a.notes.exists() else ""

    platforms = {}
    errors = []
    for key, (pattern, fmt) in PLATFORMS.items():
        name = pattern.format(v=version)
        artifact = a.dir / name
        sig = a.dir / (name + ".sig")
        if not artifact.is_file():
            errors.append(f"{key}: missing {name}")
            continue
        if not sig.is_file() or not sig.read_text(encoding="utf-8").strip():
            errors.append(f"{key}: missing or empty signature {name}.sig")
            continue
        platforms[key] = {
            "url": f"https://github.com/{a.repo}/releases/download/{a.tag}/{name}",
            "signature": sig.read_text(encoding="utf-8").strip(),
            "format": fmt,
        }
    if errors:
        for e in errors:
            print(f"::error::{e}", file=sys.stderr)
        return 1

    manifest = {
        "version": version,
        "notes": notes,
        "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "platforms": platforms,
    }
    a.out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {a.out} for {version}: {', '.join(platforms)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
