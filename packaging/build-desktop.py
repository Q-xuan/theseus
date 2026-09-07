#!/usr/bin/env python3
"""Build a local theseus-desktop package on the machine that will run it.

macOS / Windows: `cargo tauri build --features gui` after staging the sidecar.
Linux: `--check` (CI) or `--portable` (two-binary smoke folder). No .app / NSIS.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / "crates" / "theseus-desktop"
DIST = ROOT / "dist"

sys.path.insert(0, str(Path(__file__).resolve().parent))
import prepare_sidecar as sidecar  # noqa: E402


def host_os() -> str:
    if sys.platform == "darwin":
        return "macos"
    if sys.platform.startswith("win"):
        return "windows"
    return "linux"


def expected_artifacts(triple: str) -> list[str]:
    if "apple-darwin" in triple:
        return [
            "target/release/bundle/macos/Theseus.app",
            "target/release/bundle/dmg/Theseus_0.5.0_aarch64.dmg  (or x64)",
        ]
    if "windows" in triple:
        return [
            "target/release/bundle/nsis/Theseus_0.5.0_x64-setup.exe  (or arm64)",
            "dist/theseus-portable-<triple>/   (if --portable)",
        ]
    return ["(this host does not emit .app / NSIS)"]


def assert_before_build_finds_sidecar_script(conf_text: str) -> None:
    """Tauri 2.2 string hooks run from frontend_dir, not tauri.conf.json's dir.

    Without a package.json that is crates/ (parent of theseus-desktop). The v0.5.1
    command `python3 ../../packaging/prepare_sidecar.py` therefore resolved to
    the repo *parent*. Require an explicit cwd so the script is found from the
    repo root after tauri-cli set_current_dir(tauri_dir).
    """
    data = json.loads(conf_text)
    before = data.get("build", {}).get("beforeBuildCommand")
    if not isinstance(before, dict):
        raise SystemExit(
            "beforeBuildCommand must be {script, cwd}. A string hook uses "
            "Tauri frontend_dir (crates/ here), so ../../packaging escapes "
            "the workspace — see tag v0.5.1 release-desktop failure."
        )
    script = before.get("script") or ""
    if "prepare_sidecar.py" not in script:
        raise SystemExit("beforeBuildCommand.script must invoke prepare_sidecar.py")
    hook_cwd = Path(before.get("cwd") or ".")
    resolved_cwd = (DESKTOP / hook_cwd).resolve()
    token = next((part for part in script.split() if part.endswith("prepare_sidecar.py")), None)
    if token is None:
        raise SystemExit("beforeBuildCommand.script must pass prepare_sidecar.py")
    script_path = Path(token) if Path(token).is_absolute() else (resolved_cwd / token)
    if not script_path.is_file():
        raise SystemExit(
            f"beforeBuildCommand cannot find {script_path} "
            f"(cwd={hook_cwd} from {DESKTOP} -> {resolved_cwd})"
        )


def run_check() -> None:
    conf = DESKTOP / "tauri.conf.json"
    text = conf.read_text()
    required = [
        '"active": true',
        '"createUpdaterArtifacts": false',
        "binaries/theseus-app-server",
        '"productName": "Theseus"',
        '"identifier": "dev.theseus.desktop"',
        '"signingIdentity": "-"',
        '"certificateThumbprint": null',
    ]
    if "pi.app" in text or "pi_0.5.0" in text or '"productName": "pi"' in text:
        raise SystemExit("tauri.conf.json still uses pi installer branding")
    missing = [item for item in required if item not in text]
    if missing:
        raise SystemExit(f"tauri.conf.json missing {missing}")
    if '"updater"' in text:
        raise SystemExit("tauri.conf.json must not enable the updater plugin")
    assert_before_build_finds_sidecar_script(text)
    workflow = ROOT / ".github" / "workflows" / "release-desktop.yml"
    if not workflow.is_file():
        raise SystemExit("missing .github/workflows/release-desktop.yml")
    wf = workflow.read_text()
    for needle in (
        "macos-latest",
        "windows-latest",
        "packaging/build-desktop.py",
        "workflow_dispatch",
        "v*",
    ):
        if needle not in wf:
            raise SystemExit(f"release-desktop.yml missing {needle!r}")
    if "createUpdaterArtifacts: true" in wf or "tauri-plugin-updater" in wf:
        raise SystemExit("release-desktop.yml must not enable the updater")
    icons = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico"]
    for name in icons:
        path = DESKTOP / "icons" / name
        if not path.is_file() or path.stat().st_size < 32:
            raise SystemExit(f"missing icon {path}")
    print(f"sidecar_stage_name {sidecar.staged_name()}")
    print("packaging check ok")
    print(f"host_triple {sidecar.host_triple()}")
    print("linux CI: cargo test --workspace && python3 packaging/build-desktop.py --check")
    print("macOS/Windows package: python3 packaging/build-desktop.py")
    print("GitHub Release (unsigned): push an existing v* tag, or workflow_dispatch with that tag")
    for line in expected_artifacts(sidecar.host_triple()):
        print(f"  artifact {line}")


def assemble_portable(profile: str) -> Path:
    triple = sidecar.host_triple()
    dest_dir = DIST / f"theseus-portable-{triple}"
    if dest_dir.exists():
        shutil.rmtree(dest_dir)
    dest_dir.mkdir(parents=True)
    suffix = sidecar.exe_suffix(triple)
    staged = sidecar.stage(profile=profile, build=True)
    args = ["cargo", "build", "-p", "theseus-desktop", "-q"]
    if profile == "release":
        args.append("--release")
    # Portable folder on Linux is the preview binary (no gui). On Mac/Win, gui.
    if host_os() != "linux":
        args.extend(["--features", "gui"])
    subprocess.check_call(args, cwd=ROOT)
    desktop_src = ROOT / "target" / profile / f"theseus-desktop{suffix}"
    server_src = sidecar.sidecar_src(profile, triple)
    if not desktop_src.is_file():
        raise SystemExit(f"theseus-desktop missing: {desktop_src}")
    if not server_src.is_file():
        server_src = staged
    shutil.copy2(desktop_src, dest_dir / desktop_src.name)
    shutil.copy2(server_src, dest_dir / f"theseus-app-server{suffix}")
    (dest_dir / "README.txt").write_text(
        "Theseus portable folder\n"
        "Keep both binaries in this directory. Double-click theseus-desktop "
        "(the Tauri bundle uses the name Theseus).\n"
        "THESEUS_LLM_API_KEY is inherited from the user/system environment only.\n"
        "Legacy PI_* names still work as a temporary fallback.\n"
        "No key box. Closing the window must kill theseus-app-server.\n",
        encoding="utf-8",
    )
    print(f"portable {dest_dir}")
    return dest_dir


def run_tauri_build() -> None:
    if host_os() == "linux":
        raise SystemExit(
            "Linux does not produce .app / NSIS. Use --check or --portable, "
            "or run this script on macOS / Windows."
        )
    sidecar.stage(profile="release", build=True)
    tauri = shutil.which("cargo-tauri") or shutil.which("tauri")
    cmd = ["cargo", "tauri", "build", "--features", "gui"]
    print("running", " ".join(cmd), "in", DESKTOP)
    try:
        subprocess.check_call(cmd, cwd=DESKTOP)
    except FileNotFoundError as err:
        raise SystemExit(
            "tauri-cli missing. Install: cargo install tauri-cli --version '^2.2'\n"
            f"or assemble a two-binary folder: python3 packaging/build-desktop.py --portable\n({err})"
        ) from err
    except subprocess.CalledProcessError as err:
        if err.returncode != 0:
            print(
                "tauri build failed. A two-binary portable folder still works:\n"
                "  python3 packaging/build-desktop.py --portable",
                file=sys.stderr,
            )
        raise
    print("bundle outputs under target/release/bundle/")
    bundle = ROOT / "target" / "release" / "bundle"
    if bundle.is_dir():
        for path in sorted(bundle.rglob("*")):
            if path.is_file() and path.suffix.lower() in {".app", ".dmg", ".exe", ".msi"} or path.suffix == ".app":
                print(f"  {path}")
        # .app is a directory
        for path in sorted(bundle.glob("macos/*.app")):
            print(f"  {path}")
        for path in sorted(bundle.glob("dmg/*")):
            print(f"  {path}")
        for path in sorted(bundle.glob("nsis/*")):
            print(f"  {path}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--portable", action="store_true")
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    args = parser.parse_args()
    os.chdir(ROOT)
    if args.check:
        run_check()
        return
    if args.portable:
        assemble_portable(args.profile)
        return
    run_tauri_build()


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as err:
        sys.exit(err.returncode)
