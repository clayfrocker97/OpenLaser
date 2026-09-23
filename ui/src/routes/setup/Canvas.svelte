<script lang="ts">
  import { distance, quantity } from '../../lib/units.svelte';
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
  import { Viewport } from '../../lib/viewport.svelte';
  import { api } from '../../api/client';
  import { featureEdits } from '../../stores/feature-edits';
  import { LatestPick, orderGroups } from '../../lib/picking';
  import { orderPosition, orderSegments } from '../../lib/cut-order';
  import { copiedLeadOverrides, leadLookup, setLeadOverrides } from '../../lib/lead-overrides';
  import { pasteBatch, pasteable, type CopiedShapes, type PasteSettings } from '../../lib/copy-paste';
  import { layersOf, partsOf } from '../../lib/job-parts';
  import { server } from '../../stores/server.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { osk } from '../../lib/osk.svelte';
  import { frameOf } from '../../lib/frame';
  import { explain, fmt } from '../../lib/format';
  import { boxOfBounds, pathOf, type Box } from '../../lib/svg';
  import { drawingPath, hitPath, boundsOfShapes, marqueeGroups, unionBounds, axisAlignedBounds, transformedBounds, BoundsIndex, inverseBounds } from '../../lib/viewer-geometry';
  import type { Features, Lead, PreviewContour, Spot, Transform } from '../../api';

  let { rail, selectedContours = $bindable([]), orderProgress = 0, clipboard = $bindable(null), pasting = $bindable(false), pasteSettings }: {
    rail: Snippet; selectedContours?: number[]; orderProgress?: number; clipboard?: CopiedShapes | null; pasting?: boolean; pasteSettings: PasteSettings;
  } = $props();

  type Point = [number, number];
  const doc = $derived(server.doc!);
  const draft = $derived(doc.draft);
  const scene = $derived(server.canvasDraft);
  const updating = $derived(draft?.error === 'preparing geometry');
  const layers = $derived(layersOf(partsOf(draft, doc.library.parts)));
  const preview = $derived(!updating && ui.picking?.revision === draft?.revision ? (ui.picking?.preview ?? scene?.preview ?? null) : scene?.preview ?? null);
  const groups = $derived(scene?.groups ?? []);
  const stockOutline = $derived(ui.nestPreview ? ui.nestStock?.outline ?? scene?.stock_outline ?? [] : ui.nestLive?.stock_outline ?? scene?.stock_outline ?? []);
  const stockCutouts = $derived(ui.nestPreview ? ui.nestStock?.cutouts ?? [] : ui.nestLive?.stock_cutouts ?? scene?.stock_cutouts ?? []);
  const frame = $derived(frameOf({ ...doc, draft: scene }));

  /** What the machine adds to a drawing coordinate: the canvas is in machine coordinates. */
  const zero = $derived(frame.zero);
  const toDrawing = (p: Point): Point => [p[0] - zero[0], p[1] - zero[1]];

  const view = new Viewport();
  // Fit frames the parts; the bed is only the fallback with nothing drawn.
  const fit = () => {
    view.setLimit(frame.bed);
    if (preview?.bounds) { const b = boxOfBounds(preview.bounds); view.fit({ minX: b.minX + zero[0], maxX: b.maxX + zero[0], minY: b.minY + zero[1], maxY: b.maxY + zero[1] }); }
    else if (frame.bed) view.home();
  };
  let fitted = '';
  $effect(() => {
    const key = `${draft?.generation ?? ''}/${draft?.job ?? ''}/${frame.bed ? 'bed' : ''}/${preview?.bounds ? 'parts' : ''}`;
    if (key !== fitted) { fitted = key; untrack(fit); }
  });
  $effect(() => () => { ui.picking = null; });

  // Uniform similarity transforms, as the server keeps them: [a b c d e f].
  const after = (a: Transform, b: Transform): Transform => [a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1], a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3], a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5]];
  const translate = (dx: number, dy: number): Transform => [1, 0, 0, 1, dx, dy];
  const about = (m: Transform, c: Point): Transform => after(translate(c[0], c[1]), after(m, translate(-c[0], -c[1])));
  const rotation = (degrees: number, c: Point): Transform => { const r = (degrees * Math.PI) / 180; return about([Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0], c); };
  const mirror = (c: Point): Transform => about([-1, 0, 0, 1, 0, 0], c);
  const apply = (m: Transform, p: Point): Point => [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]];
  const svgMatrix = (m: Transform) => `matrix(${m.join(' ')})`;
  const tenth = (v: number) => Math.round(v * 10) / 10;
  const centre = (b: Box): Point => [(b.minX + b.maxX) / 2, (b.minY + b.maxY) / 2];

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
  const selectedSet = $derived(new Set(selected));
  $effect(() => { selectedContours = updating ? [] : contoursOf(selected); });
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
    const contours = indices.flatMap((i) => { const p = draft.placed[i]; return p ? [{ source: p.source, transform: [...p.transform] as Transform }] : []; });
    const relative = new Map(indices.map((i, n) => [i, n]));
    const grouping = selected.map(g => (groups[g] ?? []).flatMap(i => relative.has(i) ? [relative.get(i)!] : []));
    clipboard = { parts: draft.parts.map((p) => p.id), contours, leads: copiedLeadOverrides(draft.features.leads, indices), grouping, width: selection.maxX - selection.minX, height: selection.maxY - selection.minY, offset: [0, 0] };
    ui.setupPanel = 'clipboard';
  }
  export async function paste(count = 1): Promise<void> {
    if (pasting || updating || !clipboard || ui.picking || ui.nestShown || ui.nestPicking || !draft) return;
    if (!pasteable(clipboard, draft)) { ui.say('The copied shapes belong to another job. Copy shapes from this drawing before pasting.', true); return; }
    pasting = true;
    try {
      const copied = clipboard;
      const batch = pasteBatch(copied, { ...pasteSettings, count }, draft.placed.length);
      const { contours, draft: saved } = await api.add(batch.contours, batch.lead_overrides, batch.grouping);
      copied.offset = batch.offset;
      if (saved?.revision === server.doc?.draft?.revision) pending = contours;
      ui.say(`Pasted ${count} ${count === 1 ? 'copy' : 'copies'}.`);
    } catch (error) { fail(error); } finally { pasting = false; }
  }
  const selectionLocked = $derived(updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking);
  function groupSelection(together: boolean): void {
    if (selectionLocked) return;
    api.group(contoursOf(selected), together).catch(fail);
  }
  function remove(): void {
    if (updating || !selected.length) return;
    api.remove(contoursOf(selected)).then(() => { selected = []; }).catch(fail);
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
    osk.number(axis === 'scale' ? 'Scale selection' : axis === 0 ? 'Selection width (height linked)' : 'Selection height (width linked)', axis === 'scale' ? 100 : length, axis === 'scale' ? '%' : 'mm', v => {
      const scale = axis === 'scale' ? v / 100 : v / length;
      if (!(scale > 0 && Number.isFinite(scale))) { ui.say('Use a positive size.', true); return; }
      commit(chosen, about([scale, 0, 0, scale, 0, 0], centre(box)), revision);
    });
  }
  function centerOnBed(): void {
    if (!selection || !frame.bed) return;
    const at = centre(selection), target = centre(frame.bed);
    commit(selected, translate(target[0] - zero[0] - at[0], target[1] - zero[1] - at[1]));
  }
  function gridSize(): void {
    osk.number('Grid spacing', ui.grid, 'mm', v => { if (v > 0 && v <= 1000) ui.setGrid(v); else ui.say(`Grid spacing must be above 0 and at most ${quantity(1000, 'mm')}.`, true); });
  }

  function act(what: 'mirror' | 'vertical' | 'turn' | 'reset'): void {
    if (!selected.length || !selection) return;
    if (what === 'reset') { commit(selected, null); return; }
    commit(selected, what === 'mirror' ? mirror(centre(selection)) : what === 'vertical' ? about([1, 0, 0, -1, 0, 0], centre(selection)) : rotation(90, centre(selection)));
  }
  function typed(axis: 0 | 1): void {
    if (updating || !selection) return;
    const current = (axis ? selection.minY : selection.minX) + zero[axis];
    const chosen = [...selected];
    const revision = draft?.revision;
    osk.number(axis ? 'Selection Y, machine' : 'Selection X, machine', current, 'mm', (v) => commit(chosen, translate(axis ? 0 : v - current, axis ? v - current : 0), revision));
  }
  function turnBy(): void {
    if (updating || !selection) return;
    const at = centre(selection);
    const chosen = [...selected];
    const revision = draft?.revision;
    osk.number('Rotate by', 0, '°', (v) => { if (v) commit(chosen, rotation(v, at), revision); });
  }

  // Gestures: a shape moves, the handle turns the selection about its
  // centre. While picking, the shapes stay put and taps go to the feature.
  const grab = (target: Element): string | null => {
    if (updating || ui.picking || ui.nestPicking || ui.nestShown) return null;
    const lead = target.closest<SVGGElement>('[data-lead]')?.dataset['lead'];
    if (lead !== undefined) return `lead:${lead}`;
    if (target.closest('.resize-handle')) return 'resize';
    if (target.closest('.handle')) return 'rotate';
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
    if (kind === 'resize') {
      const radius = Math.hypot(from[0] - anchor.center[0], from[1] - anchor.center[1]);
      const scale = Math.max(0.001, Math.hypot(to[0] - anchor.center[0], to[1] - anchor.center[1]) / Math.max(radius, 1e-6));
      local = { groups: anchor.groups, m: about([scale, 0, 0, scale, 0, 0], anchor.center) };
      return;
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
      const edges: Array<[number, number, boolean]> = [[box.minX + zero[0] + dx, bed.minX, true], [box.maxX + zero[0] + dx, bed.maxX, true], [box.minY + zero[1] + dy, bed.minY, false], [box.maxY + zero[1] + dy, bed.maxY, false]];
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
      const [x, y] = handle.anchor;
      const turn = Math.atan2(point[1] - y, point[0] - x) - Math.atan2(handle.end[1] - y, handle.end[0] - x);
      const f = structuredClone($state.snapshot(draft.features));
      if (!f.leads) return;
      const lead = { ...handle.definition, length: Math.hypot(point[0] - x, point[1] - y),
        angle: handle.definition.angle + turn * 180 / Math.PI * (handle.which === 'entry' ? -handle.side : handle.side) };
      setLeadOverrides(f.leads, targets, handle.which, lead);
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
        ui.nestPicking = false; selected = []; ui.say('Outline set as stock · excluded from cutting');
      }).catch(fail);
      return;
    }
    if (ui.nestShown) return;
    if (ui.picking) { pickAt(at); return; }
    const g = target.closest<SVGGElement>('[data-group]')?.dataset['group'];
    if (g === undefined) { if (!additive) selected = []; return; }
    const n = Number(g);
    selected = additive ? (selected.includes(n) ? selected.filter((x) => x !== n) : [...selected, n]) : [n];
  }

  function onmarquee(swept: Box, additive: boolean): void {
    if (updating || ui.picking || ui.nestPicking || ui.nestShown) return;
    const box = { minX: swept.minX - zero[0], maxX: swept.maxX - zero[0], minY: swept.minY - zero[1], maxY: swept.maxY - zero[1] };
    const inside = marqueeGroups(shapes, boundsIndex, box);
    selected = additive ? [...new Set([...selected, ...inside])] : inside;
  }

  // Escape clears, ⌘A takes everything, the arrows nudge by a millimetre;
  // ⌘C, ⌘V, ⌫, ⌘Z and ⇧⌘Z do what they do everywhere.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest('input, textarea, [contenteditable]') || ui.picking || ui.nestPicking || ui.nestShown || ui.modal || osk.open) return;
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
  const hint = $derived.by(() => {
    const p = ui.picking;
    if (!p) return '';
    switch (p.feature) {
      case 'joints': return 'Tap a contour where a joint should hold it';
      case 'cooling': return 'Tap a contour where the cut should pause to cool';
      case 'start': return 'Tap a contour where its cut should start';
      case 'bridges': return p.first ? 'Tap the other end of the bridge' : 'Tap the first end of the bridge';
      case 'order': return `Tap the contours in cutting order · ${pickedCount} of ${candidates.length}`;
    }
  });

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
    pickBusy = true;
    try { await api.setFeatures(picking.features, picking.revision); if (ui.picking === picking) ui.picking = null; } catch (error) { fail(error); } finally { pickBusy = false; }
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
    const list = !updating && ui.picking?.feature === 'order' ? ui.picking.order : typeof scene?.features.order.strategy === 'object' ? scene.features.order.strategy.manual : [];
    return new Map(list.map((contour, i) => [contour, i + 1]));
  });

  type LeadHandle = { matching: number | null; which: 'entry' | 'exit'; target: Spot; definition: Lead; anchor: Point; end: Point; side: number };
  const leadHandles = $derived.by((): LeadHandle[] => {
    if (updating || featureEdits.busy || ui.setupPanel !== 'leads' || ui.picking || local) return [];
    const chosen = new Set(contoursOf(selected));
    const definitionFor = leadLookup(draft?.features.leads ?? null);
    return (preview?.contours ?? []).filter(c => c.sources.some(i => chosen.has(i))).flatMap(c => {
      const target = c.lead_target;
      if (!target) return [];
      const body = c.paths.filter(p => p.kind !== 'lead_in' && p.kind !== 'lead_out').flatMap(p => p.points);
      const area = body.reduce((sum, a, i) => { const b = body[(i + 1) % body.length]!; return sum + a[0] * b[1] - b[0] * a[1]; }, 0);
      const inside = draft?.features.leads?.side === 'inside' || (draft?.features.leads?.side === 'auto' && c.depth % 2 === 1);
      const side = (area >= 0) === inside ? 1 : -1;
      return (['entry', 'exit'] as const).flatMap(which => {
        const definition = definitionFor(target, which);
        if (definition?.shape !== 'line') return [];
        const path = c.paths.find(p => p.kind === (which === 'entry' ? 'lead_in' : 'lead_out'));
        if (!path || path.points.length < 2) return [];
        return [{ matching: c.matching_contour, which, target, definition, side, anchor: (which === 'entry' ? path.points.at(-1)! : path.points[0]!) as Point, end: (which === 'entry' ? path.points[0]! : path.points.at(-1)!) as Point }];
      });
    });
  });
  let leadDrag = $state<{ handle: LeadHandle; targets: Spot[]; point: Point; revision: number } | null>(null);
  const orderPaths = $derived(orderSegments(preview));
  const playback = $derived(ui.setupPanel === 'order' ? orderPosition(orderPaths, orderProgress) : null);

  // The drawing's layers: shown or hidden here, cut or skipped in the job.
  let layersOpen = $state(false);
  function skip(layer: string, skipped: boolean): void {
    if (!draft) return;
    featureEdits.change((f) => { f.skip_layers = skipped ? [...new Set([...f.skip_layers, layer])] : f.skip_layers.filter((l) => l !== layer); }).catch(fail);
  }
  const hidden = (layer: string) => ui.hiddenDrawingLayers.includes(layer);
  function hide(layer: string, off: boolean): void {
    ui.hiddenDrawingLayers = off ? [...new Set([...ui.hiddenDrawingLayers, layer])] : ui.hiddenDrawingLayers.filter((l) => l !== layer);
  }

  /** A mark size in millimetres that keeps its screen size. */
  const mark = $derived(Math.min(view.w, view.h) / 120);
  // Non-scaling round strokes keep the same dot size without rewriting a
  // radius on every contour during zoom. Recompute only with canvas size.
  const markPixels = $derived(Math.min(view.pixelWidth, Math.round(view.h / view.mmPerPixel)) / 120);
  $effect(() => { void ui.selectionEpoch; untrack(() => { selected = []; local = null; }); });
