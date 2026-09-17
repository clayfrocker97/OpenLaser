// Functional checks for events that arrive before the next animation frame.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:8px;right:8px;z-index:10000;padding:12px;background:#102730;color:white;font:12px monospace';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Viewer gesture checks'; button.textContent = 'Verify gestures';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(r => setTimeout(r, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!test()) { if (performance.now() - start > 30000) throw Error('Timed out'); await delay(10); } };
const prepared = async () => { await wait(() => server.doc?.draft?.preview && server.doc.draft.error !== 'preparing geometry'); await frames(); };
const draft = () => server.doc.draft;
const svg = () => document.querySelector('svg[aria-label="Drawing"]');
const check = (test, message) => { if (!test) throw Error(message); };
const view = () => svg().getAttribute('viewBox').split(' ').map(Number);
const equalView = expected => check(view().every((v, i) => Math.abs(v - expected[i]) < .001), `Camera ${view()} differs from ${expected}`);
const fit = async () => { document.querySelector('button[title="Show the bed"]').click(); await frames(); };
const event = (target, type, at, id = 91, pointerType = 'mouse', button = 0) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: id, pointerType, isPrimary: id === 91, clientX: at.x, clientY: at.y, button, buttons: type === 'pointerup' || type === 'pointercancel' ? 0 : button === 1 ? 4 : 1 }));
const endpoint = () => {
  const shape = svg().querySelector('[data-group="0"]');
  const p = draft().preview.contours[0].paths[0].points[0];
  return { target: shape.querySelector('.hit'), at: new DOMPoint(p[0], p[1]).matrixTransform(shape.getScreenCTM()) };
};
const reset = async () => { await api.transform(draft().placed.map((_, i) => i), null); await prepared(); await fit(); };

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'gesture-qa', pilot: true, started_utc: new Date().toISOString(), results: [], errors: [], checks: [] };
  const passed = name => { result.checks.push(name); label.textContent = `Passed: ${name}`; };
  try {
    const manifest = await fetch('/bench/manifest').then(r => r.json());
    await wait(() => server.doc);
    await api.openPart(manifest.scenes.find(s => s.parts === 1).id);
    ui.tab = 'setup'; ui.setupPanel = null; ui.modal = null; ui.snap = false;
    await prepared(); await api.setFeatures(manifest.features); await prepared(); await reset();
    const b = draft().preview.bounds;
    await api.setOrigin([b.min.x, b.min.y], 'front_left'); await prepared(); await fit();

    let { target, at } = endpoint();
    const k = view()[2] / svg().getBoundingClientRect().width;
    let revision = draft().revision;
    event(target, 'pointerdown', at);
    event(svg(), 'pointermove', { x: at.x + 8 / k, y: at.y - 5 / k });
    event(svg(), 'pointerup', { x: at.x + 12 / k, y: at.y - 9 / k });
    await wait(() => draft().revision > revision); await prepared();
    check(draft().placed.every(p => Math.abs(p.transform[4] - 12) < .001 && Math.abs(p.transform[5] - 9) < .001), 'Release position must be committed before rAF');
    passed('release before animation frame'); await reset();

    for (const rendered of [false, true]) {
      ({ target, at } = endpoint()); revision = draft().revision;
      event(target, 'pointerdown', at);
      event(svg(), 'pointermove', { x: at.x + 30, y: at.y + 20 });
      if (rendered) await frames();
      event(svg(), 'pointercancel', { x: at.x + 30, y: at.y + 20 });
      await frames(); await delay(50);
      check(draft().revision === revision && draft().placed.every(p => p.transform[4] === 0 && p.transform[5] === 0), 'Cancelled drag must not commit');
      passed(rendered ? 'cancel after rendered drag' : 'cancel before animation frame');
    }

    await fit();
    const rect = svg().getBoundingClientRect();
    let camera = view();
    for (const [u, v, deltaY] of [[.3, .4, -60], [.65, .55, -40], [.45, .7, 20]]) {
      const wheel = new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: rect.left + rect.width * u, clientY: rect.top + rect.height * v, deltaY });
      // Chromium's WheelEvent constructor truncates fractional client pixels.
      const actualU = (wheel.clientX - rect.left) / rect.width, actualV = (wheel.clientY - rect.top) / rect.height;
      const width = camera[2] / Math.exp(-deltaY * .0015), height = width * camera[3] / camera[2];
      camera = [camera[0] + actualU * (camera[2] - width), camera[1] + actualV * (camera[3] - height), width, height];
      svg().dispatchEvent(wheel);
    }
    await frames(); equalView(camera); passed('accumulated wheel deltas with moving cursor');

    await fit(); camera = view();
    const panStart = { x: rect.left + rect.width * .4, y: rect.top + rect.height * .4 };
    const panEnd = { x: panStart.x + 70, y: panStart.y - 40 };
    event(svg(), 'pointerdown', panStart, 91, 'mouse', 1);
    event(svg(), 'pointermove', { x: panStart.x + 40, y: panStart.y - 25 }, 91, 'mouse', 1);
    event(svg(), 'pointerup', panEnd, 91, 'mouse', 1);
    await frames(); equalView([camera[0] - 70 * camera[2] / rect.width, camera[1] + 40 * camera[2] / rect.width, camera[2], camera[3]]);
    passed('middle-button pan release');

    await fit(); camera = view(); revision = draft().revision;
    const a = { x: rect.left + rect.width * .3, y: rect.top + rect.height * .5 };
    const c = { x: rect.left + rect.width * .7, y: a.y };
    const a2 = { x: a.x - rect.width * .04 + 15, y: a.y + 10 };
    const c2 = { x: c.x + rect.width * .04 + 15, y: c.y + 10 };
    event(svg(), 'pointerdown', a, 91, 'touch'); event(svg(), 'pointerdown', c, 92, 'touch');
    event(svg(), 'pointermove', a2, 91, 'touch'); event(svg(), 'pointermove', c2, 92, 'touch');
    event(svg(), 'pointerup', a2, 91, 'touch'); event(svg(), 'pointerup', c2, 92, 'touch');
    await frames();
    const w = camera[2] / 1.2, h = camera[3] / 1.2;
    equalView([camera[0] + camera[2] / 2 - (.5 + 15 / rect.width) * w, camera[1] + camera[3] / 2 - (.5 + 10 / rect.height) * h, w, h]);
    check(draft().revision === revision, 'Pinch must not edit parts'); passed('two-finger pinch and release');

    await fit(); ({ target, at } = endpoint()); revision = draft().revision;
    const moved = { x: at.x + 35, y: at.y + 20 }, second = { x: at.x - 60, y: at.y - 30 };
    event(target, 'pointerdown', at, 91, 'touch'); event(svg(), 'pointermove', moved, 91, 'touch'); await frames();
    event(svg(), 'pointerdown', second, 92, 'touch'); event(svg(), 'pointerup', moved, 91, 'touch'); event(svg(), 'pointerup', second, 92, 'touch');
    await frames(); await delay(50); check(draft().revision === revision, 'Second finger must cancel a part drag');
    passed('second finger cancels part drag'); await fit();
    label.textContent = `Gesture checks complete · ${result.checks.length} passed`;
  } catch (error) { result.errors.push(String(error.stack ?? error)); label.textContent = `FAILED: ${error.message}`; }
  await fetch('/bench/result', { method: 'POST', body: JSON.stringify(result) });
  button.disabled = false;
});
