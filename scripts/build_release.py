#!/usr/bin/env python3
"""Build the release binaries without the builder's home-folder paths in them.

Rust embeds source paths in panic messages and debug info, for example
`/Users/<name>/.cargo/registry/src/...`. Those reveal the builder's user name
and folder layout, so every release binary is built with the paths rewritten
to neutral prefixes and then checked. Run from any directory:

    python3 scripts/build_release.py            # UI, both Mac slices and Windows
    python3 scripts/build_release.py --target aarch64-apple-darwin

Windows is built with `cargo build` on Windows and `cargo xwin build`
elsewhere. Package afterwards with scripts/package_release.py, which repeats
the path check and refuses binaries that still contain a home folder.
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc")
# The oldest macOS the release supports; matches LSMinimumSystemVersion.
MACOS_DEPLOYMENT_TARGET = "11.0"


def home_prefixes() -> list[str]:
    """Absolute folders whose paths must not appear in a release binary."""
    folders = {Path.home(), ROOT}
    for variable in ("CARGO_HOME", "RUSTUP_HOME", "CARGO_TARGET_DIR"):
        if value := os.environ.get(variable):
            folders.add(Path(value))
    return sorted({str(folder.resolve()) for folder in folders}, key=len)


def remap_flags() -> list[str]:
    """`--remap-path-prefix` flags, most specific last because the last match wins."""
    home = str(Path.home().resolve())
    cargo_home = str(Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve())
    flags = [f"--remap-path-prefix={home}=~"]
    flags.append(f"--remap-path-prefix={cargo_home}=/cargo")
    flags.append(f"--remap-path-prefix={ROOT}=/openlaser")
    return flags


def configured_flags(target: str) -> list[str]:
    """The target's `rustflags` from .cargo/config.toml.

    Environment rustflags replace, rather than extend, the configured flags, so they are
    carried over explicitly (the Windows static C runtime lives there).
    """
    config = tomllib.loads((ROOT / ".cargo/config.toml").read_text())
    return list(config.get("target", {}).get(target, {}).get("rustflags", []))


def forbidden(payload: bytes) -> list[str]:
    """Home-folder paths found in a binary, as text for the error message."""
    found = []
    for prefix in home_prefixes():
        for encoded in (prefix.encode(), prefix.encode("utf-16-le")):
            if encoded in payload:
                found.append(prefix)
                break
    return found


def check(binary: Path) -> None:
    """Fail when the binary still contains a home-folder path."""
    if leaks := forbidden(binary.read_bytes()):
        raise SystemExit(f"{binary} still contains {', '.join(leaks)}; "
                         "rebuild it with scripts/build_release.py")
    print(f"  no home-folder paths in {binary.relative_to(ROOT) if ROOT in binary.parents else binary}")


def build(target: str, target_dir: Path) -> Path:
    env = dict(os.environ)
    # The encoded form separates flags with 0x1f, so paths may contain spaces.
    env.pop("RUSTFLAGS", None)
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(configured_flags(target) + remap_flags())
    windows = "windows" in target
    if "apple" in target:
        env["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    if windows and platform.system() != "Windows":
        if shutil.which("cargo-xwin") is None:
            raise SystemExit("Install cargo-xwin to cross-build Windows, or pass --target for the Mac slices only")
        command = ["cargo", "xwin", "build"]
    else:
        command = ["cargo", "build"]
    if windows:
        subprocess.run(["node", "scripts/prepare-webview2.mjs", "--force"], cwd=ROOT, check=True)
    command += ["--release", "--locked", "-p", "openlaser", "--features", "desktop", "--target", target]
    print(f"Building {target}…", flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    return target_dir / target / "release" / ("openlaser.exe" if windows else "openlaser")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--target", action="append", choices=TARGETS,
                        help="build only this target (repeatable); default: all three")
    parser.add_argument("--skip-ui", action="store_true", help="reuse the existing ui/dist")
    parser.add_argument("--check-only", type=Path, nargs="+", metavar="BINARY",
                        help="only check existing binaries for home-folder paths")
    args = parser.parse_args()
    if args.check_only:
        for binary in args.check_only:
            check(binary.resolve())
        return
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    if not args.skip_ui:
        subprocess.run(["npm", "run", "build"], cwd=ROOT / "ui", check=True)
    binaries = [build(target, target_dir) for target in (args.target or TARGETS)]
    print("Checking for home-folder paths…", flush=True)
    for binary in binaries:
        check(binary)
    print("Release binaries are ready to package.", flush=True)


if __name__ == "__main__":
    sys.exit(main())
