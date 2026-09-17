// Benchmark-only entry point. Replays the actual drawing event handlers and
// records browser scheduling; this is not a measurement of physical input lag.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';
import { defaultLeads } from '../src/lib/features';

const params = new URLSearchParams(location.search);
const variant = location.pathname.split('/').filter(Boolean)[0];
const manifest = await fetch('/bench/manifest').then(r => r.json());
const chosen = (params.get('parts') ?? '1,256,1024,1666').split(',').map(Number);
const repeats = Number(params.get('repeats') ?? 1);
const duration = Number(params.get('duration') ?? 3000);
const maxZoom = Number(params.get('zoom') ?? 8);
const panel = document.createElement('aside');
panel.id = 'viewer-benchmark';
panel.style.cssText = 'position:fixed;z-index:10000;right:8px;top:8px;max-width:440px;padding:8px 12px;border:1px solid #7196a5;background:#102730;color:#eefaff;font:12px/1.4 monospace;border-radius:8px';
const label = document.createElement('div');
const button = document.createElement('button');
button.textContent = 'Run viewer benchmark';
button.style.cssText = 'margin-top:4px;background:#b4e9d5;color:#102730;padding:5px 12px;border-radius:4px';
panel.append(label, button);
document.body.append(panel);
const status = s => { label.textContent = `${variant} · ${s}`; };
status(`ready · ${chosen.join(', ')} parts × ${repeats}`);
const delay = ms => new Promise(r => setTimeout(r, ms));
const frames = async (n = 2) => { await tick(); for (let i = 0; i < n; i++) await new Promise(requestAnimationFrame); };
const waitFor = async (test, description, timeout = 120000) => {
  const start = performance.now();
  while (!test()) { if (performance.now() - start > timeout) throw Error(`Timed out: ${description}`); await delay(25); }
};
const assert = (yes, message) => { if (!yes) throw Error(message); };
const draft = () => server.doc?.draft;
const prepared = async () => {
  await waitFor(() => !!draft()?.preview && draft()?.error !== 'preparing geometry', 'prepared preview');
  assert(!draft().error, `Preparation error: ${draft().error}`);
  await frames();
};
const drawing = () => document.querySelector('svg[aria-label="Drawing"]');
const fit = async () => { document.querySelector('button[title="Show the bed"]').click(); await frames(); };
const stats = samples => {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = p => sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))] : null;
  return { count: samples.length, p50_ms: at(.5), p95_ms: at(.95), p99_ms: at(.99), max_ms: at(1), over_33ms: samples.filter(v => v > 33.34).length, over_50ms: samples.filter(v => v > 50).length };
};
const counts = () => {
  const preview = draft()?.preview;
  return {
    groups: draft()?.groups.length, contours: preview?.contours.length,
    points: preview?.contours.reduce((n, c) => n + c.paths.reduce((m, p) => m + p.points.length, 0), 0),
    lead_ins: preview?.contours.reduce((n, c) => n + c.paths.filter(p => p.kind === 'lead_in').length, 0),
    rendered_groups: document.querySelectorAll('[data-group]').length,
    svg_nodes: drawing()?.querySelectorAll('*').length,
  };
};
async function measured(name, work) {
  status(name);
  const intervals = [], longTasks = [], longFrames = [], visibility = [];
  const supported = PerformanceObserver.supportedEntryTypes ?? [];
  const observers = [];
  for (const [type, list] of [['longtask', longTasks], ['long-animation-frame', longFrames]]) {
    if (!supported.includes(type)) continue;
    const observer = new PerformanceObserver(records => list.push(...records.getEntries().map(e => ({ start_ms: e.startTime, duration_ms: e.duration, blocking_ms: e.blockingDuration ?? null }))));
    observer.observe({ type }); observers.push(observer);
  }
  const onVisibility = () => visibility.push({ time_ms: performance.now(), state: document.visibilityState, focused: document.hasFocus() });
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('blur', onVisibility);
  const state = { start_visibility: document.visibilityState, start_focused: document.hasFocus() };
  let running = true, last = performance.now(), raf;
  function sample(now) { intervals.push(now - last); last = now; if (running) raf = requestAnimationFrame(sample); }
  raf = requestAnimationFrame(sample);
  const start = performance.now();
  let detail;
  try { detail = await work(); await frames(); }
  finally {
    running = false; cancelAnimationFrame(raf);
    document.removeEventListener('visibilitychange', onVisibility); window.removeEventListener('blur', onVisibility);
    for (const observer of observers) observer.disconnect();
  }
  return { name, elapsed_ms: performance.now() - start, ...state, visibility, frames: stats(intervals), frame_intervals_ms: intervals, long_tasks: supported.includes('longtask') ? longTasks : null, long_animation_frames: supported.includes('long-animation-frame') ? longFrames : null, ...detail, ...counts() };
}
async function replay(send) {
  const dispatchTimes = [], intervals = [];
  const start = performance.now();
  let lastFrame = start, raf;
  function sample(now) { intervals.push(now - lastFrame); lastFrame = now; raf = requestAnimationFrame(sample); }
  raf = requestAnimationFrame(sample);
  let previous = 0;
  while (previous < 1) {
    const progress = Math.min(1, (performance.now() - start) / duration);
    const t = performance.now(); send(progress, previous); dispatchTimes.push(performance.now() - t);
    previous = progress;
    if (progress < 1) await delay(1000 / 120);
  }
  await frames(1);
  cancelAnimationFrame(raf);
  return { replay_elapsed_ms: performance.now() - start, dispatched: dispatchTimes.length, dispatch: stats(dispatchTimes), dispatch_times_ms: dispatchTimes,
    interaction_frames: stats(intervals), interaction_intervals_ms: intervals };
}
const key = (key, ctrlKey = false) => document.body.dispatchEvent(new KeyboardEvent('keydown', { key, ctrlKey, bubbles: true, cancelable: true }));
const pointer = (target, type, x, y) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 81, pointerType: 'mouse', isPrimary: true, button: 0, buttons: type === 'pointerup' ? 0 : 1, clientX: x, clientY: y }));
// These gestures start with no transient group transform. Project the known
// drawing coordinates through the serialized camera instead of getScreenCTM:
// SVG's float32 matrix rounds a small part's center far from the origin enough
// to invalidate an otherwise exact resize assertion at some viewport sizes.
const screenPoint = (_element, x, y) => {
  const svg = drawing(), rect = svg.getBoundingClientRect();
  const [vx, vy, vw, vh] = svg.getAttribute('viewBox').split(/\s+/).map(Number);
  const scale = Math.min(rect.width / vw, rect.height / vh);
  const [zx, zy] = draft().zero ?? [0, 0];
  return {
    x: rect.left + (rect.width - vw * scale) / 2 + (x + zx - vx) * scale,
    y: rect.top + (rect.height - vh * scale) / 2 + (-y - zy - vy) * scale,
  };
};
async function resetTransforms() {
  await api.transform(draft().placed.map((_, i) => i), null);
  await prepared();
  key('Escape'); await fit();
}
async function motion(name, all, resize = false) {
  key('Escape'); if (all) key('a', true); await frames();
  if (all) assert(document.querySelectorAll('.shape.selected').length === draft().groups.length, 'All parts selected before motion');
  const svg = drawing();
  let target, start, resizeCenter;
  if (resize) {
    target = svg.querySelector('.resize-handle');
    assert(target, 'Resize handle is visible');
    start = screenPoint(target, Number(target.getAttribute('cx')), Number(target.getAttribute('cy')));
    const box = svg.querySelector('.sel-box');
    // SVGAnimatedLength.baseVal stores float32; use the exact serialized
    // attributes so a small part far from zero is not a false size failure.
    resizeCenter = screenPoint(box, Number(box.getAttribute('x')) + Number(box.getAttribute('width')) / 2, Number(box.getAttribute('y')) + Number(box.getAttribute('height')) / 2);
  } else {
    const g = Math.floor(draft().groups.length / 2);
    const shape = svg.querySelector(`[data-group="${g}"]`);
    target = shape?.querySelector('.hit');
    const contour = draft().preview.contours.find(c => c.sources.includes(draft().groups[g][0]));
    const p = contour.paths.find(p => p.kind !== 'lead_in').points[0];
    assert(target, `Shape ${g} is rendered`);
    start = screenPoint(shape, p[0], p[1]);
  }
  const rect = svg.getBoundingClientRect();
  const [, , vw, vh] = svg.getAttribute('viewBox').split(/\s+/).map(Number);
  const k = Math.max(vw / rect.width, vh / rect.height);
  const before = structuredClone(draft().placed);
  const revision = draft().revision;
  const selectedIndices = all ? before.map((_, i) => i) : draft().groups[Math.floor(draft().groups.length / 2)];
  const end = resize ? { x: start.x - rect.width * .045, y: start.y + rect.height * .03 } : { x: start.x + 6 / k, y: start.y - 4 / k };
  const expectedScale = resize ? Math.hypot(end.x - resizeCenter.x, end.y - resizeCenter.y) / Math.hypot(start.x - resizeCenter.x, start.y - resizeCenter.y) : 1;
  // The path sweeps out and returns to a small, known displacement.
  return measured(name, async () => {
    pointer(target, 'pointerdown', start.x, start.y);
    const replayMetrics = await replay(p => {
      const wave = Math.sin(p * Math.PI * 2) * (resize ? 12 : 40);
      pointer(svg, 'pointermove', start.x + (end.x - start.x) * p + wave, start.y + (end.y - start.y) * p - wave * .3);
    });
    const released = performance.now();
    pointer(svg, 'pointerup', end.x, end.y);
    await waitFor(() => draft()?.revision > revision, `${name} commit`);
    await prepared();
    const commitMs = performance.now() - released;
    const after = draft().placed;
    let checked = 0;
    for (const i of selectedIndices) {
      const a = after[i].transform, b = before[i].transform;
      assert(a.every(Number.isFinite), `${name}: finite transform`);
      if (!resize) assert(Math.abs(a[4] - b[4] - 6) < .03 && Math.abs(a[5] - b[5] - 4) < .03, `${name}: final displacement (${a[4] - b[4]}, ${a[5] - b[5]})`);
      else assert(Math.abs(a[0] - expectedScale * b[0]) < 1e-7 && Math.abs(a[0] - a[3]) < 1e-9, `${name}: exact uniform scale ${a[0]} != ${expectedScale * b[0]} at ${JSON.stringify({start, end, resizeCenter})}`);
      checked++;
    }
    if (!all) {
      const chosen = new Set(selectedIndices);
      for (let i = 0; i < before.length; i++) if (!chosen.has(i)) assert(after[i].transform.every((v, j) => v === before[i].transform[j]), `${name}: unselected contour unchanged`);
    }
    return { ...replayMetrics, commit_ms: commitMs, checked_contours: checked, expected_scale: expectedScale, final_transform: after[selectedIndices[0]].transform };
  });
}
async function runScene(scene, repetition) {
  const result = { variant, scene, repetition, operations: [] };
  status(`load ${scene.parts} parts`);
  result.operations.push(await measured('open-sheet', async () => {
    await api.openPart(scene.id); ui.tab = 'setup'; ui.setupPanel = null; ui.modal = null; ui.snap = false;
    ui.hiddenLayers = []; ui.hiddenDrawingLayers = []; await prepared();
  }));
  if (draft().recipe?.id !== manifest.recipe) { await api.setRecipe(manifest.recipe); await prepared(); }
  if (draft().placed.some(p => p.transform.some((v, i) => v !== [1, 0, 0, 1, 0, 0][i]))) { await api.transform(draft().placed.map((_, i) => i), null); await prepared(); }
  if (JSON.stringify(draft().features) !== JSON.stringify(manifest.features)) { await api.setFeatures(structuredClone(manifest.features)); await prepared(); }
  const bounds = draft().preview.bounds;
  if (!draft().zero?.every(v => Math.abs(v) < 1e-9)) { await api.setOrigin([bounds.min.x, bounds.min.y], 'front_left'); await prepared(); }
  await fit();
  assert(draft().groups.length === scene.parts, 'Expected number of parts');
  assert(draft().preview.contours.length === scene.contours, 'Expected number of contours');
  result.operations.push(await measured('add-all-lead-ins', async () => {
    const started = performance.now();
    await api.setFeatures({ ...structuredClone(manifest.features), leads: defaultLeads() });
    const apiMs = performance.now() - started;
    await prepared();
    assert(counts().lead_ins === scene.contours, `Lead-in count ${counts().lead_ins} != ${scene.contours}`);
    return { api_reply_ms: apiMs };
  }));
  const svg = drawing(), rect = svg.getBoundingClientRect();
  const initial = svg.getAttribute('viewBox');
  result.operations.push(await measured('zoom-in-out', async () => {
    const logZoom = p => Math.sin(p * Math.PI) * Math.log(maxZoom);
    const metrics = await replay((p, previous) => svg.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2, deltaY: -(logZoom(p) - logZoom(previous)) / .0015 })));
    await frames();
    const before = initial.split(' ').map(Number), after = svg.getAttribute('viewBox').split(' ').map(Number);
    assert(before.every((v, i) => Math.abs(v - after[i]) < .01), 'Zoom returns to its initial view');
    return { ...metrics, initial_view: initial, final_view: svg.getAttribute('viewBox') };
  }));
  await fit();
  result.operations.push(await motion('drag-one', false));
  await resetTransforms();
  result.operations.push(await motion('drag-all', true));
  await resetTransforms();
  result.operations.push(await motion('resize-all', true, true));
  await resetTransforms();
  result.operations.push(await measured('remove-all-lead-ins', async () => {
    await api.setFeatures(structuredClone(manifest.features)); await prepared();
    assert(counts().lead_ins === 0, 'All lead-ins removed');
  }));
  return result;
}
button.addEventListener('click', async () => {
  button.disabled = true;
  const report = {
    variant, pilot: params.has('pilot'), started_utc: new Date().toISOString(), user_agent: navigator.userAgent,
    viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
    hardware_concurrency: navigator.hardwareConcurrency, duration_ms: duration, max_zoom: maxZoom,
    note: 'Synthetic 120 Hz target input replay in visible production build; timer cadence is measured, not guaranteed. rAF intervals are browser scheduling, not physical input-to-photon latency.',
    results: [], errors: [],
  };
  try {
    await waitFor(() => server.doc, 'document');
    for (let repetition = 0; repetition < repeats; repetition++) {
      for (const scene of manifest.scenes.filter(s => chosen.includes(s.parts))) {
        const result = await runScene(scene, repetition); report.results.push(result);
        await fetch('/bench/result', { method: 'POST', body: JSON.stringify({ ...report, partial: true, results: [result] }) });
      }
    }
    status(`complete · ${report.results.length} sheets · results saved`);
  } catch (error) { report.errors.push(String(error.stack ?? error)); status(`FAILED · ${error.message}`); }
  report.finished_utc = new Date().toISOString();
  await fetch('/bench/result', { method: 'POST', body: JSON.stringify(report) });
  button.disabled = false;
});
