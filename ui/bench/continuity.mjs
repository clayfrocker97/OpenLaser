// Run only against the private loopback simulator. Samples the actual SVG
// on animation frames and after DOM updates, including optimistic drags.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:8px;right:8px;z-index:10000;padding:12px;background:#102730;color:white;font:12px monospace';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Preview continuity checks'; button.textContent = 'Verify preview continuity';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!test()) { if (performance.now() - start > 30000) throw Error('Timed out'); await delay(10); } };
const draft = () => server.doc.draft;
const prepared = async () => { await wait(() => draft()?.preview && draft().error !== 'preparing geometry'); await frames(); };
const svg = () => document.querySelector('svg[aria-label="Drawing"]');
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const fit = async () => { document.querySelector('button[title="Show the bed"]').click(); await frames(); };
const key = (value, options = {}) => svg().dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, ...options }));
const pointer = (target, type, at, button = 0) => target.dispatchEvent(new PointerEvent(type, {
  bubbles: true, pointerId: 91, pointerType: 'mouse', isPrimary: true,
  clientX: at.x, clientY: at.y, button, buttons: type === 'pointerup' ? 0 : button === 1 ? 4 : 1,
}));
const point = () => {
  const shape = svg()?.querySelector('[data-group="0"]');
  const paths = [...(shape?.querySelectorAll('.path.cut') ?? [])];
  if (!paths.length) return null;
  // Leads can legitimately shorten near another part after a move. Track
  // the part's cut bounds, not the regenerated approach endpoint.
  const boxes = paths.map(path => path.getBBox());
  return new DOMPoint(Math.min(...boxes.map(box => box.x)), Math.min(...boxes.map(box => box.y)))
    .matrixTransform(shape.getScreenCTM());
};

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'preview-continuity', protocol: 1, baseline: __CONTINUITY_BASELINE__, started_utc: new Date().toISOString(), checks: [], errors: [] };
  const onError = event => result.errors.push(event.message);
  window.addEventListener('error', onError);
  async function delayed(changes, operation) {
    const ready = server.doc, revision = ready.draft_revision + 1;
    // A local transport revision prevents an already queued ready SSE from
    // ending the controlled gap early. No API request uses this revision.
    server.applyDraft({ ...ready.draft, ...changes, revision, preview: null, compiled: null, error: 'preparing geometry' }, revision);
    try {
      check(draft().preview === null, 'Authoring state must remain unprepared');
      await operation();
    } finally { server.apply(ready, true); }
    await frames();
  }
  async function measure(name, operation) {
    const record = { name, frames: 0, mutations: 0, preparing_frames: 0, blank_frames: 0, blank_mutations: 0,
      missing_bed: 0, min_shapes: Infinity, max_position_error_px: 0, samples: [] };
    let animation, running = true, expected = null;
    const sample = kind => {
      const stage = svg(), shapes = stage?.querySelectorAll('[data-group]').length ?? 0;
      const preparing = draft()?.error === 'preparing geometry';
      const position = expected ? point() : null;
      const error = position ? Math.hypot(position.x - expected.x, position.y - expected.y) : null;
      if (kind === 'frame') { record.frames++; if (preparing) record.preparing_frames++; if (!shapes) record.blank_frames++; }
      else { record.mutations++; if (!shapes) record.blank_mutations++; }
      if (!stage?.querySelector('.bed')) record.missing_bed++;
      record.min_shapes = Math.min(record.min_shapes, shapes);
      if (error !== null) record.max_position_error_px = Math.max(record.max_position_error_px, error);
      record.samples.push({ kind, at: performance.now(), preparing, shapes, position_error_px: error });
    };
    const loop = () => { if (running) { sample('frame'); animation = requestAnimationFrame(loop); } };
    const observer = new MutationObserver(() => sample('mutation'));
    observer.observe(svg(), { childList: true, subtree: true, attributes: true });
    animation = requestAnimationFrame(loop);
    try {
      await operation(target => { expected = target; });
      await frames(); await delay(50);
    } finally {
      running = false; cancelAnimationFrame(animation); observer.disconnect();
      record.passed = record.frames > 0 && !record.blank_frames && !record.blank_mutations && !record.missing_bed && record.max_position_error_px < .1;
      result.checks.push(record);
      label.textContent = `${name}: ${record.passed ? 'passed' : 'discontinuity'}`;
    }
  }

  async function drag(name, all) {
    key('Escape'); if (all) key('a', { ctrlKey: true }); await frames();
    const at = point(), startRevision = draft().revision;
    const before = draft().placed.map(placement => placement.transform);
    check(at, 'First shape is visible');
    const end = { x: at.x + 12, y: at.y - 9 };
    await measure(name, async expectPoint => {
      pointer(svg().querySelector('[data-group="0"] .hit'), 'pointerdown', at);
      pointer(svg(), 'pointermove', end);
      pointer(svg(), 'pointerup', end);
      expectPoint(end);
      await wait(() => draft().revision > startRevision); await prepared();
      const changed = draft().placed.filter((placement, i) => placement.transform.some((v, j) => Math.abs(v - before[i][j]) > .001)).length;
      check(changed === (all ? before.length : draft().groups[0].length), `Expected ${all ? 'all' : 'one group of'} contours to move; changed ${changed}`);
    });
  }

  try {
    const manifest = await fetch('/bench/manifest').then(r => r.json());
    await wait(() => server.doc);
    await api.openPart(manifest.scenes.find(scene => scene.parts === 1666).id);
    ui.tab = 'setup'; ui.setupPanel = null; ui.modal = null; ui.snap = false;
    await prepared();
    await api.setRecipe(manifest.recipe); await prepared();
    const leads = { closed_only: true, entry: { angle: 165, length: 2, radius: 1, shape: 'line' }, exit: null, side: 'auto' };
    await api.setFeatures({ ...manifest.features, leads }); await prepared();
    await api.transform(draft().placed.map((_, i) => i), null); await prepared();
    const bounds = draft().preview.bounds;
    await api.setOrigin([bounds.min.x, bounds.min.y], 'front_left'); await prepared(); await fit();
    result.parts = draft().groups.length; result.contours = draft().preview.contours.length;

    // A controlled transport gap makes this reproducible on fast hosts.
    // It never sends the synthetic pending state to the backend.
    await measure('delayed preparation with changed groups and origin', async () => {
      await delayed({ groups: [], placed: [], zero: [500, 500], origin: null }, () => delay(350));
    });

    for (const [name, value] of [
      ['lead angle edit', { ...leads, entry: { ...leads.entry, angle: 90 } }],
      ['leads off', null], ['leads on', leads],
    ]) await measure(name, async () => { await api.setFeatures({ ...draft().features, leads: value }); await prepared(); });

    await drag('drag one part and retain release position', false);
    await measure('undo single drag', async () => { await api.undo(); await prepared(); });
    await drag('drag all parts and retain release position', true);
    for (const [name, operation] of [['undo all drag', api.undo], ['redo all drag', api.redo], ['restore all positions', api.undo]]) {
      await measure(name, async () => { await operation(); await prepared(); });
    }
    key('Escape'); await frames();

    await measure('pan and zoom during preparation', async () => {
      const before = svg().getAttribute('viewBox');
      await delayed({}, async () => {
        await frames();
        const rect = svg().getBoundingClientRect(), at = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        svg().dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: at.x, clientY: at.y, deltaY: -20 }));
        pointer(svg(), 'pointerdown', at, 1);
        pointer(svg(), 'pointerup', { x: at.x + 10, y: at.y + 8 }, 1);
        await frames(); await delay(150);
        check(svg().getAttribute('viewBox') !== before, 'Camera must remain usable during preparation');
      });
    });
    await fit();
    await measure('save job', async () => { await api.saveJob('Preview continuity QA'); await prepared(); });
    check(result.checks.some(row => row.preparing_frames > 0), 'Must observe actual pending frames');
  } catch (error) { result.errors.push(String(error.stack ?? error)); }
  window.removeEventListener('error', onError);
  result.passed = !result.errors.length && result.checks.every(row => row.passed);
  label.textContent = `Continuity ${result.passed ? 'passed' : 'failed'} · ${result.checks.filter(row => row.passed).length}/${result.checks.length}`;
  await fetch('/bench/result', { method: 'POST', body: JSON.stringify(result) });
  button.disabled = false;
});
