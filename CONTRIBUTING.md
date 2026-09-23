# Contributing

OpenLaser controls a laser. The rules below keep it safe to change.

## Before you push

`just check` must pass. It runs exactly what CI runs: formatting, clippy with
warnings denied, tests, docs with warnings denied, the repository gates and
`cargo deny`, generated TypeScript binding checks, and the UI build, type checks
and tests. Use the Node version in `.node-version` or a newer supported version.

## How changes are shaped

- One cohesive change per pull request. Update affected callers and bindings
  together when a change crosses crate boundaries.
- Conventional Commits: `feat(protocol): ...`, `fix(prep): ...`, `docs: ...`,
  `chore: ...`.
- Every `pub` item has a doc comment that says what it is for, with an
  example where one helps.
- `pub(crate)` by default. Promote to `pub` deliberately and re-export the few
  types users need from `lib.rs`.
- Newtypes for units, enums for state. No stringly-typed value crosses a crate
  boundary.
- `thiserror` in libraries, `anyhow` only in the binary. No `unwrap` or
  `expect` outside tests.
- A lint you disagree with gets an `#[allow]` with a one-line reason beside
  it, never a blanket allow.
- Tests live next to the code they test. Integration tests in `tests/` use
  the crate's public API. Add regressions for observable failures and preserve
  byte-level fixtures for protocol and compiler changes. Exercise touch UI
  changes in the running application, including layout and cancellation.
- Interface controls follow the touch rules in [ui/DESIGN.md](ui/DESIGN.md):
  anything that moves the machine, fires the beam or sets a reference is
  held, Stop is always one tap, and destructive actions confirm.

## The machine-exact crates

`openlaser-protocol` and `openlaser-compiler` reproduce the vendor software's
behaviour. A change there names the evidence it rests on in the doc comment,
and relevant encoding and compilation regression tests must pass.

## Definition of done

A change is ready for review when its behavior is documented, affected tests
and fixtures pass, and `just check` is green. State any coverage limits or
unverified physical behavior.

## Running without a machine

`cargo run -p openlaser -- --simulate --no-browser` starts the built-in UDP
loopback simulator and HTTP server. The UI is at `http://127.0.0.1:8080`.
Use a separate `--data` directory for acceptance workloads. Contributors can
build and run fixture tests without machine hardware.
