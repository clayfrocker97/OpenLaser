// Opt-in production UI checks. Run only against a separate loopback simulator.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;bottom:6px;right:6px;z-index:10000;padding:10px;background:#102730;color:white;font:12px Inter';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Copy / paste acceptance'; button.textContent = 'Verify copy / paste';
panel.append(label, button); document.body.append(panel);
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!test()) { if (performance.now() - start > 45000) throw Error('Timed out waiting for UI'); await new Promise(resolve => setTimeout(resolve, 20)); } await frames(); };
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const snapshot = value => JSON.parse(JSON.stringify(value));
const buttons = root => [...(root ?? document).querySelectorAll('button')];
const click = async (name, root) => { const b = buttons(root).find(b => b.textContent.trim() === name); check(b && !b.disabled, `Available button: ${name}`); b.click(); await frames(); };
const draft = () => server.doc.draft;
const prepared = async () => { await wait(() => draft()?.preview && draft().error !== 'preparing geometry'); check(!draft().error, draft().error); };
const key = (key, options = {}) => document.querySelector('svg[aria-label="Drawing"]').dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...options }));
const side = () => document.querySelector('.panel.side');

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'copy-paste', protocol: 1, checks: [], errors: [] };
  const originalFetch = window.fetch, writes = [], batches = [];
  let refuseNextPaste = false;
  window.fetch = (...args) => {
    const path = String(args[0]);
    if (args[1]?.method === 'POST') writes.push(path);
    if (path.startsWith('/api/draft/add?')) {
      batches.push(JSON.parse(args[1].body));
      if (refuseNextPaste) { refuseNextPaste = false; return Promise.resolve(new Response(JSON.stringify({ error: 'Simulated stale drawing' }), { status: 409 })); }
    }
    return originalFetch(...args);
  };
  const errors = event => result.errors.push(event.message ?? String(event.reason));
  window.addEventListener('error', errors); window.addEventListener('unhandledrejection', errors);
  const measure = async (name, run) => { label.textContent = name; result.checks.push({ name, passed: true, ...await run() }); };
  try {
    await wait(() => server.doc && server.link);
    const rectangle = (x, y, width, height) => ['0','LWPOLYLINE','90','4','70','1', ...[[x,y],[x+width,y],[x+width,y+height],[x,y+height]].flatMap(([px,py]) => ['10',px,'20',py])];
    // A fresh drawing keeps reruns independent of previously retained QA drafts.
    const width = 8 + (server.doc.library.parts.length + 1) / 100;
    const drawing = ['0','SECTION','2','ENTITIES', ...rectangle(0,0,20,10), ...rectangle(5,3,3,3), ...rectangle(30,0,width,10), '0','ENDSEC','0','EOF'].join('\n');
    const part = await api.importPart('copy-paste-qa.dxf', new TextEncoder().encode(drawing).buffer);
    await api.openPart(part.id); await prepared();
    const features = snapshot(draft().features);
    features.leads = { entry: { shape: 'line', length: 1, radius: 1, angle: 45 }, exit: null, closed_only: true, side: 'auto',
      overrides: [{ location: { contour: 1, fraction: 0 }, entry: { shape: 'line', length: 2, radius: 1, angle: 60 }, exit: null }] };
    await api.setFeatures(features); await prepared();
    ui.tab = 'setup'; ui.setupPanel = null; await frames();
    const original = snapshot(draft()), originalGroups = original.groups.length;
    await measure('Copy opens a normal sidebar with count, gap and direction', async () => {
      key('a', { ctrlKey: true }); await frames();
      document.querySelector('button[title="Copy the selection"]').click(); await frames();
      check(side().querySelector('h2').textContent === 'Copy / paste', 'Copy opens sidebar');
      check(side().querySelector('[aria-label="Number of copies"]'), 'Editable copy count');
      check(side().querySelector('[aria-label="Gap between copies"]'), 'Editable spacing');
      check(!side().querySelector('select'), 'Visible directions without dropdowns');
      check(!side().querySelector('[aria-label="Copy count presets"]'), 'Only one copy-count field');
      side().querySelector('[aria-label="Number of copies"]').click(); await frames();
      await click('5', document.querySelector('.osk')); await click('Done', document.querySelector('.osk'));
      check(buttons(side()).some(b => b.textContent === 'Paste 5 copies'), 'Batch button reflects count');
    });
    await measure('One click pastes five complete selections in one request and undo step', async () => {
      const beforeRequests = batches.length, revision = draft().revision;
      const paste = buttons(side()).find(b => b.textContent === 'Paste 5 copies');
      paste.click(); paste.click(); // A quick repeat is blocked while the first request is pending.
      await wait(() => draft().revision > revision); await prepared();
      check(batches.length === beforeRequests + 1, 'One request despite repeated click');
      check(draft().placed.length === original.placed.length * 6, 'All five copies arrived');
      check(draft().past === original.past + 1, 'One undo step');
      check(draft().groups.length === originalGroups * 6, 'Groups preserved per copy');
      check(draft().features.leads.overrides.length === 6, 'Local lead definitions duplicated');
      check(same(draft().placed.slice(0, 3), original.placed), 'Original placements retained');
      for (let i = 0; i < 5; i++) {
        const first = 3 + i * 3;
        check(draft().groups.some(g => same(g, [first, first + 1])), 'Outer and hole remain grouped');
        check(draft().features.leads.overrides.some(lead => lead.location.contour === first + 1 && lead.entry.length === 2), 'Local lead targets its own copied hole');
      }
      let edited = draft().revision;
      await click('Undo'); await wait(() => draft().revision > edited); await prepared();
      check(same(draft().placed, original.placed) && same(draft().features.leads, original.features.leads), 'Undo restores originals and local settings');
      edited = draft().revision;
      await click('Redo'); await wait(() => draft().revision > edited); await prepared();
      check(draft().placed.length === 18 && draft().groups.length === originalGroups * 6, 'Redo restores the full batch');
      return { copies: 5, contours: 18, groups: draft().groups.length };
    });
    await measure('Custom count and gap use the numpad; failed pastes do not advance placement', async () => {
      side().querySelector('[aria-label="Number of copies"]').click(); await frames();
      const numpad = document.querySelector('.osk');
      check(numpad, 'Copy count opens numpad');
      await click('3', numpad); await click('Done', numpad);
      check(buttons(side()).some(b => b.textContent === 'Paste 3 copies'), 'Custom count applied');
      side().querySelector('[aria-label="Gap between copies"]').click(); await frames();
      await click('4', document.querySelector('.osk')); await click('Done', document.querySelector('.osk'));
      await click('↑ Up', side());
      refuseNextPaste = true;
      const revision = draft().revision, placed = draft().placed.length;
      await click('Paste 3 copies', side());
      await wait(() => ui.toast?.text === 'Simulated stale drawing');
      check(draft().revision === revision && draft().placed.length === placed, 'Refused paste leaves draft intact');
      const refused = snapshot(batches.at(-1));
      await click('Paste 3 copies', side());
      await wait(() => draft().revision > revision); await prepared();
      check(same(batches.at(-1), refused), 'Retry uses the same placement');
      check(draft().placed.length === placed + 9, 'Retry creates exactly three copies');
      const count = draft().placed.length, nextRevision = draft().revision;
      key('v', { ctrlKey: true }); await wait(() => draft().revision > nextRevision); await prepared();
      check(draft().placed.length === count + 3, 'Keyboard paste still adds one complete selection');
    });
    await measure('Z/W and Slow/Fast share button and text styling in both themes', async () => {
      ui.tab = 'run'; await frames();
      const oldTheme = ui.theme;
      for (const theme of ['light', 'dark']) {
        ui.previewTheme(theme); await frames();
        const speed = document.querySelector('.jog .hub'), axis = document.querySelector('.zcol .hub');
        for (const selector of [null, 'strong', 'small']) {
          const a = getComputedStyle(selector ? speed.querySelector(selector) : speed);
          const b = getComputedStyle(selector ? axis.querySelector(selector) : axis);
          for (const property of ['backgroundColor', 'borderRadius', 'borderColor', 'fontFamily', 'fontSize', 'fontWeight', 'color']) check(a[property] === b[property], `Matching ${selector ?? 'button'} ${property} in ${theme}`);
        }
        check(speed.offsetHeight === axis.offsetHeight, 'Same full button height');
        const before = writes.length;
        check(axis.textContent.includes('Tap for W'), 'Visible hint names the next axis');
        axis.click(); await frames(); check(axis.querySelector('strong').textContent === 'W' && axis.textContent.includes('Tap for Z'), 'Switches to W and updates the hint');
        axis.click(); await frames(); check(axis.querySelector('strong').textContent === 'Z' && axis.textContent.includes('Tap for W'), 'Switches back to Z and updates the hint');
        check(writes.length === before, 'Selecting an axis causes no machine motion');
      }
      ui.previewTheme(oldTheme);
    });
    await measure('Nesting selects all parts without changing the layout', async () => {
      ui.tab = 'setup'; ui.setupPanel = 'nest'; await frames();
      key('Escape'); await frames();
      const before = draft().revision, count = draft().groups.length;
      await click('Select all parts', side());
      check(side().querySelector('.nest-count').textContent.includes(`${count} selected`), 'All parts selected');
      check(draft().revision === before, 'Selection does not change the layout');
    });
    check(!result.errors.length, `Browser errors: ${result.errors}`);
    result.passed = true; label.textContent = `${result.checks.length} copy / paste checks passed`;
  } catch (error) { result.passed = false; result.error = String(error?.stack ?? error); label.textContent = `Failed: ${error}`; }
  finally {
    window.fetch = originalFetch;
    window.removeEventListener('error', errors); window.removeEventListener('unhandledrejection', errors);
    await fetch('/bench/result', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(result) });
  }
});
