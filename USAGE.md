# OpenLaser setup and operator manual

**Version 0.1.0-alpha.2 · Alpha documentation**

The public repository provides source and documentation; prebuilt application
binaries are not published. Start with the [source build instructions](README.md#build-from-source).
Package installation steps below apply when you build your own review packages.

OpenLaser runs on the machine computer. Its desktop window and any connected
phones share one library, one workspace and one controller session.

This guide describes the current alpha. Controller simulation and automated
checks establish software behavior; they do not establish physical compatibility
with every machine. Verify machine configuration and operation during commissioning.
Keep the machine's physical emergency stop accessible. A software Stop depends
on a working controller connection and is not a replacement for it.

## Contents

- [Install and launch](#install-and-launch)
- [Try the simulator](#try-the-simulator)
- [First machine setup](#first-machine-setup)
- [Parts, jobs, materials and sheets](#parts-jobs-materials-and-sheets)
- [Run your first job](#run-your-first-job)
- [Several parts in one job](#several-parts-in-one-job)
- [Simplify a drawing](#simplify-a-drawing)
- [Nest parts and reuse a remnant](#nest-parts-and-reuse-a-remnant)
- [Pause, resume, stop and restart](#pause-resume-stop-and-restart)
- [Materials and process settings](#materials-and-process-settings)
- [Settings and checklists](#settings-and-checklists)
- [Matrix correction](#matrix-correction)
- [Phones and local networking](#phones-and-local-networking)
- [Data, backups and updates](#data-backups-and-updates)
- [Troubleshooting](#troubleshooting)
- [Command line and configuration](#command-line-and-configuration)

## Install and launch

The release packages are:

| Package | Contents |
| --- | --- |
| `OpenLaser-0.1.0-alpha.2-windows-x86_64.zip` | 64-bit Windows executable, simulator launcher and documentation |
| `OpenLaser-0.1.0-alpha.2-macos-universal.zip` | Intel / Apple Silicon application, simulator launcher and documentation |
| `OpenLaser-0.1.0-alpha.2-source.zip` | Matching source, embedded UI build and build instructions |

### Windows

1. Extract the complete Windows ZIP into a folder you can keep.
2. Open `OpenLaser.exe`. The app starts disconnected.
3. If Microsoft WebView2 is missing, the bundled Microsoft installer performs a
   one-time setup that needs an internet connection.
4. Keep the included manual and notices with the application.

The executable includes the UI and runtime loader; an adjacent UI folder or
`WebView2Loader.dll` is not needed. This alpha is not publisher-signed, so Windows
may show an unidentified publisher warning. Obtain the package from the project's
release and compare its SHA-256 checksum before installing it.

### macOS

1. Extract the Mac ZIP. The same application supports Intel and Apple Silicon
   Macs running macOS 11 or later.
2. Move `OpenLaser.app` into Applications, or leave it in the extracted folder.
3. Open it. The app starts disconnected and uses macOS's built-in WebKit.

This alpha is ad-hoc signed for bundle integrity, not Apple-notarized. macOS may
require you to review the app in Privacy & Security before allowing it to open.
The package has not been verified on every supported macOS version.

### Closing the app

Closing the native OpenLaser window stops the machine session and waits for the
Rust service to exit. Do this after stopping or finishing work. A second launch
reuses the existing instance for that library.

Closing a browser tab or phone page does **not** shut down an independently
started server. Use `--shutdown` with the same configuration/data arguments to
stop that server. See [Command line](#command-line-and-configuration).

## Try the simulator

Use the included **Start simulator** launcher while it is beside `OpenLaser.exe`
or `OpenLaser.app`. It opens a simulated controller and uses a separate
`OpenLaser-Simulator` data folder. It does not connect to the machine controller.
The launcher loads the included synthetic machine fixture. That fixture is only
for simulation; never import it as a real machine's configuration.

You can also launch it explicitly:

Windows PowerShell, from the extracted folder:

```powershell
.\OpenLaser.exe --simulate --config .\simulator\machine.toml --data "$env:LOCALAPPDATA\OpenLaser-Simulator"
```

macOS Terminal, after moving the app to Applications:

```sh
/Applications/OpenLaser.app/Contents/MacOS/OpenLaser --simulate \
  --config /Applications/OpenLaser.app/Contents/Resources/simulator/machine.toml \
  --data "$HOME/Library/Application Support/OpenLaser-Simulator"
```

Connect and home the simulated machine using the interface. Import a drawing,
choose a material, compile it and exercise Start, Pause, jog, Resume and Stop.
The simulator defaults to `http://127.0.0.1:8080` and loopback-only access. It
simulates motion/protocol behavior, not cut quality, collisions or real gas flow.
Use your own commissioned backup when moving from simulation to a real machine.

## First machine setup

1. Keep an untouched copy of the working vendor machine backup and process INI.
2. Open **Settings → Machine settings → Import backup.xml**. Review the staged
   import and save it. This is the machine's configuration, not a material recipe.
3. If available, import the vendor process INI under
   **Settings → Controller & laser → Process settings**. Review and save it.
4. In **Controller & laser**, configure the controller address, local controller
   network interface and the intended Fiber or CO₂ mode. The example MCC network
   uses controller `10.1.1.168:502` and host `10.1.1.10`; use your machine's actual
   addresses and network configuration.
5. Review axis travel, direction, scale, homing, laser, safety inputs and output
   assignments against the working machine configuration. Imported values are
   machine-specific; material defaults are not a substitute for this setup.
6. Press **Connect** in the top bar. Connection initializes saved controller
   settings and verifies their readback. It does not home or start a job.
7. Resolve any reported mismatch or unsupported configuration before proceeding.
8. Home the machine and verify motion, references, interlocks and outputs through
   your normal commissioning procedure before cutting.

Selecting a job or compiling can select its recipe's Fiber or CO₂ mode. A mode
change while connected requires homing again before a run.

CO₂ uses **High Air** for cutting, piercing and gas checks. Assign the High Air
valve in the machine settings and set its pressure at the regulator. Manual
optical focus is a physical setup value; it is separate from Z following,
pierce height, retraction and the CO₂ manual-focus/controller-bypass setting.

## Parts, jobs, materials and sheets

| Item | What it represents | What opening it restores |
| --- | --- | --- |
| **Part** | Reusable drawing geometry | A fresh setup with no material, sheet or old nesting layout attached |
| **Job** | A prepared cutting setup | Its geometry, preparation, material, stock selection, layout and saved placement settings |
| **Material recipe** | Reusable cutting/piercing settings | Process values for a material, thickness and laser mode |
| **Sheet / remnant** | Physical stock and recorded cut areas | A boundary and the areas that must remain clear |
| **Pending changes** | An unfinished workspace | The particular draft you left unfinished, separate from its source part |

Creating a job does **not** create a new material. Select an existing recipe and
reuse it across jobs. Save a separate recipe only when you deliberately want a
new material/process variant.

The main flow is **Parts → Setup → Run**. Settings configures the machine and
application. Setup contains material, machining and nesting tools; they are not
a locked wizard. You can choose stock before material, but selecting it does not
assign a cutting recipe. For a remnant, choose material first so compatibility is
clear. Compatibility uses material name, thickness and laser mode, not the job's name.

```text
Import/open a PART ──→ Fresh setup ──→ Choose material ──┐
                                                       ├─→ Prepare / optional nest
Open a saved JOB ────→ Saved setup ──→ Review or change ─┘
       → Compile → Position & frame → Preflight → Run → Postflight
                                                         ↓
                                               Inspect sheet history
                                                         ↓
                                              Save usable remnant
                                                         ↓
                                          Select in a future job & nest
```

## Run your first job

1. In **Parts**, import a DXF/SVG, use the drawing tools, or open an existing part.
   Verify dimensions and closed/open contours. Text must become outlines for cutting.
   To cut several parts together, [pick them](#several-parts-in-one-job) first.
2. Move to **Setup** and choose an existing material/thickness recipe. Check its
   laser mode, nozzle, manual focus, gas and process values for your machine.
3. Prepare the geometry as needed: kerf, leads, micro-joints, bridges, cooling and
   cut order. Inspect the preview. For a one-off, nesting is optional; selecting
   actual stock makes later sheet history and remnant reuse clearer.
4. If cutting multiple parts, choose stock and [nest](#nest-parts-and-reuse-a-remnant).
   Apply the reviewed result before compiling.
5. Save a **job** when you want to reuse the complete setup. Saving a part alone
   does not attach this sheet to it. Compile the prepared job and resolve errors.
6. Open **Run**. Connect and home if needed. Position the physical sheet, jog the
   head and choose the origin behavior below.
7. Use **Frame** to inspect placement on the actual stock. Framing moves the head;
   check that the travel is clear.
8. Press **Start**, complete the **Preflight Checklist**, and start the cut.
   **Test gas** is an optional action inside the gas check; it does not erase
   previously checked steps for the same setup.
9. Watch the run. Use Pause for a temporary interruption or Stop to end it.
10. Complete **Postflight Checklist** when the run ends. Parking is an explicit
    action; merely opening the checklist does not move the machine.
11. Inspect the physical stock before saving a remnant for later use.

### Placement and origin

- **Each run:** uses a fresh origin for each new run. **Set origin** captures the
  current head position. If you have not set one, Frame or Start captures it.
  Preparing and compiling do not require that position yet.
- **Absolute:** captures absolute machine coordinates for a fixture and keeps
  them across runs. Save the job to retain the fixture location.
- **Set origin:** captures the current head coordinates again without changing
  the selected origin mode. Jogging alone leaves the captured origin in place.

Position-dependent correction is finished after placement. Check framing again
when changing stock, origin, placement or correction.

## Several parts in one job

A job can cut several library parts. On **Parts**, press **Pick parts** (or
**Cut with other parts…** in a part's details), tap each part to cut, then
**Set up job with N parts**. They open side by side, 10 mm apart and in the
order picked, wrapping onto a new row at the bed's width; nest them to fill a
sheet. Folders still open while picking, so parts can come from several. Saved
jobs cannot be picked: a job keeps its own parts.

In Setup, the **Parts** card lists the job's parts. **Add parts** puts more
beside the layout as one undoable edit, and tapping a part in the list selects
its shapes on the drawing. A part cannot be added twice; copy it on the sheet
instead. Delete a part's shapes and the part leaves the job; Undo brings it back.

The job is saved with its parts, and each part stays in the library as it was.
A part cannot be deleted while a saved job cuts it. Saving a set of nested
sheets makes one job per sheet, each cutting only the parts on its sheet.
Matrix correction applies when any of the parts came from a DXF. Jobs of one
part are stored exactly as before; a job of several parts needs this version or
later to open.

## Simplify a drawing

CAD exports often flatten splines, ellipses and fillets into thousands of very
short lines. In a part's details, **Simplify drawing…** joins such runs into the
arcs and lines they follow, drops contours that repeat another on the same
layer (which would otherwise be cut twice), and drops specks smaller than the
tolerance. Choose **Fine** (0.01 mm), **Standard** (0.02 mm), **Coarse**
(0.05 mm) or **Rough** (0.1 mm); nothing moves further than that, and the
dialog shows the line, arc and contour counts before anything is saved. The
result is saved as a new part named "… simplified"; the original stays for the
jobs that use it. Simpler drawings prepare faster, stay further from the sheet
limits below and send less to each screen.

One sheet prepares up to 20 000 contours and 200 000 lines and arcs; nearest-next
cut ordering takes up to 10 000 contours. Split larger work over several sheets
with nesting.

## Nest parts and reuse a remnant

Open **Nest parts** from Setup. Select the intended stock, preview the sheets,
then apply the result. Stock selection by itself does not rearrange parts.

| Option | Effect |
| --- | --- |
| Rectangular stock | New sheet with the entered dimensions |
| Drawing outline | Use a closed drawing contour as the stock boundary; it is excluded from cutting |
| Remnant stock | Use a saved stock boundary while avoiding recorded cut areas |
| Selected part quantity | Total copies, including the original; select one part/group to edit it (1–500) |
| Part spacing | Gap between newly nested parts |
| Edge margin | Clearance from the stock boundary |
| Extra remnant clearance | Additional room around old cutouts and the remnant edge for alignment |
| Dense | Allow any rotation angle |
| Keep grain | Allow 0° / 180° rotations |
| Across grain | Allow 90° / 270° rotations |
| Fixed | Do not rotate the part |
| Search time | 5, 10 or 30 seconds; more time allows more arrangement search |

Other parts already in the draft stay in the nesting set. Quantity applies to
the selected part/group. Remove connected/common-edge arrangements before
re-nesting independent parts. Lead and kerf clearance is included; part holes
remain reserved rather than being automatically filled with more parts.

**Preview sheets** searches for a layout. While it searches, the drawing shows
the sheet it is working on, each part as it is placed and again each time the
sheet packs tighter; that view is never applied. Review every numbered sheet
and then press **Apply layout** or **Apply N sheets**. Until Apply, the result is a preview.
If the drawing changes, preview again. Undo restores the previous applied layout.
Saving a multi-sheet set creates a folder with one job per sheet.

When parts overflow, additional fresh sheets are generated. A remnant is used
only on the first sheet; overflow uses fresh **rectangular** stock of the same
bounding dimensions. Review and supply those additional sheets before running.

### Save the stock left after a cut

1. Complete the run, then open **Parts → Sheets → Cut history**.
2. Inspect the actual sheet and record the usable remnant. If the original job
   had no stock outline, enter the actual rectangular bounds first.
3. Choose **Save inspected remnant** and give it a recognizable name.
4. Store/label the physical sheet so you can identify and orient it later.

The record keeps the outer boundary, removed part footprints, open cuts, kerf
and previous leads as unavailable areas. It is a record of the cut, not a camera
measurement of bent stock, loose pieces, debris or later damage.

### Use it in a later job

| You want to cut… | Steps |
| --- | --- |
| 25 of the same part | Open the part afresh, choose its existing material recipe, select the remnant, set quantity to 25, Preview sheets, Apply, compile and frame |
| A different part | Open/import that part, choose a compatible material/thickness/mode, select the remnant, then preview and apply a new nest |
| An already saved job | Open it (duplicate/save separately if you want to preserve the original), replace its old stock with the remnant, re-nest and review the full layout |

**Run again** does not automatically select the newest remnant. A saved job
retains its original stock/setup until you change it. Opening its source part
still starts without a sheet attached.

Selecting stock or saving a job does not consume a remnant. Starting a cut
reserves it against reuse. After a completed cut, inspect the resulting sheet
history and save the next usable remnant. Stopped runs keep conservative
reservations and do not automatically become ready stock. Explicitly marking a
saved job as cut creates an operator-reported record, identified as such.

The software does not automatically locate a remnant on the bed. Match its
physical orientation, set the origin, allow suitable remnant clearance and frame
the new layout before cutting.

## Pause, resume, stop and restart

- **Pause** holds the active execution and retains its stopped position, cut
  history and controller connection. It opens the **Pause Checklist**.
- Once held, use the manual controls to move the head away if necessary. This
  does not replace the saved resume position.
- Complete the pause checks. **Test gas** is also available for the paused job's
  gas selections. Resume returns to the retained position with the beam off and
  continues the retained execution.
- **Stop** requests an immediate stop, performs job cleanup and opens postflight,
  including if the job was paused. It ends the active run.
- **Adjust restart** is the separate restart editor. Choose a path and a point by
  distance or percentage. Completed paths remain visible. Skips and marks belong
  to the retained execution; later draft edits do not rewrite that history.

A restart after stopping requires a valid machine reference and confirmation
that sheet placement is still correct. If the controller link is lost, do not
assume the machine position or paused execution remains valid. Resolve the
connection error and verify the physical state before deciding how to restart.

Start, Pause and Resume retain the drawing's zoom and pan.

## Materials and process settings

The bundled library contains 50 metal recipes from the supplied clean MLaser
library plus a basswood CO₂ recipe. These are starting recipes whose suitability
must be checked for your machine, laser power, optics, nozzle and stock.

Material cards open the recipe editor. The header contains material, thickness,
mode and recipe actions. Under **Cutting → Head setup**, edit nozzle diameter,
single/double nozzle type and manual optical focus. Gas selection is beside the
head setup controls. Cutting and piercing process values remain separate.

Nozzle and optical focus are operator setup values. Editing them does not move a
focus motor or command Z. Original MLaser XML files still import without these
optional fields; enter the nozzle/focus afterward when absent. Source process
values are retained on import. If description notes disagree with XML pressure
values, inspect both rather than assuming the note overwrote the process value.

Use the recipe editor to rename a material, change thickness, inspect imported
values or delete a recipe. Changes affect the material library; review any
existing job's retained settings separately. New jobs reuse the chosen recipe
instead of making a material for every job.

## Settings and checklists

The version appears beside **Settings**. Category buttons sit in that header;
Matrix correction is a peer of Machine settings.

| Tab | Purpose |
| --- | --- |
| Machine settings | Import backup.xml, search/edit grouped machine values, read and write controller settings |
| Matrix correction | Generate calibration coupons and enter/apply size measurements |
| Checklists | Edit preflight, pause and postflight defaults |
| Display | Units and interface preferences |
| Controller & laser | Controller network, mode and process INI import |
| Axes & homing | Axis and homing setup |
| Safety I/O | Input and safety signal assignments |
| Outputs | Output assignments |

Machine settings offers **All values**, **Mismatches** and **Edited** filters.
**Read controller** compares the current controller with the saved backup without
rewriting the XML. **Write controller** reapplies the saved settings. Mismatches
identify the field, register and expected/observed values. Host-only values
explicitly have no controller readback.

Edits and imports are staged for review. Saving while connected applies and
verifies machine settings; saving offline retains them for the next connection.
Pending edits survive page reloads. Keep your original vendor backup separately.

### Checklists

Preflight, pause and postflight each have editable defaults. Their popup headers
include **Edit**, which opens the relevant settings. Closing the editor discards
unsaved modal edits; **Review changes** stages them for saving.

Gas readiness is a normal editable step included in defaults. It can be changed
or removed through the checklist editor. The gas test uses the compiled job's
selections, and a paused job retains its original gas checks for Resume.
A gas test does not uncheck the preceding steps for an unchanged setup.

Postflight appears after completion or Stop. Its parking actions are explicit;
opening the popup itself does not move the machine. Normal end-of-job retraction
still follows the recipe. A parking move raises a lowered head when the shared
XY positioning guard requires it.

### Units

**Display → Units** switches dimensions, coordinates, process values and numeric
entry between metric and imperial. Inches, inches/minute and psi are display
units; geometry and machine files keep their original metric values. This
preference is remembered on the device.

## Matrix correction

1. Open **Settings → Matrix correction** for the intended laser mode.
2. Load the nine uncorrected 100 × 100 mm squares at the indicated bed positions.
3. Choose appropriate material and normal preparation for the coupon job, verify
   placement and cut it. Matrix correction is disabled for that coupon job.
4. Measure X and Y size for each square and enter the values. Measurements save
   as they are entered.
5. Apply the complete profile. It becomes available to future DXF drafts in that
   laser mode; each saved job retains its own correction profile.

Correction is applied after final placement and recalculated for the bed position
when work moves. These measurements describe local size variation; they do not
establish absolute positioning, squareness or backlash compensation.

## Phones and local networking

The native desktop app shares on the local network by default:

- `http://openlaser.local` — normal entry, no port suffix when using port 80.
- `http://openlaser.local/mobile` — phone interface.
- `http://openlaser.local/app` — full desktop interface.

For a headless server, enable sharing with `--lan` or `lan = true` in
`machine.toml`. The simulator is loopback-only unless you explicitly enable
sharing. An explicit listen address/port is respected; `--lan` selects port 80,
and `--listen` overrides it.

The host firewall must allow the HTTP port and multicast DNS on the shop network.
On macOS, allow OpenLaser in **System Settings → Privacy & Security → Local
Network**. Quit and reopen the native app after granting access. The packaged
app includes the local-network purpose and Bonjour service declarations.
The Local network panel reports discovery problems. If `.local` discovery does
not work on a device, find the machine computer's Wi-Fi/Ethernet IP address in
its operating-system network settings and use that address with the configured
port. On the machine computer itself, `http://127.0.0.1` also works on port 80.
If an old background instance has stopped announcing its name, stop it with
`--shutdown` using its data folder, then reopen OpenLaser.
Discovery excludes the controller interface. Port 80 must be available; when
using another port, include it in the URL, for example `http://host-address:8080`.

LAN sharing assumes a **trusted local network**. There are no accounts or pairing
codes. Do not expose the service to the internet. All screens share the same
draft, saved library and machine session. One screen controls the workspace at
a time; another can take control while idle. Every screen can stop motion.
Control expires after a page stops checking in; held jog/output actions use
separate short leases.

Phones follow **Parts → Setup → Run → Settings**. Manual controls live under
**Run → Controls**. Detailed editing opens separately. Use
**Settings → Display → Full interface** to switch to the desktop layout while
retaining the current page.

## Data, backups and updates

| System | Default application data |
| --- | --- |
| Windows | `%LOCALAPPDATA%\OpenLaser` |
| macOS | `~/Library/Application Support/OpenLaser` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/openlaser` |

This is where the local library and settings live. `--data` selects a different
folder. The supplied simulator launchers use a separate `OpenLaser-Simulator`
folder. A portable `machine.toml` can select storage relative to itself.

To back up or update:

1. Finish or stop the active job and close OpenLaser so the service exits.
2. Copy the **entire data folder**, plus any externally referenced machine files
   and custom `machine.toml`, to your backup location.
3. Keep the previous executable/package and a matching data snapshot.
4. Replace the app with the new release, then review configuration and test in
   simulation before the next machine run.

There is no automatic updater. Do not assume an older alpha can read data written
by a newer one. Restore the matching data snapshot when reverting versions.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Another screen has control | Take control from its popup while idle, or return to the active screen |
| `openlaser.local` does not resolve | Same network, host firewall and multicast DNS; inspect Local network, restart a stale server, or use the host IP from OS network settings |
| App cannot bind port 80 | Another service or OS restrictions may occupy/prevent it; use an explicit available port and include that port in phone URLs |
| Controller link lost / no reply to register 10000 | Check controller power, Ethernet, controller/host addresses and competing controller software; reconnect from the top bar, then verify reference and job state |
| Controller settings mismatch | Inspect the named field/register, saved backup and readback before reapplying configuration |
| Start or Frame is unavailable | Check connection, homing, laser mode, compiled job, active screen ownership and reported errors |
| A remnant is incompatible | Check material name, thickness and laser mode against the selected recipe |
| Nesting added sheets | Requested copies did not fit; review every sheet, spacing, margin and remnant clearance |
| "One sheet can prepare up to 20000" | Split the parts over more sheets with nesting, or [simplify](#simplify-a-drawing) dense drawings |
| A part cannot be deleted | A saved job cuts it; the message names the job |
| Reopened part has no sheet | Expected: open a saved job to restore a full setup, or Pending changes for an unfinished draft |
| Imported material has no nozzle/focus | Older MLaser XML may omit these optional values; enter them in Head setup |
| Windows has no native UI | Check whether the one-time WebView2 setup completed |
| Closing a phone did not exit OpenLaser | Only closing the native host window shuts down its service; a headless server needs `--shutdown` |

When reporting a bug, include the OpenLaser version, OS, simulation or real
controller, laser mode, exact steps, expected result and exact error. Include a
small reproducible drawing or redacted configuration when relevant. Do not share
private machine/network data unnecessarily.

## Command line and configuration

```text
openlaser --version
openlaser --help
openlaser [--config machine.toml] [--data DIR] [--listen ADDR]
          [--lan] [--simulate] [--no-browser] [--shutdown]
```

A native build opens its window by default. `--no-browser` starts only the HTTP
server. Without the desktop feature, run the server and open its URL yourself.

Example portable configuration (replace the addresses/files for your machine):

```toml
controller = "10.1.1.168:502"
host = "10.1.1.10"
backup = "backup.xml"
mode = "fiber"
data = "data"
lan = true
# listen = "0.0.0.0:8080"  # Optional explicit HTTP address/port.
```

An executable-adjacent `machine.toml` is used when present; otherwise the app uses
the default user-data location. `--config` selects a file explicitly. Relative
data and backup paths resolve from the configuration location. `--data` overrides
the storage location. When shutting down a non-default instance, pass the same
`--config` and/or `--data` used to launch it.

Build instructions are in [README.md](README.md#build-from-source), and packaging
instructions are in [RELEASING.md](RELEASING.md).

### Current scope

Initialization covers the ordinary five-axis MCC configuration. Backups that
request separate dual-drive, motorized optical-focus, vertical correction or
secondary CO₂ travel-limit branches are refused with the responsible field
identified. Full physical commissioning across machine variants remains open.
STEP import, fill/scan engraving, fly cutting, camera integration and automatic
updates are outside this alpha's scope.

See [CHANGELOG.md](CHANGELOG.md) for the release changes and
[CONTRIBUTING.md](CONTRIBUTING.md) for development and verification.
