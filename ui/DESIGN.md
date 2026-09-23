# OpenLaser interface guidelines

The interface runs on a touch screen beside the machine and on phones in the
shop. These rules keep it deliberate to operate and quick to stop. Every
control in `ui/` follows them; a change that adds a control says which rule it
follows.

## Touch rules

They are distilled from [ncSender](https://github.com/siganberg/ncSender)'s
touch-first CNC interface and adapted for a laser.

1. **Hold to act.** A control that moves the machine, fires the beam or turns
   an output on by itself acts only after a press is held: by default 1 s for
   motion, beam and outputs, and 0.75 s for setting an origin or reference.
   Both times are one setting shared by every screen, adjustable from 0.3 s to
   3 s under Settings → Display and saved in Pending changes. Use
   `components/HoldButton.svelte`, which reads the setting at every press. A
   tap does nothing but say "Press and hold".
   - Held for 1 s: Start, Resume, the checklist's Start, Home, Go origin,
     Calibrate, Frame, move to restart, checklist and postflight moves, gas
     tests, the laser pulse, an alarm relief that moves an axis, and the laser
     mode switch.
   - Held for 0.75 s: Set origin, Absolute positioning, and checklist actions
     that set the origin.
2. **Fire once, at the threshold.** The action runs the moment the hold
   completes, while the finger is still down, never again on release. Nothing
   ever fires later by itself: no tap-to-arm countdowns.
3. **Show the hold.** A fill grows across the button from 150 ms, so a quick
   tap does not flash. A small "Hold" mark sits in the corner. A completed hold
   shows a green edge; an early release shows a red outline and "Press and
   hold".
4. **Abandon, never guess.** Releasing early, sliding more than 12 px or off
   the control, losing the pointer, a blurred window, a hidden page or the
   control becoming disabled mid-hold all do nothing. One press at a time.
   Enter and Space must be held too.
5. **Deadman motion.** Jog, table, Z and manual outputs run only while held:
   `lib/hold.ts` leases the press, renews it every 75 ms and releases on any
   doubt, and the controller stops them when renewals stop. Their buttons use
   `touch-action: none` (`.deadman`, `.jog`, `.zcol`) so a drifting finger
   never turns into a scroll that cancels the press.
6. **Stop and Pause are one tap, everywhere.** They are never held and never
   behind a confirmation. While the machine is connected a Stop is on every
   screen: the desktop top bar, the phone's Stop bar, the run controls and the
   control gate. Nothing may cover them.
7. **Confirm what loses work or changes the machine.** Deleting a part, job or
   recipe, discarding a draft or staged edits, turning a job's checklist off,
   writing controller settings, applying matrix correction and disconnecting
   mid-job ask first through `ui.confirm()`. The confirming button names the
   consequence ("Delete part", "Stop and disconnect"), in the danger colour
   when work is lost. Routine, undoable edits are not confirmed.
8. **Say why on screen.** A touch screen has no hover, so a disabled machine
   control shows its reason as text (`.gate-reason`), not only in a tooltip.
9. **Big targets.** Every control is at least 44 px (`--touch` 48 px for
   buttons; 64 and 80 px for the primary machine actions), with gaps of 8 px
   or more.
10. **Hardened surfaces.** Controls never open a long-press callout, select
    text or zoom on a double tap (`touch-action: manipulation`). Held
    controls use `touch-action: none`. On the canvas a finger must travel
    12 px (a mouse 4 px) before a tap becomes a drag.
11. **Errors stay.** An error stays on screen until dismissed, and never
    covers a Stop control. Other notes fade.
12. **Dialogs close on purpose.** A dialog closes on its close button or on a
    tap that both starts and ends on the backdrop. A dialog running motion
    cannot close until that motion ends; it shows Stop instead.

### Not copied from ncSender

- **Countdowns armed by a tap.** They would turn a stray tap into a delayed
  move or beam with nobody at the controls.
- **Retrying an unlock for seconds.** Clearing a hazard must never re-enable
  the machine without a new, deliberate action.
- **Any delay before the beam goes off on Stop.**
- **Holds that fire on release.** Every hold fires at the threshold (rule 2).
- **Hidden double-tap modes, and labels that read as "set zero" but move the
  machine.** Go origin (a move) and Set origin (a reference) stay visibly
  different.

## Colour

| Colour | Means |
| --- | --- |
| Green | Motion |
| Dark green | Start |
| Red | Stop, danger and destructive confirmations |
| Amber | Hold, alarms, caution, gas and shutter outputs |
| Orange | Selection and primary actions |

Gas figures use their own colours so a mix reads at a glance: blue for
nitrogen, red for oxygen and grey-green for air (`--gas-n2`, `--gas-o2`,
`--gas-air`). They label figures only and never mark a control.

Tokens live in `:root` in `src/app.css`, and the dark theme overrides them.
Stylesheet rules stay one per line, in cascade order.
