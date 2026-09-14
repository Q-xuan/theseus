#!/usr/bin/env python3
"""v0.8: product sidecar is PATH `pi --mode rpc`, not an embedded binary.

Kept so older docs / `build-desktop.py --check` still find this file.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN_DIR = ROOT / "crates" / "theseus-desktop" / "binaries"


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
    return f"pi-on-path-{triple}{exe_suffix(triple)}"


def sidecar_src(profile: str, triple: str) -> Path:
    del profile, triple
    return BIN_DIR / "README.md"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    del args
    print(f"host_triple {host_triple()}")
    print("sidecar PATH pi --mode rpc (not embedded theseus-app-server)")
    print("https://github.com/badlogic/pi-mono")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as err:
        sys.exit(err.returncode)
