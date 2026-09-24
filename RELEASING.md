# Preparing an OpenLaser release

The current version is **0.1.0-alpha.4**, published as source and documentation.
No prebuilt binaries are published. The following steps produce local review
packages; they do not push a branch, create a remote tag or publish a release.
Binary publication requires separate maintainer approval.

## Version

Use Semantic Versioning: `0.1.0-alpha.1`, `0.1.0-alpha.2`, then an appropriate
beta/RC or stable version. The Git tag is prefixed with `v`, for example
`v0.1.0-alpha.1`.

Update the workspace version and internal path dependency versions in
`Cargo.toml`, the workspace entries in `Cargo.lock`, `ui/package.json` and both
root version fields in `ui/package-lock.json`. Update the manual, README and
changelog. The UI, CLI and Windows resources take their versions from these
manifests. The packager rejects mismatched versions.

## Verify and build

Use the toolchain and checks described in CONTRIBUTING.md. From the repository
root, with Node, Rust, just and cargo-deny installed:

```sh
just ui-install
just check
```

Build all three release binaries with the release build script. It builds the
UI, both macOS slices and Windows (`cargo build` on Windows, `cargo xwin build`
elsewhere), rewrites home-folder paths such as `/Users/<name>/.cargo/...` to
neutral prefixes with `--remap-path-prefix`, and fails if any home-folder path
is left in a binary:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc
python3 scripts/build_release.py
```

Pass `--target <triple>` (repeatable) to build only some targets, and
`--check-only <binary>...` to check existing binaries. Do not build release
binaries with plain `cargo build`: the packager rejects binaries that contain
the builder's home folder.

MSVC builds statically link the C runtime and WebView2 loader. GNU builds require
an adjacent loader DLL and are not the Windows release package.

## Package

Run on macOS with Python 3.11+ after all three binaries have been built:

```sh
python3 scripts/package_release.py --target-dir target --output ../openlaser-release
```

If `CARGO_TARGET_DIR` points somewhere else, pass that same location explicitly.
The output directory must be new or empty, so an earlier candidate is not
silently overwritten. The script:

- checks versions across Cargo and npm;
- combines the two Mac slices into a universal app and ad-hoc signs it;
- verifies the Windows executable is x64 with the GUI subsystem;
- creates Windows and Mac ZIPs with simulator launchers, manual and notices;
- creates matching source from public Git-tracked/non-ignored files plus the
  built UI, excluding local/private state;
- writes an input manifest and SHA-256 checksums for the packages.

Build and package from the same unchanged checkout. Retain build/check logs and
identify any uncommitted candidate changes in the review record. Source ZIPs are
snapshots, not substitutes for committing/tagging the exact reviewed source.

## Review before publishing

Inspect the packaged application in simulation, check the displayed version,
exercise the affected workflows, and test the Windows package on Windows. State
what was simulated and what was physically commissioned. Keep code-signing and
notarization status explicit; an ad-hoc Mac signature is not notarization.

Review README, USAGE, CHANGELOG, source contents and third-party notices together
with the binaries. Back up real application data before version acceptance.
Post draft documentation to the agreed support channel as a draft, not a release
announcement. Do not attach private test data, controller backups or captures.

After maintainer approval, commit the exact candidate, set the release date in
CHANGELOG, tag the reviewed commit and prepare a GitHub **prerelease**. Attach the
Mac, Windows and matching source ZIPs plus SHA256SUMS. Review the GitHub draft
before publishing. Do not silently rebuild different binaries under an existing
published version; use the next prerelease number.
