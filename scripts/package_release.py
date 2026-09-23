#!/usr/bin/env python3
"""Package the reviewed desktop binaries and corresponding public source.

Run on macOS with Python 3.11+, Cargo, lipo and codesign. Build all three targets
from this checkout first; see RELEASING.md. No remote repository is modified.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import plistlib
import shutil
import struct
import subprocess
import tomllib
import zipfile
from pathlib import Path

from build_release import forbidden


ROOT = Path(__file__).resolve().parents[1]
DOCS = (
    "README.md", "USAGE.md", "CHANGELOG.md", "RELEASING.md",
    "CONTRIBUTING.md", "LICENSE", "THIRD-PARTY-NOTICES.md",
)


def run(*args: str | Path, cwd: Path = ROOT) -> str:
    return subprocess.check_output([str(arg) for arg in args], cwd=cwd, text=True).strip()


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def archive(folder: Path, destination: Path) -> None:
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=6,
                         strict_timestamps=False) as package:
        for path in sorted(folder.rglob("*")):
            if path.is_file():
                package.write(path, Path(folder.name) / path.relative_to(folder))
    with zipfile.ZipFile(destination) as package:
        if (failed := package.testzip()) is not None:
            raise RuntimeError(f"ZIP verification failed: {failed}")


def versions() -> str:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = cargo["workspace"]["package"]["version"]
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    ui = json.loads((ROOT / "ui/package.json").read_text())
    ui_lock = json.loads((ROOT / "ui/package-lock.json").read_text())
    candidates = [ui["version"], ui_lock["version"], ui_lock["packages"][""]["version"]]
    candidates += [entry["version"] for entry in cargo["workspace"]["dependencies"].values()
                   if isinstance(entry, dict) and "path" in entry]
    candidates += [entry["version"] for entry in lock["package"]
                   if entry["name"] == "openlaser" or entry["name"].startswith("openlaser-")]
    if any(candidate != version for candidate in candidates):
        raise RuntimeError("Cargo and npm release versions disagree")
    return version


def public_sources() -> list[Path]:
    names = run("git", "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
    paths = {ROOT / name for name in names if name}
    paths.update((ROOT / "ui/dist").rglob("*"))
    selected = []
    for path in sorted(paths):
        relative = path.relative_to(ROOT)
        if relative.parts[0] in {"reference", "source", "target", "data", ".git", "node_modules"}:
            raise RuntimeError(f"Private/build directory in public source inventory: {relative}")
        if path.is_symlink():
            raise RuntimeError(f"Review source symlink before packaging: {relative}")
        if path.is_file():
            selected.append(path)
    return selected


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    target = args.target_dir.resolve()
    output = args.output.resolve()
    if output == ROOT or ROOT in output.parents:
        parser.error("output must be outside the source checkout")
    if output.exists() and any(output.iterdir()):
        parser.error("output must be new or empty")
    version = versions()
    binaries = {
        "windows-x86_64": target / "x86_64-pc-windows-msvc/release/openlaser.exe",
        "macos-arm64": target / "aarch64-apple-darwin/release/openlaser",
        "macos-x86_64": target / "x86_64-apple-darwin/release/openlaser",
    }
    for binary in binaries.values():
        if not binary.is_file():
            parser.error(f"build first: {binary}")
    if not (ROOT / "ui/dist/index.html").is_file():
        parser.error("build the UI first")

    for name, binary in binaries.items():
        if leaks := forbidden(binary.read_bytes()):
            raise RuntimeError(f"{name} contains home-folder paths ({', '.join(leaks)}); "
                               "build it with scripts/build_release.py")

    pe = binaries["windows-x86_64"].read_bytes()
    offset = struct.unpack_from("<I", pe, 0x3C)[0]
    if (pe[offset:offset + 4] != b"PE\0\0"
            or struct.unpack_from("<H", pe, offset + 4)[0] != 0x8664
            or struct.unpack_from("<H", pe, offset + 24)[0] != 0x20B
            or struct.unpack_from("<H", pe, offset + 24 + 68)[0] != 2):
        raise RuntimeError("Windows binary must be x64 PE32+ with the GUI subsystem")
    for name, architecture in (("macos-arm64", "arm64"), ("macos-x86_64", "x86_64")):
        if run("lipo", "-archs", binaries[name]) != architecture:
            raise RuntimeError(f"Incorrect architecture for {name}")

    assets = [path for path in (ROOT / "ui/dist").rglob("*") if path.is_file()]
    assets.append(ROOT / "LICENSE")
    for name, binary in binaries.items():
        payload = binary.read_bytes()
        for asset in assets:
            if asset.read_bytes() not in payload:
                raise RuntimeError(f"{name} has stale/missing embedded asset: {asset.relative_to(ROOT)}")
    bootstrapper = ROOT / "crates/openlaser/assets/MicrosoftEdgeWebview2Setup.exe"
    if bootstrapper.read_bytes() not in pe:
        raise RuntimeError("Windows WebView2 bootstrapper is missing")

    output.mkdir(parents=True, exist_ok=True)
    paths = public_sources()
    inputs = {str(path.relative_to(ROOT)): digest(path) for path in paths}
    source = output / f"OpenLaser-{version}-source"
    for path in paths:
        copy(path, source / path.relative_to(ROOT))
    print("Vendoring locked Rust dependencies and their notices…", flush=True)
    vendor = source / "vendor"
    vendor_config = run("cargo", "vendor", "--locked", "--offline", "--versioned-dirs", vendor)
    config = source / ".cargo/config.toml"
    config.write_text(config.read_text() + "\n" + vendor_config.replace(str(vendor), "vendor") + "\n")
    # Include the UI runtime's source and MIT notices alongside the editable UI.
    for name in ("svelte", "esm-env", "clsx"):
        module = ROOT / "ui/node_modules" / name
        shutil.copytree(module, source / "vendor-ui" / name)
    subprocess.run(["cargo", "metadata", "--all-features", "--offline", "--locked",
                    "--format-version", "1"], cwd=source, check=True, stdout=subprocess.DEVNULL)

    notices = output / "third-party-licenses"
    for directory in (vendor, source / "vendor-ui"):
        for path in directory.rglob("*"):
            if path.is_file() and path.name.lower().startswith(("license", "copying", "notice")):
                copy(path, notices / directory.name / path.relative_to(directory))
    for path in (ROOT / "ui/static/licenses").iterdir():
        if path.is_file():
            copy(path, notices / "assets" / path.name)

    windows = output / f"OpenLaser-{version}-windows-x86_64"
    macos = output / f"OpenLaser-{version}-macos-universal"
    for folder in (windows, macos):
        for name in DOCS:
            copy(ROOT / name, folder / name)
        shutil.copytree(notices, folder / "third-party-licenses")
        shutil.copytree(ROOT / "assets/readme", folder / "assets/readme")
        for name in ("logo.svg", "logo-white.svg"):
            copy(ROOT / "ui/static" / name, folder / "ui/static" / name)
        # Preserve the relative destinations of the public provenance links.
        for name in ("crates/openlaser/assets/README.md", "crates/openlaser-svg/fonts/SOURCES.md",
                     "fixtures/fonts/SOURCES.md", "fixtures/fonts/OFL.txt"):
            copy(ROOT / name, folder / name)
        shutil.copytree(ROOT / "ui/static/licenses", folder / "ui/static/licenses")
    copy(binaries["windows-x86_64"], windows / "OpenLaser.exe")
    (windows / "Start simulator.cmd").write_bytes(
        b'@echo off\r\nstart "OpenLaser Simulator" "%~dp0OpenLaser.exe" --simulate '
        b'--config "%~dp0simulator\\machine.toml" '
        b'--data "%LOCALAPPDATA%\\OpenLaser-Simulator"\r\n')

    app = macos / "OpenLaser.app"
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    executable = contents / "MacOS/OpenLaser"
    run("lipo", "-create", binaries["macos-arm64"], binaries["macos-x86_64"], "-output", executable)
    executable.chmod(0o755)
    copy(ROOT / "crates/openlaser/assets/OpenLaser.icns", contents / "Resources/OpenLaser.icns")
    for name in ("LICENSE", "USAGE.md", "THIRD-PARTY-NOTICES.md"):
        copy(ROOT / name, contents / "Resources" / name)
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({
            "CFBundleDevelopmentRegion": "en", "CFBundleName": "OpenLaser",
            "CFBundleDisplayName": "OpenLaser", "CFBundleExecutable": "OpenLaser",
            "CFBundleIdentifier": "org.openlaser.desktop", "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version.split("-")[0],
            "CFBundleVersion": version.rsplit(".", 1)[-1] if "-" in version else "1",
            "OpenLaserVersion": version, "LSMinimumSystemVersion": "11.0",
            "NSHighResolutionCapable": True, "CFBundleIconFile": "OpenLaser.icns",
            "NSAppTransportSecurity": {"NSAllowsLocalNetworking": True},
            "NSLocalNetworkUsageDescription":
                "OpenLaser connects to your laser controller and shares its interface with devices on your local network.",
            "NSBonjourServices": ["_http._tcp"],
        }, stream)
    for simulation in (windows / "simulator", contents / "Resources/simulator"):
        copy(ROOT / "fixtures/xml/harness-backup.xml", simulation / "backup.xml")
        (simulation / "machine.toml").write_text(
            '# Simulation fixture only. Never use this as a real machine backup.\n'
            'backup = "backup.xml"\nmode = "fiber"\nco2_manual_focus = false\n'
            'listen = "127.0.0.1:8080"\n')
    run("codesign", "--force", "--sign", "-", app)
    run("codesign", "--verify", "--strict", app)
    architectures = run("lipo", "-archs", executable).split()
    if set(architectures) != {"arm64", "x86_64"}:
        raise RuntimeError("Universal Mac binary is incomplete")
    if run(executable, "--version") != f"OpenLaser {version}":
        raise RuntimeError("Packaged Mac version does not match the source")
    launcher = macos / "Start simulator.command"
    launcher.write_text('#!/bin/sh\nset -eu\ncd -- "$(dirname -- "$0")"\n'
                        'exec "./OpenLaser.app/Contents/MacOS/OpenLaser" --simulate '
                        '--config "./OpenLaser.app/Contents/Resources/simulator/machine.toml" '
                        '--data "$HOME/Library/Application Support/OpenLaser-Simulator"\n')
    launcher.chmod(0o755)

    if public_sources() != paths or any(digest(path) != inputs[str(path.relative_to(ROOT))] for path in paths):
        raise RuntimeError("Source changed during packaging; rebuild and package a new candidate")
    manifest = {
        "version": version,
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "git_head": run("git", "rev-parse", "HEAD"),
        "includes_uncommitted_changes": bool(run("git", "status", "--porcelain")),
        "binary_sha256": {name: digest(path) for name, path in binaries.items()},
        "macos_packaged_binary_sha256": digest(executable),
        "macos_architectures": architectures,
        "macos_signature": "ad-hoc; not notarized",
        "windows_signature": "not publisher-signed",
        "embedded_assets_verified_per_binary": len(assets),
        "toolchain": {name: run(name, "--version") for name in ("rustc", "cargo", "node")},
        "source_inputs_sha256": inputs,
    }
    write_json(source / "RELEASE-INPUTS.json", manifest)
    write_json(output / "RELEASE-INPUTS.json", manifest)
    for folder in (windows, macos, source):
        print(f"Archiving {folder.name}…", flush=True)
        archive(folder, output / f"{folder.name}.zip")
    checksums = [(path.name, digest(path)) for path in sorted(output.glob("*.zip"))]
    checksums.append(("RELEASE-INPUTS.json", digest(output / "RELEASE-INPUTS.json")))
    (output / "SHA256SUMS").write_text("".join(f"{sha}  {name}\n" for name, sha in checksums))
    print(f"Prepared {version} in {output}", flush=True)


if __name__ == "__main__":
    main()
