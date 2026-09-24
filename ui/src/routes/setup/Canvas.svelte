<script lang="ts">
  import { quantity } from '../../lib/units.svelte';
  // The part on the bed, shape by shape: an outline with what it encloses
  // is one shape. A tap selects a shape, a shift-tap adds one, a marquee
  // takes every group it touches, and a tap on nothing clears. A drag moves the
  // selection and the handle turns it; the change is drawn locally and
  // posted once, on release, so the drawing follows the finger and the
  // toolpath catches up. Copies are pasted beside what was copied, a gap
  // apart and further each time; undo and redo are the server's. While a panel asks for places, a tap
  // snaps to a contour and the feature takes the spot.
  import { untrack, type Snippet } from 'svelte';
  import Stage from '../../components/Stage.svelte';
  import ToolBar, { type Tool } from './ToolBar.svelte';
  import LayerAssign from './LayerAssign.svelte';
  import PickBar from './PickBar.svelte';
  import SelectionHud from './SelectionHud.svelte';
  import { Viewport } from '../../lib/viewport.svelte';
  import { api } from '../../api/client';
  import { featureEdits } from '../../stores/feature-edits';
  import { LatestPick, orderGroups } from '../../lib/picking';
  import { orderPosition, orderSegments } from '../../lib/cut-order';
  import { copiedLeadOverrides, setLeadOverrides } from '../../lib/lead-overrides';
  import { draggedLead, leadHandlesOf, type LeadHandle } from '../../lib/lead-handles';
  import { pasteBatch, pasteable, type CopiedShapes, type PasteSettings } from '../../lib/copy-paste';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { frameOf } from '../../lib/frame';
  import { explain, plural } from '../../lib/format';
  import { withBusy } from '../../lib/busy';
  import { about, apply, centre, mirror, mirrorVertical, rotation, scaling, svgMatrix, tenth, translate, type Point } from '../../lib/transform';
  import { pathOf, type Box } from '../../lib/svg';
  import { layerColor } from '../../lib/drawing-layers';
  import {
    drawingPath, hitPath, boundsOfShapes, marqueeGroups, nearestShape, smallShapeAround, unionBounds, axisAlignedBounds, transformedBounds, BoundsIndex, inverseBounds,
  } from '../../lib/viewer-geometry';
  import type { Features, PreviewContour, Spot, Transform } from '../../api';

  let { selectedContours = $bindable([]), orderProgress = 0, clipboard = $bindable(null), pasting = $bindable(false), pasteSettings }: {
    selectedContours?: number[]; orderProgress?: number; clipboard?: CopiedShapes | null; pasting?: boolean; pasteSettings: PasteSettings;
  } = $props();

  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const scene = $derived(server.canvasDraft);
  const updating = $derived(draft?.error === 'preparing geometry');
  const layers = $derived(draft?.layers ?? []);
  /** Marked layers draw dashed: traced on the surface, not cut through. */
  const marked = $derived(new Set(layers.filter((l) => l.mode === 'mark').map((l) => l.name)));
  /** Each layer's colour; none draws in the screen's cut colour. */
  const colors = $derived(new Map(layers.map((l) => [l.name, layerColor(layers, l.name)])));
  const preview = $derived(!updating && ui.picking?.revision === draft?.revision ? (ui.picking?.preview ?? scene?.preview ?? null) : scene?.preview ?? null);
  const groups = $derived(scene?.groups ?? []);
  const stockOutline = $derived(ui.nestPreview ? ui.nestStock?.outline ?? scene?.stock_outline ?? [] : ui.nestLive?.stock_outline ?? scene?.stock_outline ?? []);
  const stockCutouts = $derived(ui.nestPreview ? ui.nestStock?.cutouts ?? [] : ui.nestLive?.stock_cutouts ?? scene?.stock_cutouts ?? []);
  const frame = $derived(frameOf({ ...doc, draft: scene }));

  /** What the machine adds to a drawing coordinate: the canvas is in machine coordinates. */
  const zero = $derived(frame.zero);
  const toDrawing = (p: Point): Point => [p[0] - zero[0], p[1] - zero[1]];

  const view = new Viewport();
  /** The sheet and the parts on it, in drawing coordinates: what Fit frames. */
  const content = $derived.by(() => {
    // While nesting, the old layout is not on the sheet shown.
    const parts = ui.nestPreview ? ui.nestPreview.bounds : ui.nestLive ? null : preview?.bounds;
    const points = [...stockOutline, ...(parts ? [[parts.min.x, parts.min.y], [parts.max.x, parts.max.y]] : [])];
    if (!points.length) return null;
    const xs = points.map((p) => p[0]!), ys = points.map((p) => p[1]!);
    return { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) };
  });
  // Fit frames the sheet and its parts; the bed is only the fallback with nothing drawn.
  const fit = () => {
    view.setLimit(frame.bed);
    const b = content;
    if (b) view.fit({ minX: b.minX + zero[0], maxX: b.maxX + zero[0], minY: b.minY + zero[1], maxY: b.maxY + zero[1] });
    else if (frame.bed) view.home();
  };
  let fitted = '';
  // Refit on a new job, and when nesting shows a sheet of another size.
  const sheetKey = $derived(content && (ui.nestPreview || ui.nestLive)
    ? [content.minX, content.minY, content.maxX, content.maxY].map((v) => Math.round(v)).join(',') : '');
  $effect(() => {
    const key = `${draft?.generation ?? ''}/${draft?.job ?? ''}/${frame.bed ? 'bed' : ''}/${content ? 'parts' : ''}/${sheetKey}`;
    if (key !== fitted) { fitted = key; untrack(fit); }
  });
  $effect(() => () => { ui.picking = null; });

  // Shapes: the preview contours of each group, by the drawing contour they came from.
  const groupOf = $derived(new Map(groups.flatMap((members, g) => members.map((c) => [c, g] as const))));
  const shapes = $derived.by(() => {
    const by = new Map<number, PreviewContour[]>();
    for (const contour of preview?.contours ?? []) {
      const g = groupOf.get(contour.sources[0] ?? -1);
      if (g === undefined) continue;
      const contours = by.get(g);
      if (contours) contours.push(contour); else by.set(g, [contour]);
    }
    return by;
  });
  const contoursOf = (gs: number[]) => gs.flatMap((g) => groups[g] ?? []);
  const pointsOf = (g: number): Point[][] => (shapes.get(g) ?? []).flatMap((c) => c.paths.map((p) => p.points as Point[]));
  const groupBounds = $derived(boundsOfShapes(shapes));
  const boxOfGroups = (gs: number[], m: Transform | null): Box | null => {
    const box = unionBounds(gs.map(g => groupBounds.get(g)));
    return m ? axisAlignedBounds(box, m) ?? transformedBounds(gs.flatMap(pointsOf), m) : box;
  };

  // The selection, and the transform drawn on it before the server has it.
  let selected = $state<number[]>([]);
  /** One shape of the selected part, tapped again to take it alone: a hole
   *  inside a part, a mark around a hole. Parts still move as a whole. */
  let shape = $state<number | null>(null);
  /** Half a fingertip, in pixels: how far from a line a tap still takes it. */
  const HALO = 20;
  const selectedSet = $derived(new Set(selected));
  $effect(() => { if (shape !== null && !contoursOf(selected).includes(shape)) shape = null; });
  $effect(() => { selectedContours = updating ? [] : shape !== null ? [shape] : contoursOf(selected); });
  let local = $state<{ groups: number[]; m: Transform } | null>(null);
  const movingSet = $derived(new Set(local?.groups ?? []));
  const movingAll = $derived(!!local && movingSet.size === groups.length);
  const boundsIndex = $derived(new BoundsIndex(groupBounds));
  let lastVisible: number[] = [];
  const visibleGroups = $derived.by(() => {
    // Overscan keeps marks and hit strokes visible at the boundary and avoids
    // repeatedly mounting shapes while the pointer moves by a few pixels.
    const padding = view.mmPerPixel * 80;
    const camera = { minX: view.x - zero[0] - padding, maxX: view.x + view.w - zero[0] + padding,
      minY: -view.y - view.h - zero[1] - padding, maxY: -view.y - zero[1] + padding };
    const visible = movingAll ? [] : boundsIndex.query(camera).filter(g => !movingSet.has(g));
    if (local) {
      const original = inverseBounds(camera, local.m);
      visible.push(...(original ? boundsIndex.query(original).filter(g => movingSet.has(g)) : local.groups));
    }
    visible.sort((a, b) => a - b);
    if (visible.length === lastVisible.length && visible.every((g, i) => g === lastVisible[i])) return lastVisible;
    lastVisible = visible;
    return visible;
  });
  // Another part or job starts unselected; the selection survives every
  // other document update.
  const opened = $derived(`${draft?.generation ?? ''}/${draft?.job ?? ''}`);
  $effect(() => { void opened; untrack(() => { selected = []; local = null; }); });
  let known = '';
  $effect.pre(() => {
    // Keep the release position until the replacement geometry is drawn,
    // then remove the local transform before updating the SVG.
    const now = JSON.stringify(scene?.placed ?? []);
    if (now !== known) { known = now; untrack(() => { local = null; }); }
  });
  const selectionBase = $derived(boxOfGroups(selected, null));
  const selection = $derived(local ? axisAlignedBounds(selectionBase, local.m) ?? transformedBounds(selected.flatMap(pointsOf), local.m) : selectionBase);
  const fail = (error: unknown) => ui.say(explain(error), true);
  /** Picking or nesting has the canvas: shapes stay put and taps go there. */
  const canvasTaken = $derived(!!ui.picking || ui.nestShown || ui.nestPicking);
  /** Nothing on the canvas can be moved or edited. */
  const canvasBusy = $derived(updating || canvasTaken);
  /** The selection cannot be edited: there is none, or the canvas is busy. */
  const selectionLocked = $derived(canvasBusy || !selection);
  /** The selection box and its HUD are drawn. */
  const selectionShown = $derived(!!selection && !canvasTaken);
  /** Moving, turning and sizing take whole parts, not one shape of a part. */
  const transformLocked = $derived(selectionLocked || shape !== null);

  // The clipboard snapshots the selection; one add request pastes a whole batch.
  let pending = $state<number[] | null>(null);
  export function selectAll(): void {
    if (!updating && !pasting && !ui.picking) selected = groups.map((_, g) => g);
  }
  /** Selects every shape drawn from the part whose contours are `first`
   *  to `first + count` in the job's drawing. */
  export function selectPart(first: number, count: number): void {
    if (updating || pasting || ui.picking || !draft) return;
    const mine = (i: number) => { const source = draft.placed[i]?.source ?? -1; return source >= first && source < first + count; };
    selected = groups.flatMap((members, g) => (members.some(mine) ? [g] : []));
  }
  function copy(): void {
    if (pasting || selectionLocked || !draft || !selected.length || !selection) return;
    const indices = contoursOf(selected);
    const contours = indices.flatMap((i) => {
      const p = draft.placed[i];
      return p ? [{ source: p.source, transform: [...p.transform] as Transform }] : [];
    });
    const relative = new Map(indices.map((i, n) => [i, n]));
    const grouping = selected.map(g => (groups[g] ?? []).flatMap(i => relative.has(i) ? [relative.get(i)!] : []));
    clipboard = {
      parts: draft.parts.map((p) => p.id), contours, leads: copiedLeadOverrides(draft.features.leads, indices), grouping,
      width: selection.maxX - selection.minX, height: selection.maxY - selection.minY, offset: [0, 0],
    };
    ui.setupPanel = 'clipboard';
  }
  export async function paste(count = 1): Promise<void> {
    if (pasting || canvasBusy || !clipboard || !draft) return;
    if (!pasteable(clipboard, draft)) { ui.say('The copied shapes belong to another job. Copy shapes from this drawing before pasting.', true); return; }
    const copied = clipboard;
    const placed = draft.placed.length;
    await withBusy((b) => (pasting = b), async () => {
      const batch = pasteBatch(copied, { ...pasteSettings, count }, placed);
      const { contours, draft: saved } = await api.add(batch.contours, batch.lead_overrides, batch.grouping);
      copied.offset = batch.offset;
      if (saved?.revision === server.doc?.draft?.revision) pending = contours;
      ui.say(`Pasted ${plural(count, 'copy', 'copies')}.`);
    });
  }

  // The drawing bar's tools; ToolBar lays them out and orders them, as it does the machining bar.
  const hasGroup = () => selected.some(g => (groups[g]?.length ?? 0) > 1);
  const DRAW_TOOLS: Record<string, Tool> = {
    fit: { label: 'Fit', title: 'Frame the parts', when: 'idle', icon: 'ic-fit', disabled: () => false, run: () => fit() },
    'zoom-in': { label: 'Zoom in', when: 'idle', icon: 'ic-plus', disabled: () => false, run: () => view.zoom(1.25) },
    'zoom-out': { label: 'Zoom out', when: 'idle', icon: 'ic-minus', disabled: () => false, run: () => view.zoom(0.8) },
    layer: {
      label: 'Layer', title: 'Move the selection to a layer', when: 'selected', icon: 'ic-layers',
      disabled: () => selectionLocked, run: () => { assigning = shape !== null ? [shape] : contoursOf(selected); },
    },
    snap: {
      label: 'Snap', when: 'idle', text: () => (ui.snap ? 'On' : 'Off'), on: () => ui.snap,
      disabled: () => false, run: () => ui.toggleSnap(),
    },
    grid: { label: 'Grid', when: 'idle', text: () => quantity(ui.grid, 'mm'), disabled: () => false, run: () => gridSize() },
    'mirror-x': {
      label: 'Mirror X', title: 'Mirror horizontally', when: 'selected', icon: 'ic-mirror',
      disabled: () => transformLocked, run: () => act('mirror'),
    },
    'mirror-y': {
      label: 'Mirror Y', title: 'Mirror vertically', when: 'selected', icon: 'ic-mirror vertical',
      disabled: () => transformLocked, run: () => act('vertical'),
    },
    turn: {
      label: '90°', title: 'Turn the selection a quarter turn', when: 'selected', icon: 'ic-rotate',
      disabled: () => transformLocked, run: () => act('turn'),
    },
    scale: { label: 'Scale', when: 'selected', text: () => '%', disabled: () => transformLocked, run: () => resize('scale') },
    center: {
      label: 'Center', title: 'Center on the bed', when: 'selected', icon: 'ic-target',
      disabled: () => transformLocked || !frame.bed, run: () => centerOnBed(),
    },
    reset: {
      label: 'Reset', title: 'Put the selection back where the drawing has it', when: 'selected', icon: 'ic-reset',
      disabled: () => transformLocked, run: () => act('reset'),
    },
    group: {
      label: 'Group', title: 'Group the selected shapes', when: 'selected', absent: () => selected.length < 2,
      disabled: () => transformLocked || selected.length < 2, run: () => groupSelection(true),
    },
    ungroup: {
      label: 'Ungroup', title: 'Ungroup the selected shapes', when: 'selected', absent: () => !hasGroup(),
      disabled: () => selectionLocked || !hasGroup(), run: () => groupSelection(false),
    },
    copy: {
      label: 'Copy', title: 'Copy the selection', when: 'selected', icon: 'ic-copy', on: () => ui.setupPanel === 'clipboard',
      disabled: () => pasting || transformLocked, run: () => copy(),
    },
    paste: {
      label: 'Paste', title: 'Paste one copy', when: 'always', icon: 'ic-paste', absent: () => !clipboard,
      disabled: () => pasting || canvasBusy || !pasteable(clipboard, draft), run: () => { void paste(); },
    },
    delete: {
      label: 'Delete', title: 'Take the selection off the sheet', when: 'selected', icon: 'ic-trash',
      disabled: () => selectionLocked, run: () => remove(),
    },
    deselect: {
      label: 'Deselect', title: 'Clear the selection', when: 'selected', icon: 'ic-x',
      disabled: () => !selectionShown, run: () => { selected = []; },
    },
    undo: { label: 'Undo', title: 'Undo the last edit', when: 'always', icon: 'ic-undo', disabled: () => !draft?.past, run: () => history(true) },
    redo: { label: 'Redo', title: 'Do the edit undone again', when: 'always', icon: 'ic-redo', disabled: () => !draft?.future, run: () => history(false) },
  };
  function groupSelection(together: boolean): void {
    if (selectionLocked) return;
    api.group(contoursOf(selected), together).catch(fail);
  }
  function remove(): void {
    if (updating || !selected.length) return;
    api.remove(shape !== null ? [shape] : contoursOf(selected)).then(() => { selected = []; }).catch(fail);
  }
  const history = (back: boolean) => (back ? api.undo() : api.redo()).catch(fail);
  let groupKeys: string[][] = [];
  function selectPending(): void {
    if (!pending) return;
    const added = new Set(pending);
    const taken = groups.flatMap((members, g) => members.some((i) => added.has(i)) ? [g] : []);
    if (taken.length) { selected = taken; pending = null; }
  }
  $effect(() => {
    void pending;
    const keys = groups.map((members) => members.flatMap((i) => {
      const placed = scene?.placed[i];
      return placed ? [`${placed.source}:${placed.copy}`] : [];
    }));
    untrack(() => {
      const chosen = new Set(selected.flatMap(g => groupKeys[g] ?? []));
      selected = keys.flatMap((members, g) => members.some(key => chosen.has(key)) ? [g] : []);
      groupKeys = keys;
      selectPending();
    });
  });

  function commit(gs: number[], m: Transform | null, revision = draft?.revision): void {
    if (updating) return;
    local = m ? { groups: gs, m } : null;
    api.transform(contoursOf(gs), m, revision).catch((error: unknown) => { fail(error); local = null; });
  }
  function resize(axis: 0 | 1 | 'scale'): void {
    if (updating || !selection) return;
    const box = selection;
    const length = axis === 0 ? box.maxX - box.minX : box.maxY - box.minY;
    const chosen = [...selected], revision = draft?.revision;
    const label = axis === 'scale' ? 'Scale selection' : axis === 0 ? 'Selection width (height linked)' : 'Selection height (width linked)';
    const scaled = axis === 'scale';
    osk.number(label, scaled ? 100 : length, scaled ? '%' : 'mm', v => {
      const scale = scaled ? v / 100 : v / length;
      if (!(scale > 0 && Number.isFinite(scale))) { ui.say('Use a positive size.', true); return; }
      commit(chosen, scaling(scale, centre(box)), revision);
    });
  }
  function centerOnBed(): void {
    if (!selection || !frame.bed) return;
    const at = centre(selection), target = centre(frame.bed);
    commit(selected, translate(target[0] - zero[0] - at[0], target[1] - zero[1] - at[1]));
  }
  function gridSize(): void {
    osk.number('Grid spacing', ui.grid, 'mm', v => {
      if (v > 0 && v <= 1000) ui.setGrid(v);
      else ui.say(`Grid spacing must be above 0 and at most ${quantity(1000, 'mm')}.`, true);
    });
  }

  function act(what: 'mirror' | 'vertical' | 'turn' | 'reset'): void {
    if (!selected.length || !selection) return;
    if (what === 'reset') { commit(selected, null); return; }
    const at = centre(selection);
    commit(selected, what === 'mirror' ? mirror(at) : what === 'vertical' ? mirrorVertical(at) : rotation(90, at));
  }
  function typed(axis: 0 | 1): void {
    if (updating || !selection) return;
    const current = (axis ? selection.minY : selection.minX) + zero[axis];
    const chosen = [...selected];
    const revision = draft?.revision;
    const label = axis ? 'Selection Y, machine' : 'Selection X, machine';
    osk.number(label, current, 'mm', (v) => commit(chosen, translate(axis ? 0 : v - current, axis ? v - current : 0), revision));
  }
  function turnBy(): void {
    if (updating || !selection) return;
    const at = centre(selection);
    const chosen = [...selected];
    const revision = draft?.revision;
    osk.number('Rotate by', 0, '°', (v) => { if (v) commit(chosen, rotation(v, at), revision); });
  }

  // Gestures: a shape moves, the handle turns the selection about its
  // centre; sizes change only through typed values, never by a drag. While picking, the shapes stay put and taps go to the feature.
  const grab = (target: Element): string | null => {
    if (canvasBusy) return null;
    const lead = target.closest<SVGGElement>('[data-lead]')?.dataset['lead'];
    if (lead !== undefined) return `lead:${lead}`;
    if (target.closest('.handle')) return 'rotate';
    // Anywhere inside the selection's box moves it.
    if (target.closest('[data-selection]') && selected.length) return `move:${selected[0]}`;
    const g = target.closest<SVGGElement>('[data-group]')?.dataset['group'];
    return g === undefined ? null : `move:${g}`;
  };
  let anchor: { revision: number; groups: number[]; center: Point; angle: number; box: Box } | null = null;
  let guides = $state<Array<[number, number, number, number]>>([]);
  let dragging = $state(false);

  function ondrag(kind: string, at: Point, towards: Point): void {
    if (updating) return;
    const from = toDrawing(at);
    const to = toDrawing(towards);
    if (kind.startsWith('lead:')) {
      if (leadDrag) { leadDrag.point = to; return; }
      const handle = leadHandles[Number(kind.slice(5))];
      if (handle && draft) {
        const targets = handle.matching !== null
          ? leadHandles.filter(other => other.which === handle.which && other.matching === handle.matching).map(other => other.target)
          : [handle.target];
        leadDrag = { handle, targets, point: to, revision: draft.revision };
      }
      return;
    }
    if (!anchor) {
      // A drag on a shape outside the selection takes that shape alone.
      if (kind.startsWith('move:')) { const g = Number(kind.slice(5)); if (!selected.includes(g)) selected = [g]; }
      const box = boxOfGroups(selected, null);
      if (!box) return;
      const center = centre(box);
      if (!draft) return;
      anchor = { revision: draft.revision, groups: [...selected], center, angle: Math.atan2(from[1] - center[1], from[0] - center[0]), box };
      dragging = true;
    }
    if (kind === 'rotate') {
      let delta = ((Math.atan2(to[1] - anchor.center[1], to[0] - anchor.center[0]) - anchor.angle) * 180) / Math.PI;
      const snapped = Math.round(delta / 15) * 15;
      if (ui.snap && Math.abs(delta - snapped) < 4) delta = snapped;
      local = { groups: anchor.groups, m: rotation(tenth(delta), anchor.center) };
      return;
    }
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    guides = [];
    const { box } = anchor;
    if (ui.snap) {
      dx = Math.round((box.minX + zero[0] + dx) / ui.grid) * ui.grid - box.minX - zero[0];
      dy = Math.round((box.minY + zero[1] + dy) / ui.grid) * ui.grid - box.minY - zero[1];
    }
    const bed = frame.bed;
    if (bed && ui.snap) {
      // Edges snap to the bed's within a fingertip; the box is in drawing
      // coordinates and the bed in the machine's.
      const tolerance = 8 * view.mmPerPixel;
      const edges: Array<[number, number, boolean]> = [
        [box.minX + zero[0] + dx, bed.minX, true], [box.maxX + zero[0] + dx, bed.maxX, true],
        [box.minY + zero[1] + dy, bed.minY, false], [box.maxY + zero[1] + dy, bed.maxY, false],
      ];
      for (const [value, target, vertical] of edges) {
        if (Math.abs(value - target) < tolerance) {
          if (vertical) dx += target - value; else dy += target - value;
          guides.push(vertical ? [target, bed.minY, target, bed.maxY] : [bed.minX, target, bed.maxX, target]);
        }
      }
    }
    local = { groups: anchor.groups, m: translate(dx, dy) };
  }

  function ondragend(_kind: string, cancelled: boolean): void {
    if (leadDrag) {
      const { handle, targets, point, revision } = leadDrag;
      leadDrag = null;
      if (cancelled || !draft || draft.revision !== revision) return;
      const f = structuredClone($state.snapshot(draft.features));
      if (!f.leads) return;
      setLeadOverrides(f.leads, targets, handle.which, draggedLead(handle, point));
      api.setFeatures(f, revision).catch(fail);
      return;
    }
    const done = local;
    const revision = anchor?.revision;
    anchor = null;
    guides = [];
    dragging = false;
    if (cancelled || updating || !draft || revision !== draft.revision) { local = null; return; }
    if (done) commit(done.groups, done.m, revision);
  }

  function ontap(where: Point, target: Element, additive: boolean): void {
    if (updating) return;
    const at = toDrawing(where);
    if (ui.nestPicking && draft) {
      const revision = draft.revision;
      api.pick(at, 10 * view.mmPerPixel, false, revision).then(async ({ pick }) => {
        if (!ui.nestPicking || !pick || server.doc?.draft?.revision !== revision) return;
        await api.setStock({ kind: 'outline', contour: pick.owner }, revision);
        ui.nestPicking = false; selected = []; ui.say('Outline set as the sheet · excluded from cutting');
      }).catch(fail);
      return;
    }
    if (ui.nestShown) return;
    if (ui.picking) { pickAt(at); return; }
    // Each line has a halo a fingertip wide; where halos overlap the nearest
    // line wins, so a hole inside a ring can be tapped. Inside a
    // fingertip-sized hole takes the hole; other empty space, inside a
    // large outline or not, takes nothing. A tap again on a selected part takes the
    // one shape under the finger.
    const shown = (c: PreviewContour) => !hidden(c.layer);
    const hit = nearestShape(shapes, boundsIndex, at, HALO * view.mmPerPixel, shown)
      ?? smallShapeAround(shapes, boundsIndex, at, 5 * HALO * view.mmPerPixel, shown);
    if (!hit) { if (!additive) { selected = []; shape = null; } return; }
    const n = hit.group;
    const one = hit.contour.sources.length === 1 ? hit.contour.sources[0]! : null;
    if (!additive && one !== null && selected.length === 1 && selected[0] === n && (groups[n]?.length ?? 0) > 1) {
      shape = shape === one ? null : one;
      return;
    }
    shape = null;
    selected = additive ? (selected.includes(n) ? selected.filter((x) => x !== n) : [...selected, n]) : [n];
  }

  function onmarquee(swept: Box, additive: boolean): void {
    if (canvasBusy) return;
    const box = { minX: swept.minX - zero[0], maxX: swept.maxX - zero[0], minY: swept.minY - zero[1], maxY: swept.maxY - zero[1] };
    const inside = marqueeGroups(shapes, boundsIndex, box);
    selected = additive ? [...new Set([...selected, ...inside])] : inside;
  }

  // Escape clears, ⌘A takes everything, the arrows nudge by a millimetre;
  // ⌘C, ⌘V, ⌫, ⌘Z and ⇧⌘Z do what they do everywhere.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest('input, textarea, [contenteditable]');
      if (typing || canvasTaken || ui.modal || osk.open) return;
      if (e.key === 'Escape') { selected = []; return; }
      const meta = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      if (meta && key === 'z') { e.preventDefault(); history(!e.shiftKey); return; }
      if (meta && key === 'y') { e.preventDefault(); history(false); return; }
      if (updating) return;
      if (meta && key === 'a') { e.preventDefault(); selectAll(); return; }
      if (meta && key === 'g') { e.preventDefault(); groupSelection(!e.shiftKey); return; }
      if (meta && key === 'c') { e.preventDefault(); copy(); return; }
      if (meta && key === 'v') { e.preventDefault(); paste(); return; }
      if ((e.key === 'Delete' || e.key === 'Backspace') && selected.length) { e.preventDefault(); remove(); return; }
      const step: Record<string, Point> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] };
      const d = step[e.key];
      if (d && selected.length) { e.preventDefault(); const k = e.shiftKey ? 10 : 1; commit(selected, translate(d[0] * k, d[1] * k)); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // Picking: a tap snaps to a contour and the feature takes the spot.
  const spotsOf = (placement: object): Spot[] => ('manual' in placement ? (placement as { manual: Spot[] }).manual : []);
  const candidates = $derived(orderGroups(preview?.contours ?? [], ui.hiddenDrawingLayers));
  const pickedCount = $derived(candidates.filter((group) => group.some((source) => ui.picking?.order.includes(source))).length);
  const latestPick = new LatestPick();

  let pickBusy = $state(false);
  async function pickAt(at: Point): Promise<void> {
    let picking = ui.picking;
    if (updating || !picking || !draft || pickBusy) return;
    if (featureEdits.busy) { ui.say('Wait for the feature edit to finish.'); return; }
    if (picking.revision !== draft.revision) {
      ui.say('The drawing changed. Start the picking session again.', true); ui.picking = null; return;
    }
    const ticket = latestPick.begin(draft.revision);
    pickBusy = true;
    try {
      const { pick } = await api.pick(at, 10 * view.mmPerPixel, picking.feature === 'bridges', ticket.revision, picking.features);
      if (!latestPick.accepts(ticket, server.doc?.draft?.revision) || ui.picking !== picking) return;
      if (!pick) { ui.say('No contour there.'); return; }
      const f = structuredClone($state.snapshot(picking.features)) as Features;
      switch (picking.feature) {
        case 'joints': if (f.joints) f.joints.placement = { manual: [...spotsOf(f.joints.placement), pick.spot] }; break;
        case 'cooling': if (f.cooling) f.cooling.placement = { manual: [...spotsOf(f.cooling.placement), pick.spot] }; break;
        case 'start': f.start.spots = [...f.start.spots.filter((s) => s.contour !== pick.spot.contour), pick.spot]; break;
        case 'bridges': {
          const end = { contour: pick.spot.contour, point: { x: pick.point[0], y: pick.point[1] } };
          if (!picking.first) { ui.picking = { ...picking, first: end, firstOwner: pick.owner }; return; }
          f.bridges?.connections.push({ first: picking.first, second: end });
          ui.picking = { ...picking, first: null };
          break;
        }
        case 'order': {
          if (picking.order.includes(pick.spot.contour)) return;
          const group = candidates.find((group) => group.includes(pick.spot.contour));
          if (!group) return;
          const order = [...new Set([...picking.order, ...group])];
          ui.picking = { ...picking, order };
          const all = [...new Set(preview?.contours.flatMap((c) => c.sources) ?? [])];
          f.order.strategy = { manual: [...order, ...all.filter((source) => !order.includes(source))] };
          break;
        }
      }
      const point = draft.placed[pick.owner] ? apply(draft.placed[pick.owner]!.transform, pick.point) : pick.point;
      const staged = { ...(ui.picking ?? picking), features: f, marks: [...picking.marks, point] };
      ui.picking = staged;
      const reply = await api.previewFeatures(f, ticket.revision);
      if (ui.picking === staged && server.doc?.draft?.revision === ticket.revision) ui.picking = { ...staged, preview: reply.preview };
    } catch (error) {
      ui.say(explain(error), true);
    } finally { pickBusy = false; }
  }
  async function finishPicks(): Promise<void> {
    const picking = ui.picking;
    if (updating || !picking || pickBusy) return;
    if (picking.first) { ui.say('Complete or cancel the pending bridge end first.', true); return; }
    await withBusy((b) => (pickBusy = b), async () => {
      await api.setFeatures(picking.features, picking.revision);
      if (ui.picking === picking) ui.picking = null;
    });
  }

  /** The first end of a bridge being placed, where its contour now lies. */
  const firstEnd = $derived.by((): Point | null => {
    const first = ui.picking?.first;
    const picking = ui.picking;
    if (!first || !draft || picking?.revision !== draft.revision || picking.firstOwner === undefined) return null;
    const m = draft.placed[picking.firstOwner]?.transform;
    const p: Point = [first.point.x, first.point.y];
    return m ? apply(m, p) : null;
  });

  /** The cutting rank of each drawing contour, while an order is picked or once one is set. */
  const ranks = $derived.by(() => {
    const strategy = scene?.features.order.strategy;
    const list = !updating && ui.picking?.feature === 'order' ? ui.picking.order : typeof strategy === 'object' ? strategy.manual : [];
    return new Map(list.map((contour, i) => [contour, i + 1]));
  });

  const leadHandles = $derived.by((): LeadHandle[] => {
    if (updating || featureEdits.busy || ui.setupPanel !== 'leads' || ui.picking || local) return [];
    return leadHandlesOf(preview?.contours ?? [], new Set(contoursOf(selected)), draft?.features.leads ?? null);
  });
  let leadDrag = $state<{ handle: LeadHandle; targets: Spot[]; point: Point; revision: number } | null>(null);
  const orderPaths = $derived(orderSegments(preview));
  const playback = $derived(ui.setupPanel === 'order' ? orderPosition(orderPaths, orderProgress) : null);

  // The drawing's layers: shown or hidden here, cut or skipped in the job.
  /** Shapes on their way to a layer, while their layer is chosen. */
  let assigning = $state<number[] | null>(null);
  const hidden = (layer: string) => ui.hiddenDrawingLayers.includes(layer);

  /** A mark size in millimetres that keeps its screen size. */
  const mark = $derived(Math.min(view.w, view.h) / 120);
  // Non-scaling round strokes keep the same dot size without rewriting a
  // radius on every contour during zoom. Recompute only with canvas size.
  const markPixels = $derived(Math.min(view.pixelWidth, Math.round(view.h / view.mmPerPixel)) / 120);
  $effect(() => { void ui.selectionEpoch; untrack(() => { selected = []; local = null; }); });
</script>

<div class="canvas-wrap">

  <Stage {view} bed={frame.bed} head={frame.head} origin={ui.nestShown ? null : frame.origin} {grab} {ondrag} {ondragend} {ontap} {onmarquee}>
    <g transform="translate({zero[0]} {zero[1]})">
    {#if stockOutline.length}
      <path d={pathOf(stockOutline, false) + 'Z'} fill="var(--accent)" fill-opacity="0.035" stroke="var(--accent)"
        stroke-dasharray="8 5" stroke-width="1.5" vector-effect="non-scaling-stroke" pointer-events="none" />
    {/if}
    {#each stockCutouts as cutout}
      <path d={pathOf(cutout, false) + 'Z'} fill="var(--ink-3)" fill-opacity="0.16" stroke="var(--ink-3)"
        stroke-dasharray="3 3" stroke-width="1" vector-effect="non-scaling-stroke" pointer-events="none" />
    {/each}
    {#if ui.nestLive && !ui.nestPreview}
      <!-- A running search: each copy is a group of the drawing, moved. -->
      {#each ui.nestLive.copies as copy}
        <g transform={svgMatrix(copy.transform)}>
          {#each shapes.get(copy.group) ?? [] as contour}
            {#each contour.paths as path}
              {#if path.kind === 'cut'}
                <path class="path cut live" d={pathOf(path.points, false)} vector-effect="non-scaling-stroke" pointer-events="none" />
              {/if}
            {/each}
          {/each}
        </g>
      {/each}
    {/if}
    {#if ui.nestPreview}
      {#each ui.nestPreview.contours as contour}
        {#each contour.paths as path}<path class="path {path.kind}" d={pathOf(path.points, false)} vector-effect="non-scaling-stroke" pointer-events="none" />{/each}
      {/each}
    {/if}
    {#if preview && !ui.nestShown}
      <g transform={movingAll && local ? svgMatrix(local.m) : ''}>
      {#each visibleGroups as g (g)}
        {@const on = selectedSet.has(g)}
        {@const bounds = groupBounds.get(g)}
        {@const details = on || !!bounds && Math.max(bounds.maxX - bounds.minX, bounds.maxY - bounds.minY) >= 24 * view.mmPerPixel}
        <g class="shape" class:selected={on} class:drilled={on && shape !== null} class:dragging={dragging && on} data-group={g}
          transform={!movingAll && local && movingSet.has(g) ? svgMatrix(local.m) : ''}>
          {#each shapes.get(g) ?? [] as contour}
            {#if !hidden(contour.layer)}
              {@const picked = shape !== null && contour.sources[0] === shape}
              <g class="contour" class:picked>
              <!-- The selection's halo: the whole part softly, one shape taken alone strongly. -->
              {#if on && (shape === null || picked)}<path class="halo" class:strong={picked} d={hitPath(contour)} vector-effect="non-scaling-stroke"/>{/if}
              <path class="hit" d={hitPath(contour)} vector-effect="non-scaling-stroke"/>
              {#each contour.paths as path}
                {#if ui.layerShown(path.kind)}<path class="path {path.kind}" class:mark={marked.has(contour.layer)}
                  style:stroke={path.kind === 'cut' ? colors.get(contour.layer) : null} d={drawingPath(path.points)} vector-effect="non-scaling-stroke"/>{/if}
              {/each}
              {#if details && (ui.setupPanel !== 'nest' || on)}
                {#if ui.layerShown('cooling')}
                  {#each contour.cooling as [x, y]}
                    <path class="mark cooling" d="M{x} {y}h0" stroke="#d04c79" stroke-width={markPixels * 2}
                      stroke-linecap="round" vector-effect="non-scaling-stroke"/>
                  {/each}
                {/if}
                <path class="mark start" d="M{contour.start[0]} {contour.start[1]}h0" stroke="var(--accent)"
                  stroke-width={Math.min(markPixels * 1.4, 4)} stroke-linecap="round" vector-effect="non-scaling-stroke"/>
              {/if}
              {#if contour.sources.length === 1 && (ranks.has(contour.sources[0]!) || ui.setupPanel === 'order')}
                <text class="order-label" transform="translate({contour.start[0] + 1.5 * mark} {contour.start[1] + 1.5 * mark}) scale(1 -1)"
                  font-size={mark * 3}>{ranks.get(contour.sources[0]!) ?? contour.sources[0]! + 1}</text>
              {/if}
              </g>
            {/if}
          {/each}
        </g>
      {/each}
      </g>
      {#each ui.picking?.marks ?? [] as [x, y]}<circle class="mark bridge" cx={x} cy={y} r={mark} />{/each}
      {#if playback}
        <circle cx={playback.point[0]} cy={playback.point[1]} r={mark * 1.7} fill="var(--accent)" stroke="var(--bg)" vector-effect="non-scaling-stroke" />
      {/if}
      {#each leadHandles as handle, i}
        <g data-lead={i} style="cursor:crosshair">
          <circle cx={handle.end[0]} cy={handle.end[1]} r={mark * 2} fill="var(--accent)" stroke="var(--bg)" vector-effect="non-scaling-stroke" />
        </g>
      {/each}
      {#if leadDrag}
        <path d="M{leadDrag.handle.anchor.join(' ')}L{leadDrag.point.join(' ')}" stroke="var(--accent)" stroke-width="2" vector-effect="non-scaling-stroke" />
      {/if}
      {#if firstEnd}<circle class="mark bridge" cx={firstEnd[0]} cy={firstEnd[1]} r={mark * 1.5} vector-effect="non-scaling-stroke"/>{/if}
      {#if selection && selectionShown && shape === null}
        <g class="gizmo">
          <rect class="sel-grab" data-selection x={selection.minX - 2 * mark} y={selection.minY - 2 * mark}
            width={selection.maxX - selection.minX + 4 * mark} height={selection.maxY - selection.minY + 4 * mark}/>
          <rect class="sel-box" x={selection.minX - 2 * mark} y={selection.minY - 2 * mark}
            width={selection.maxX - selection.minX + 4 * mark} height={selection.maxY - selection.minY + 4 * mark} vector-effect="non-scaling-stroke"/>
          <path class="stalk" d="M{centre(selection)[0]} {selection.maxY + 2 * mark}V{selection.maxY + 8 * mark}" vector-effect="non-scaling-stroke"/>
          <g class="handle" transform="translate({centre(selection)[0]} {selection.maxY + 8 * mark})">
            <circle r={3 * mark} class="hit"/><circle r={1.4 * mark} vector-effect="non-scaling-stroke"/>
          </g>
        </g>
      {/if}
    {/if}
    </g>
    <g class="guide">{#each guides as [x1, y1, x2, y2]}<line {x1} {y1} {x2} {y2} vector-effect="non-scaling-stroke"/>{/each}</g>
    {#snippet overlay()}
      {#if ui.picking}
        <PickBar picking={ui.picking} picked={pickedCount} total={candidates.length} busy={updating || pickBusy} onfinish={finishPicks} />
      {/if}
      {#if shape !== null && selectionShown}<div class="hint">One shape · tap it again for the whole part</div>
      {:else if preview?.warnings.length}<div class="hint">{preview.warnings[0]}</div>{/if}
      {#if selection && selectionShown && shape === null}
        <SelectionHud {selection} {zero} disabled={updating} onplace={typed} onturn={turnBy} onresize={resize} />
      {/if}
    {/snippet}
  </Stage>
</div>
<ToolBar class="drawing-toolbar" label="Drawing tools" tools={DRAW_TOOLS} order={ui.drawBar} save={(order) => ui.setDrawBar(order)}
  bind:editing={ui.editDrawBar} selected={!!selection} />
{#if assigning}<LayerAssign contours={assigning} {layers} onclose={() => (assigning = null)} />{/if}
