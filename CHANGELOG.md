# Changelog

Versions use Semantic Versioning, with prerelease identifiers for alpha builds.

## 0.1.0-alpha.4 — review candidate

Local review candidate built on 2026-09-24. Not published.

### Build

- `scripts/build_release.py` (`just release-build`) builds the UI, both Mac
  slices and Windows with the builder's home folder, Cargo home and checkout
  rewritten by `--remap-path-prefix`, and fails if a home-folder path is left
  in a binary. `scripts/package_release.py` refuses such binaries.

### Plain language

- Alarms have plain names and a "how to fix" line; the vendor label and bits
  are under Details. Every row that homing the head clears shares one card
  and one Home head button.
- Run shows one status line instead of three copies of the same refusal; tap
  it for which controls are blocked, the alarms and the original technical
  text. Toasts reword developer text such as `ManuParam.FC…` and keep the
  original under Details.

### Small screens and touch

- Four type sizes (13, 15, 18 and 24 px), nothing smaller, and every control
  at least 44 px. The phone and desktop layouts switch live as the window
  resizes. Fit frames the sheet and its parts, and cut lines are brighter.
  A selection no longer has a drag handle to scale it; Scale and the typed
  sizes set exact sizes.

### Run and jog

- One Run chip beside Cut / Dry run says the next step (connect, home XY,
  home Z, calibrate Z, set origin) or why Start waits. Cut, dry run, frame
  and the layer views float over the machine area.
- Start needs Z calibrated for the current material at the current W table
  position; a changed material, reconnection or moved table asks again.

- A 3×3 jog pad with diagonals. A tap moves one step (0.1, 1 or 10 mm); a
  press held past 0.4 s jogs continuously and stops on release. Go to X/Y
  takes typed job or machine coordinates and moves on a held Go.
- Laser and gas tests, manual outputs and the mode switch moved from Settings
  to Run → Machine tests.

### Setup and sheets

- Sheets on hand: full sheets per material, thickness and laser with counts,
  kept in library folders beside their jobs, with remnants alongside. A
  completed cut takes one off.
- A nest fills several kinds of sheet in order: a suggested remnant, then
  sheets on hand, then new sheets, each within its count. The progress bar
  keeps moving while the search packs.
- The sheet chooser rotates a sheet, offers common sheet sizes laid along the
  bed and keeps saved sizes on the machine for every screen. Add text moved from the parts library to Setup
  and joins the open job.
- The drawing bar is one row: editing tools appear only with a selection, and
  Order drags the tools into your own sequence.

### Layers

- Layers as in LightBurn: ordered top to bottom by drag, each with a colour
  (from DXF layer and entity colours and SVG strokes), Output, Show, a
  recipe, and Cut or Mark. Marks never make holes or parts and carry no
  machining unless given their own; leads, joints, cooling, kerf and start
  can be set per layer. Cut layers run on the job's material; only a Mark
  layer among others asks for a recipe of its own.
- A tap selects the part whose line is nearest. Inside a shape each line's
  halo grows until it meets the next, so the middle of a hole takes the
  hole; outside every shape a halo reaches a fingertip. A second tap takes one shape of it, so inner shapes can be
  moved to another layer. The selection glows, so it shows on touch
  screens; one shape taken alone glows strongly and the rest of its part
  fades. A drag inside the selection's box moves it. Layer colours leave out
  the selection's orange.
- The machining bar and the drawing bar are one component, arranged with
  Order the same way; machining tools have icons.
- Setup's side panel fits without scrolling: a one-row material with its
  values, then the layers under it, and gas and laser folded to one line.
- One place for layers: the list under the material drags to reorder and
  switches output; tapping a layer opens its sheet (colour, name, show,
  Cut or Mark, recipe, machining). **Layer** in the drawing bar only moves
  shapes, and a new layer opens its sheet. Layer colours follow their names,
  so reordering keeps them.

### Materials and recipes

- Import files, folders or a drop, with a review that assigns each file to a
  material, flags duplicates (same material, thickness and gas) with
  Replace, Keep both or Skip, and reads nozzle, focus and lens from the notes
  and file names. One material summary everywhere; Power is the peak power
  and Duty the duty cycle.
- Recipe fields use M-Laser's names (Cut Power, Peak Current, Cut Freq …),
  with a line explaining the common ones.

