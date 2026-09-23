# Changelog

Versions use Semantic Versioning, with prerelease identifiers for alpha builds.

## 0.1.0-alpha.2 — review candidate

Local review candidate built on 2026-09-22. Not published.

### Touch controls

- Machine motion, framing, Start and Resume, the laser pulse, gas tests and the
  laser mode switch act only after a held press; setting an origin is held too.
  A tap only prompts "Press and hold". The hold times (1 s and 0.75 s by
  default) are shared by every screen and adjustable from 0.3 s to 3 s under
  Settings → Display.
- Stop and Pause stay one tap, and a Stop is in the desktop top bar on every
  page.
- Deleting parts, jobs and recipes, discarding drafts and edits, writing
  controller settings, applying matrix correction, turning a job's checklist
  off and disconnecting mid-job ask for confirmation.
- Disabled machine controls say why on screen; touch targets are at least
  44 px; errors stay until dismissed; dialogs close only on a completed tap.
  The rules are in `ui/DESIGN.md`.

### Fixes

- Run and Frame are refused while a job is paused; Frame previously discarded
  the paused job.
- Switching between fiber and CO₂ keeps the XY reference while the controller
  reports it and the axis configuration is unchanged.
- A bad head calibration is reported without dropping the controller link.
- Held jog and output controls keep working after many page reloads.
- DXF drawings without units warn that they were read in millimetres.
- Multi-sheet nesting compresses the last sheet with the remaining search
  time, and starts a new sheet before preparation's contour limits.
- An unreadable library file is left untouched and listed on the Parts page
  instead of preventing the library from opening.
- SVG files with a document type declaration import; entity declarations are
  refused.
- Quitting from the macOS Dock or at logout shuts down the machine service.
- Motion sampling keeps its cadence across segments shorter than one
  interpolation cycle.

### Validation limits

Automated checks and simulator runs only. The carried-over XY reference and
the sampling change are not yet verified on a physical controller.

## 0.1.0-alpha.1 — 2026-09-17

Initial public source alpha. Application binaries are not published.

### Operator workflow

- Pause retains the execution position, controller connection and visible cut
  history. Manual movement while paused preserves the return point for Resume.
- Stop ends the run and opens postflight, including when paused.
- Editable preflight, pause and postflight checklists, with direct Edit actions
  and gas testing that preserves completed steps for the same setup.
- Larger XYZ controls and a full-width Frame action.
- Opening a part starts without a previous job's stock or nesting attached;
  saved jobs and pending drafts retain their own setup.
- Sheet history, inspected remnants and multi-sheet nesting with recorded cut
  areas excluded from future layouts.

### Materials and settings

- Bundled clean MLaser metal recipes and default vector material artwork.
- Editable nozzle diameter/type and manual optical focus; original MLaser XML
  remains importable when those optional fields are absent.
- CO₂ uses High Air for cutting, piercing and gas checks.
- Larger recipe actions and revised material cards and header.
- Compact Settings navigation, a separate Matrix correction tab and Import
  backup.xml alongside Read controller / Write controller.

### Desktop and documentation

- Windows x86_64 and universal Intel / Apple Silicon macOS packaging.
- The UI, assets and notices are embedded in the application.
- Closing the native window shuts down its Rust service.
- Controller command completion publishes its resulting state before replying,
  avoiding a native-app startup race that could report connected without binding
  the machine settings.
- Local-network hosting at `http://openlaser.local` on port 80 by default.
- Version shown in Settings and available with `--version`.
- Setup/operator manual and reproducible packaging instructions.

### Validation limits

Automated checks and simulator runs verify software behavior. They do not certify
machine compatibility, cut quality or physical safety. Real-machine pause/resume
retesting and Windows runtime acceptance remain release-review tasks unless a
separate acceptance record says otherwise. The macOS alpha is ad-hoc signed and
not notarized; the Windows alpha is not publisher-signed.
