#!/usr/bin/env python3
"""Stage theseus-app-server as a Tauri externalBin (host triple suffix)."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN_DIR = ROOT / "crates" / "theseus-desktop" / "binaries"
SIDECAR_STEM = "theseus-app-server"


def host_triple() -> str:
    env = (
        os.environ.get("THESEUS_TARGET_TRIPLE")
        or os.environ.get("PI_TARGET_TRIPLE")
        or os.environ.get("CARGO_BUILD_TARGET")
    )
    if env:
        return env.strip()
    rustc = shutil.which("rustc")
    if not rustc:
        raise SystemExit("rustc not found")
    out = subprocess.check_output([rustc, "-vV"], text=True)
    for line in out.splitlines():
        if line.startswith("host:"):
            return line.split(":", 1)[1].strip()
    raise SystemExit("could not read rustc host triple")


def exe_suffix(triple: str) -> str:
    return ".exe" if "windows" in triple else ""


def staged_name(triple: str | None = None) -> str:
    triple = triple or host_triple()
    return f"{SIDECAR_STEM}-{triple}{exe_suffix(triple)}"


def sidecar_src(profile: str, triple: str) -> Path:
    suffix = exe_suffix(triple)
    target_root = ROOT / "target"
    crossed = target_root / triple / profile / f"{SIDECAR_STEM}{suffix}"
    native = target_root / profile / f"{SIDECAR_STEM}{suffix}"
    if crossed.is_file():
        return crossed
    return native


def build_sidecar(profile: str) -> None:
    args = ["cargo", "build", "-p", "theseus-app-server", "-q"]
    if profile == "release":
        args.append("--release")
    subprocess.check_call(args, cwd=ROOT)


def stage(profile: str = "release", build: bool = True) -> Path:
    triple = host_triple()
    if build:
        build_sidecar(profile)
    src = sidecar_src(profile, triple)
    if not src.is_file():
        raise SystemExit(f"sidecar missing: {src} (build theseus-app-server first)")
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    dest = BIN_DIR / staged_name(triple)
    shutil.copy2(src, dest)
    dest.chmod(dest.stat().st_mode | 0o111)
    return dest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--check", action="store_true", help="print paths; do not copy")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    triple = host_triple()
    dest = BIN_DIR / staged_name(triple)
    src = sidecar_src(args.profile, triple)
    print(f"host_triple {triple}")
    print(f"sidecar_src {src}")
    print(f"staged {dest}")
    if args.check:
        return
    path = stage(profile=args.profile, build=not args.no_build)
    print(f"copied {path} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as err:
        sys.exit(err.returncode)
