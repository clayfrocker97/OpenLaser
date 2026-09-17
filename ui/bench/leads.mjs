// Opt-in acceptance harness, served only by the private loopback simulator.
// Sends pointer gestures through the production Stage/Canvas handlers.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:8px;right:8px;z-index:10000;padding:12px;background:#102730;color:white;font:12px monospace';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Lead selection checks'; button.textContent = 'Verify lead selection';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!test()) { if (performance.now() - start > 30000) throw Error('Timed out'); await delay(10); } };
const draft = () => server.doc.draft;
const prepared = async () => { await wait(() => draft()?.preview && draft().error !== 'preparing geometry'); if (draft().error) throw Error(draft().error); await frames(); };
const svg = () => document.querySelector('svg[aria-label="Drawing"]');
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const key = (value, options = {}) => svg().dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, ...options }));
const pointer = (target, type, at, options = {}) => target.dispatchEvent(new PointerEvent(type, {
  bubbles: true, pointerId: 91, pointerType: 'mouse', isPrimary: true,
  clientX: at.x, clientY: at.y, button: 0, buttons: type === 'pointerup' ? 0 : 1,
  ...options,
}));
const snapshot = () => JSON.parse(JSON.stringify(draft().preview.contours));
// HTTP replies and SSE can serialize object keys in different orders.
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const clickTitle = async title => { document.querySelector(`button[title="${title}"]`).click(); await frames(); };
const selectedGroups = () => [...svg().querySelectorAll('.shape.selected')].map(shape => Number(shape.dataset.group));
const select = async (group, additive = false) => {
  // Select from the drawing before opening the lead tool, as an operator
  // would; at full-bed zoom the lead handles can cover the part outline.
  const panel = ui.setupPanel;
  ui.setupPanel = null;
  if (!additive) key('Escape');
  await frames();
  let shape = svg().querySelector(`[data-group="${group}"] .hit`);
  check(shape, 'Selectable shape must be visible');
  const bounds = shape.getBoundingClientRect();
  if (bounds.width < 60) {
    svg().dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true,
      clientX: bounds.left + bounds.width / 2, clientY: bounds.top + bounds.height / 2,
      deltaY: -Math.log(80 / Math.max(bounds.width, 1)) / .0015 }));
    await frames();
    shape = svg().querySelector(`[data-group="${group}"] .hit`);
    check(shape, 'Zoom retains the selected part');
  }
  const point = shape.getPointAtLength(shape.getTotalLength() / 3).matrixTransform(shape.getScreenCTM());
  pointer(shape, 'pointerdown', point, { shiftKey: additive }); pointer(shape, 'pointerup', point, { shiftKey: additive }); await frames();
  check(selectedGroups().includes(group), `Part ${group} is selected`);
  ui.setupPanel = panel; await frames();
  check(svg().querySelector('[data-lead] circle'), 'Selection exposes a lead handle');
};

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'lead-selection', protocol: 3, started_utc: new Date().toISOString(), checks: [], errors: [] };
  const onError = event => result.errors.push(event.message);
  window.addEventListener('error', onError);
  async function measure(name, operation) {
    label.textContent = name;
    const detail = await operation();
    result.checks.push({ name, passed: true, ...detail });
  }
  async function drag(name, handleIndex = 0) {
    await measure(name, async () => {
      const before = snapshot(), globals = { ...draft().features.leads, overrides: [] }, revision = draft().revision;
      const groups = selectedGroups(), selected = new Set(groups.flatMap(group => draft().groups[group]));
      const candidates = before.filter(c => c.sources.some(source => selected.has(source)) && c.lead_target);
      const target = candidates[Math.floor(handleIndex / 2)];
      check(target, 'Dragged contour exists');
      const expected = before.flatMap((c, i) => c.sources.some(source => selected.has(source)) &&
        (target.matching_contour === null ? c === target : c.matching_contour === target.matching_contour) ? [i] : []);
      const past = draft().past;
      const handle = svg().querySelector(`[data-lead="${handleIndex}"] circle`);
      check(handle, `Lead handle ${handleIndex} exists`);
      const start = new DOMPoint(+handle.getAttribute('cx'), +handle.getAttribute('cy')).matrixTransform(handle.getScreenCTM());
      const end = { x: start.x + 6, y: start.y - 4 };
      const began = performance.now();
      pointer(handle, 'pointerdown', start); pointer(svg(), 'pointermove', end); pointer(svg(), 'pointerup', end);
      await wait(() => draft().revision > revision); await prepared();
      const readyMs = performance.now() - began;
      const after = snapshot(), changed = after.flatMap((c, i) => same(c.paths, before[i].paths) ? [] : [i]);
      check(expected ? same(changed, expected) : changed.length === 1, `Expected ${expected?.length ?? 1} selected contours to change, got ${changed.length}`);
      const role = handleIndex % 2 === 0 ? 'lead_in' : 'lead_out';
      for (const index of changed) check(same(before[index].paths.filter(p => p.kind !== role), after[index].paths.filter(p => p.kind !== role)), 'The other role and body must be unchanged');
      check(same(globals, { ...draft().features.leads, overrides: [] }), 'Job defaults must be unchanged');
      check(draft().past === past + 1, 'One drag is one undo step');
      if (expected) {
        const owners = new Set(expected.map(i => before[i].lead_target.contour));
        const edits = draft().features.leads.overrides.filter(edit => owners.has(edit.location.contour));
        const definitions = edits.map(edit => edit[handleIndex % 2 === 0 ? 'entry' : 'exit']);
        check(definitions.length === expected.length && definitions.every(value => same(value, definitions[0])), 'Selected leads share the same definition');
      }
      return { contours: after.length, selected_groups: groups.length, changed, role, overrides: draft().features.leads.overrides.length, ready_ms: readyMs };
    });
  }
  try {
    const manifest = await fetch('/bench/manifest').then(r => r.json());
    await wait(() => server.doc);
    const settings = { entry: { shape: 'line', length: 1.5, radius: 1, angle: 45 },
      exit: { shape: 'line', length: .8, radius: 1, angle: 40 }, closed_only: true, side: 'auto', overrides: [] };
    async function open(parts) {
      const scene = manifest.scenes.find(scene => scene.parts === parts);
      await api.openPart(scene.id);
      ui.tab = 'setup'; ui.setupPanel = 'leads'; ui.modal = null; ui.snap = false;
      await prepared();
      if (draft().placed.length > scene.contours) { await api.remove(draft().placed.flatMap((_, i) => i >= scene.contours ? [i] : [])); await prepared(); }
      await api.setRecipe(manifest.recipe); await prepared();
      await api.setFeatures({ ...manifest.features, leads: settings }); await prepared();
      await api.transform(draft().placed.map((_, i) => i), null); await prepared();
      await clickTitle('Show the bed'); await select(0);
    }
    await open(1);
    await drag('entry drag changes one of three contours');
    await drag('repeat entry drag keeps the same owner');
    check(draft().features.leads.overrides.length === 1, 'Repeated edits use one override');
    await drag('exit drag changes only its own role', 1);
    const edited = snapshot();
    await measure('undo and redo restore exact local geometry', async () => {
      await api.undo(); await prepared(); await api.redo(); await prepared();
      check(same(edited, snapshot()), 'Redo restores the edited geometry');
    });
    await measure('rotation keeps the individual definition', async () => {
      const local = JSON.parse(JSON.stringify(draft().features.leads.overrides));
      const revision = draft().revision;
      await clickTitle('Turn the selection a quarter turn'); await wait(() => draft().revision > revision); await prepared();
      check(same(local, draft().features.leads.overrides), 'Rotation retains local settings');
    });
    await drag('drag after rotation stays local');
    const source = draft().features.leads.overrides[0];
    await measure('copy and paste retain an independent local lead', async () => {
      await clickTitle('Copy the selection');
      const revision = draft().revision;
      await clickTitle('Paste the copied shapes'); await wait(() => draft().revision > revision); await prepared();
      const copies = draft().features.leads.overrides;
      check(copies.length === 2 && copies[0].location.contour !== copies[1].location.contour, 'Copy owns its own override');
      check(same(copies[0].entry, copies[1].entry) && same(copies[0].exit, copies[1].exit), 'Copy inherits edited definitions');
    });
    await drag('editing the pasted lead leaves the original unchanged');
    check(same(source, draft().features.leads.overrides[0]), 'Original stays unchanged after copy edit');
    await measure('return selected copy to job defaults', async () => {
      const revision = draft().revision;
      [...document.querySelectorAll('button')].find(b => b.textContent === 'Use job defaults for selection').click();
      await wait(() => draft().revision > revision); await prepared();
      check(draft().features.leads.overrides.length === 1 && same(source, draft().features.leads.overrides[0]), 'Reset removes only selected copy edits');
    });
    await measure('save and reopen retain local settings', async () => {
      const edited = JSON.parse(JSON.stringify(draft().features.leads));
      const saved = await api.saveJob('Individual lead QA'); await prepared();
      await api.openJob(saved.id); await prepared();
      check(same(edited, draft().features.leads), 'Reopened job retains local leads');
    });
    await measure('select two of three parts', async () => {
      const copy = draft().placed.slice(0, 3).map(p => ({ source: p.source, transform: [...p.transform.slice(0, 4), p.transform[4] + 100, p.transform[5] + 60] }));
      await api.add(copy); await prepared(); await clickTitle('Show the bed');
      check(draft().groups.length === 3, 'Three independently selectable parts');
      await select(0); await select(1, true);
      check(same(selectedGroups(), [0, 1]), 'Two selected parts and one unselected part');
    });
    const rightHandle = draft().preview.contours.filter(c => c.sources.some(source => source < 6)).findIndex(c => c.sources.includes(2)) * 2;
    check(rightHandle >= 0, 'Right hole has a handle');
    await drag('two selected parts update only their right-hole entries', rightHandle);
    await drag('two selected parts update only their right-hole exits', rightHandle + 1);
    await measure('undo and redo a selection drag in one step', async () => {
      const after = snapshot(), features = JSON.parse(JSON.stringify(draft().features));
      await api.undo(); await prepared();
      check(!same(after, snapshot()), 'Undo restores the previous selection leads');
      await api.redo(); await prepared();
      check(same(after, snapshot()) && same(features, draft().features), 'Redo restores every selected lead');
    });
    await measure('group controls combine whole selected parts', async () => {
      await clickTitle('Group the selected shapes'); await prepared();
      check(same(draft().groups, [[0,1,2,3,4,5],[6,7,8]]), 'Two complete parts form one group');
      check(same(selectedGroups(), [0]), 'Selection follows the new group');
    });
    await drag('grouped assembly still edits only corresponding contours');
    await measure('partial marquee selects the entire assembly', async () => {
      ui.setupPanel = null; key('Escape'); await clickTitle('Show the bed');
      const shape = svg().querySelector('[data-group="0"] .hit'), box = shape.getBoundingClientRect();
      const start = { x: box.left - 15, y: box.top + box.height / 2 - 3 };
      const end = { x: box.left + 3, y: start.y + 6 };
      pointer(svg(), 'pointerdown', start); pointer(svg(), 'pointermove', end); pointer(svg(), 'pointerup', end); await frames();
      check(same(selectedGroups(), [0]), 'Touching a fraction of one member selects the whole assembly');
      ui.setupPanel = 'leads'; await frames();
    });
    await measure('paste preserves an assembly containing repeated source contours', async () => {
      await clickTitle('Copy the selection'); await clickTitle('Paste the copied shapes'); await prepared();
      check(draft().groups.at(-1).length === 6, 'Pasted assembly stays grouped');
      const instances = draft().placed.map(p => `${p.source}:${p.copy}`);
      check(new Set(instances).size === instances.length, 'Pasted instances have unique identities');
      const groups = JSON.parse(JSON.stringify(draft().groups));
      const saved = await api.saveJob('Grouped assembly QA'); await prepared();
      await api.openJob(saved.id); await prepared();
      check(same(draft().groups, groups), 'Assembly grouping survives save and reopen');
      await api.undo(); await prepared();
      check(draft().placed.length === 9, 'One undo removes the whole paste');
      await select(0);
    });
    await measure('ungroup separates members and undo restores the assembly', async () => {
      await clickTitle('Ungroup the selected shapes'); await prepared();
      check(draft().groups.length === 7, 'Ungroup exposes six independent contours plus the unselected part');
      check(selectedGroups().length === 6, 'All former group members remain selected');
      await api.undo(); await prepared();
      check(draft().groups.length === 2 && draft().groups[0].length === 6, 'Undo restores the assembly');
      await api.redo(); await prepared();
      check(draft().groups.length === 7, 'Redo restores independent selection');
      await select(0);
    });
    await drag('single contour stays local after ungrouping');
    await open(1666);
    await drag('4998-contour entry drag changes exactly one contour');
    await drag('4998-contour exit drag changes exactly one contour', 1);
    await clickTitle('Show the bed'); key('a', { ctrlKey: true }); await frames();
    check(selectedGroups().length === 1666, 'Select All selects the complete drawing');
    await drag('Select All updates 1666 corresponding entries in one edit');
    await drag('Select All updates 1666 corresponding exits in one edit', 1);
    await measure('save and reopen the complete selection edit', async () => {
      const expected = JSON.parse(JSON.stringify(draft().features.leads));
      const saved = await api.saveJob('Selected leads QA'); await prepared();
      await api.openJob(saved.id); await prepared();
      check(same(expected, draft().features.leads), 'All selection overrides survive reopening');
    });
    const final = { features: JSON.parse(JSON.stringify(draft().features)), contours: draft().preview.contours.length };
    result.final = final;
  } catch (error) { result.errors.push(String(error.stack ?? error)); }
  window.removeEventListener('error', onError);
  result.passed = !result.errors.length;
  label.textContent = `Lead selection ${result.passed ? 'passed' : 'failed'} · ${result.checks.length} checks`;
  await fetch('/bench/result', { method: 'POST', body: JSON.stringify(result) });
  button.disabled = false;
});