### Settings

- One sidebar (General, Machine, Materials & processes, Calibration, Network
  & phones, Advanced controller parameters) with search and real switches.
  Settings save as they change; Pending changes appears only for what writes
  the machine or unsaved jobs.

### Parts library

- One filter row with search and a Filter menu for laser type and favourites,
  one place to import, larger previews and correct counts ("1 path").

### Import

- DXF splines, ellipses and blocks (mirrored, nested and arrayed inserts)
  become arcs and lines; small gaps are joined and duplicates removed, with a
  report; hidden layers are left out and layers can be chosen. SVG sizes use
  real units and ask when the scale is ambiguous; circles and arcs stay true
  arcs. A review shows the size check and open or self-crossing shapes. 27
  authored CAD fixtures with curve-accuracy tests.

### Gas and laser costs

- Laser-on and gas-on time per gas from the compiled program and each run's
  actual figures; Settings → Gas costs (refill or bulk price, compressor air,
  flow from the nozzle or a fixed L/min, currency); a Gas & laser card with
  past runs and totals; a 60-second gas calibration. Imperial units show gas
  in cubic feet. Flow estimates are not yet checked against a machine.

## 0.1.0-alpha.3 — review candidate

Local review candidate built on 2026-09-22. Not published.

### Plate contact and pauses

- A pause, from the Pause button or from an alarm such as the nozzle touching
  the plate, now switches the laser off with the stop, raises the head to the
  machine's safe height (`ZFSafeHeight` at `ZFUpSpeed`), and only then switches
  the gas off. The head rises only when it is referenced, free of Z faults, not
  under an emergency stop, and below the safe height. The
  pause waits at most 3 s for the head, so a head controller that stays busy
  after a touch no longer holds the screens in "running" until the pause
  fails. Each step is written to `server.log`. Stop and
  a pause after lost feedback still switch everything off at once and leave the
  head where it is.
- While a program cuts or frames, the head's alarm word is read between the
  full feedback polls, so a touch pauses the program within a read or two
  instead of up to a poll later. A controller that answers slowly falls back to
  the ordinary poll.
- A head jogged down onto the plate stops and rises to the safe height, and a
  head touching the plate may be jogged up.

### Jobs of several parts

- Pick several parts on the Parts page (desktop and phone) and set them up as
  one job: they open side by side in the order picked, wrapping at the bed's
  width, each part its own shape for nesting.
- Setup lists the job's parts; **Add parts** adds more beside the layout as one
  undoable edit, and a part's shapes can be selected from the list. Removing
  every shape of a part takes it out of the job; Undo brings it back.
- A saved job keeps its parts in order. Jobs of one part are stored exactly as
  before; a job of several parts is job schema 2, which earlier builds decline
  to open. Retained working copies follow the same rule.
- A part used by a saved job cannot be deleted, and the refusal names the job.
  Saving nested sheets makes each sheet a job of only the parts on it.
- When a saved job's parts change on two screens, its parts and layout merge
  as one choice instead of value by value.

### Nesting

- The drawing shows a running search as it goes: each part as it is placed and
  each tighter arrangement, on the sheet the search last changed, with that
  sheet's own stock and cutouts. Only the checked result can be applied.
- Nesting several parts onto a remnant and overflowing onto fresh sheets is
  covered by an end-to-end test.

### Drawings

- **Simplify drawing** saves a part's drawing as a new part with runs of short
  lines joined into arcs and lines within 0.01 to 0.1 mm, repeated contours on
  the same layer and specks removed. The original part is unchanged.
- One sheet now prepares up to 20 000 contours (was 5 000) and 200 000 lines
  and arcs (was 100 000); heat spreading takes a full sheet and nearest-next
  ordering 10 000 contours. Preparation measured about 0.3 s at 20 000 contours.
  Limit messages name the limit and what to do.

### Fixes

- Nesting compacts from the first fit's own width, so a sheet is never
  looser than its first arrangement; identical parts pack into a tight block
  instead of spreading across the sheet.
- Side panels scroll when their content is taller than the screen; Setup
  keeps Save, Dry run and Go to Run in reach below its cards.
- Re-nesting one sheet of an unsaved set numbers the new sheets from it.

### Validation limits

Automated checks and simulator runs only, including multi-part jobs, live
nesting and nesting onto a remnant.

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