</script>

<div class="canvas-wrap">
  <div class="rail" role="toolbar" tabindex="-1" aria-label="More tools">{@render rail()}</div>
  <Stage {view} bed={frame.bed} head={frame.head} origin={ui.nestShown ? null : frame.origin} {grab} {ondrag} {ondragend} {ontap} {onmarquee}>
    <g transform="translate({zero[0]} {zero[1]})">
    {#if stockOutline.length}
      <path d={pathOf(stockOutline, false) + 'Z'} fill="var(--accent)" fill-opacity="0.035" stroke="var(--accent)" stroke-dasharray="8 5" stroke-width="1.5" vector-effect="non-scaling-stroke" pointer-events="none" />
    {/if}
    {#each stockCutouts as cutout}<path d={pathOf(cutout, false) + 'Z'} fill="var(--ink-3)" fill-opacity="0.16" stroke="var(--ink-3)" stroke-dasharray="3 3" stroke-width="1" vector-effect="non-scaling-stroke" pointer-events="none" />{/each}
    {#if ui.nestLive && !ui.nestPreview}
      <!-- A running search: each copy is a group of the drawing, moved. -->
      {#each ui.nestLive.copies as copy}
        <g transform={svgMatrix(copy.transform)}>
          {#each shapes.get(copy.group) ?? [] as contour}
            {#each contour.paths as path}{#if path.kind === 'cut'}<path class="path cut live" d={pathOf(path.points, false)} vector-effect="non-scaling-stroke" pointer-events="none" />{/if}{/each}
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
        <g class="shape" class:selected={on} class:dragging={dragging && on} data-group={g} transform={!movingAll && local && movingSet.has(g) ? svgMatrix(local.m) : ''}>
          {#each shapes.get(g) ?? [] as contour}
            {#if !hidden(contour.layer)}
              <path class="hit" d={hitPath(contour)} vector-effect="non-scaling-stroke"/>
              {#each contour.paths as path}
                {#if ui.layerShown(path.kind)}<path class="path {path.kind}" d={drawingPath(path.points)} vector-effect="non-scaling-stroke"/>{/if}
              {/each}
              {#if details && (ui.setupPanel !== 'nest' || on)}
                {#if ui.layerShown('cooling')}{#each contour.cooling as [x, y]}<path class="mark cooling" d="M{x} {y}h0" stroke="#d04c79" stroke-width={markPixels * 2} stroke-linecap="round" vector-effect="non-scaling-stroke"/>{/each}{/if}
                <path class="mark start" d="M{contour.start[0]} {contour.start[1]}h0" stroke="var(--accent)" stroke-width={Math.min(markPixels * 1.4, 4)} stroke-linecap="round" vector-effect="non-scaling-stroke"/>
              {/if}
              {#if contour.sources.length === 1 && (ranks.has(contour.sources[0]!) || ui.setupPanel === 'order')}
                <text class="order-label" transform="translate({contour.start[0] + 1.5 * mark} {contour.start[1] + 1.5 * mark}) scale(1 -1)" font-size={mark * 3}>{ranks.get(contour.sources[0]!) ?? contour.sources[0]! + 1}</text>
              {/if}
            {/if}
          {/each}
        </g>
      {/each}
      </g>
      {#each ui.picking?.marks ?? [] as [x, y]}<circle class="mark bridge" cx={x} cy={y} r={mark} />{/each}
      {#if playback}<circle cx={playback.point[0]} cy={playback.point[1]} r={mark * 1.7} fill="var(--accent)" stroke="var(--bg)" vector-effect="non-scaling-stroke" />{/if}
      {#each leadHandles as handle, i}<g data-lead={i} style="cursor:crosshair"><circle cx={handle.end[0]} cy={handle.end[1]} r={mark * 2} fill="var(--accent)" stroke="var(--bg)" vector-effect="non-scaling-stroke" /></g>{/each}
      {#if leadDrag}<path d="M{leadDrag.handle.anchor.join(' ')}L{leadDrag.point.join(' ')}" stroke="var(--accent)" stroke-width="2" vector-effect="non-scaling-stroke" />{/if}
      {#if firstEnd}<circle class="mark bridge" cx={firstEnd[0]} cy={firstEnd[1]} r={mark * 1.5} vector-effect="non-scaling-stroke"/>{/if}
      {#if selection && !ui.picking && !ui.nestShown && !ui.nestPicking}
        <g class="gizmo">
          <rect class="sel-box" x={selection.minX - 2 * mark} y={selection.minY - 2 * mark} width={selection.maxX - selection.minX + 4 * mark} height={selection.maxY - selection.minY + 4 * mark} vector-effect="non-scaling-stroke"/>
          <path class="stalk" d="M{centre(selection)[0]} {selection.maxY + 2 * mark}V{selection.maxY + 8 * mark}" vector-effect="non-scaling-stroke"/>
          <circle class="resize-handle" cx={selection.maxX + 2 * mark} cy={selection.maxY + 2 * mark} r={mark * 1.5} fill="var(--accent)" style="cursor:nwse-resize"/>
          <g class="handle" transform="translate({centre(selection)[0]} {selection.maxY + 8 * mark})"><circle r={3 * mark} class="hit"/><circle r={1.4 * mark} vector-effect="non-scaling-stroke"/></g>
        </g>
      {/if}
    {/if}
    </g>
    <g class="guide">{#each guides as [x1, y1, x2, y2]}<line {x1} {y1} {x2} {y2} vector-effect="non-scaling-stroke"/>{/each}</g>
    {#snippet overlay()}
      {#if ui.picking}
        <div class="pick-bar">
          <span>{hint}</span>
          {#if ui.picking.feature === 'bridges' && ui.picking.first}<button class="btn btn-ghost" onclick={() => { if (ui.picking) ui.picking = { ...ui.picking, first: null }; }}>Cancel end</button>{/if}
          <button class="btn btn-ghost" onclick={() => (ui.picking = null)}>Cancel</button>
          <button class="btn btn-primary" disabled={updating || pickBusy || !!ui.picking.first} onclick={finishPicks}>Finish</button>
        </div>
      {/if}
      {#if layersOpen && layers.length && draft}
        <div class="layers-pop">
          <div class="lp-head"><h3>Layers</h3><button class="link" onclick={() => (layersOpen = false)}>Close</button></div>
          {#each layers as layer (layer.name)}
            {@const skipped = draft.features.skip_layers.includes(layer.name)}
            <div class="lrow" class:hidden-layer={hidden(layer.name)}>
              <button class="eye" class:off={hidden(layer.name)} title="Show or hide" onclick={() => hide(layer.name, !hidden(layer.name))}><span class="sw" style="background:{skipped ? 'var(--ink-3)' : 'var(--ink)'}"></span></button>
              <div><div class="lname">{layer.name}</div><div class="lmeta">{layer.contours} contour{layer.contours === 1 ? '' : 's'} · {skipped ? 'Skipped' : 'Cut'}</div></div>
              <div class="seg"><button class:on={!skipped} onclick={() => skip(layer.name, false)}>Cut</button><button class:on={skipped} onclick={() => skip(layer.name, true)}>Skip</button></div>
            </div>
          {/each}
        </div>
      {/if}
      {#if preview?.warnings.length}<div class="hint">{preview.warnings[0]}</div>{/if}
      {#if selection && !ui.picking && !ui.nestShown && !ui.nestPicking}
        <div class="canvas-hud">
          <button class="hud-btn" disabled={updating} data-numpad onclick={() => typed(0)}>X {distance(selection.minX + zero[0])}</button>
          <button class="hud-btn" disabled={updating} data-numpad onclick={() => typed(1)}>Y {distance(selection.minY + zero[1])}</button>
          <button class="hud-btn" disabled={updating} data-numpad onclick={turnBy}>Rotate…</button>
          <button class="hud-btn" disabled={updating} onclick={() => resize(0)}>W {distance(selection.maxX - selection.minX)}</button><button class="hud-btn" disabled={updating} onclick={() => resize(1)}>H {distance(selection.maxY - selection.minY)}</button><button class="hud-btn" disabled={updating} onclick={() => resize('scale')}>Scale…</button>
        </div>
      {/if}
    {/snippet}
  </Stage>
</div>
<div class="drawing-toolbar" role="toolbar" aria-label="Drawing tools">
  <div class="tool-row">
    <button class="rail-btn" title="Frame the parts" onclick={fit}><i class="ic ic-fit"></i><small>Fit</small></button>
    <button class="rail-btn" title="Zoom in" onclick={() => view.zoom(1.25)}><i class="ic ic-plus"></i><small>Zoom in</small></button>
    <button class="rail-btn" title="Zoom out" onclick={() => view.zoom(0.8)}><i class="ic ic-minus"></i><small>Zoom out</small></button>
    <button class="rail-btn" class:on={layersOpen} title="Layers" onclick={() => (layersOpen = !layersOpen)} disabled={!layers.length}><i class="ic ic-layers"></i><small>Layers</small></button>
    <button class="rail-btn" class:on={ui.snap} onclick={() => ui.toggleSnap()}><strong>{ui.snap ? 'On' : 'Off'}</strong><small>Snap</small></button>
    <button class="rail-btn" onclick={gridSize}><strong>{quantity(ui.grid, 'mm')}</strong><small>Grid</small></button>
    <button class="rail-btn" title="Group the selected shapes" disabled={selectionLocked || selected.length < 2} onclick={() => groupSelection(true)}><svg width="23" height="23" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="2" y="2" width="20" height="20" rx="2" stroke-dasharray="3 2"/><rect x="5" y="5" width="7" height="7"/><rect x="12" y="12" width="7" height="7"/></svg><small>Group</small></button>
    <button class="rail-btn" title="Ungroup the selected shapes" disabled={selectionLocked || !selected.some(g => (groups[g]?.length ?? 0) > 1)} onclick={() => groupSelection(false)}><svg width="23" height="23" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="2" y="2" width="8" height="8"/><rect x="14" y="14" width="8" height="8"/><path d="M14 4h6v6M20 4l-7 7M4 14v6h6M4 20l7-7"/></svg><small>Ungroup</small></button>
    <button class="rail-btn" title="Undo the last edit" onclick={() => history(true)} disabled={!draft?.past}><i class="ic ic-undo"></i><small>Undo</small></button>
    <button class="rail-btn" title="Do the edit undone again" onclick={() => history(false)} disabled={!draft?.future}><i class="ic ic-redo"></i><small>Redo</small></button>
  </div>
  <div class="tool-row">
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Mirror horizontally" onclick={() => act('mirror')}><i class="ic ic-mirror"></i><small>Mirror X</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Mirror vertically" onclick={() => act('vertical')}><i class="ic ic-mirror vertical"></i><small>Mirror Y</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking || !frame.bed} onclick={centerOnBed}><i class="ic ic-target"></i><small>Center</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Turn the selection a quarter turn" onclick={() => act('turn')}><i class="ic ic-rotate"></i><small>90°</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} onclick={() => resize('scale')}><strong>%</strong><small>Scale</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Put the selection back where the drawing has it" onclick={() => act('reset')}><i class="ic ic-reset"></i><small>Reset</small></button>
    <button class="rail-btn" class:on={ui.setupPanel === 'clipboard'} disabled={pasting || selectionLocked} title="Copy the selection" onclick={copy}><i class="ic ic-copy"></i><small>Copy</small></button>
    <button class="rail-btn" title="Paste one copy" onclick={() => paste()} disabled={pasting || updating || !pasteable(clipboard, draft) || !!ui.picking || ui.nestShown || ui.nestPicking}><i class="ic ic-paste"></i><small>Paste</small></button>
    <button class="rail-btn" disabled={updating || !selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Take the selection off the sheet" onclick={remove}><i class="ic ic-trash"></i><small>Delete</small></button>
    <button class="rail-btn" disabled={!selection || !!ui.picking || ui.nestShown || ui.nestPicking} title="Clear the selection" onclick={() => (selected = [])}><i class="ic ic-x"></i><small>Deselect</small></button>
  </div>
</div>

<style>
  .resize-handle { pointer-events: all; }
  .drawing-toolbar { flex-shrink: 0; display: grid; gap: 8px; padding: 12px; border-top: 1px solid var(--line); background: var(--panel); overflow-x: auto; }
  .tool-row { display: grid; grid-template-columns: repeat(10, minmax(64px, 1fr)); gap: 8px; }
  .rail-btn { min-height: 66px; width: 100%; gap: 7px; }
  .rail-btn strong { font-size: var(--t-base); }
  .rail-btn small { font-size: var(--t-sm); letter-spacing: 0; }
  .rail-btn:disabled { opacity: .38; cursor: default; }
  .rail-btn .ic { width: 23px; height: 23px; }
  .vertical { transform: rotate(90deg); }
</style>
