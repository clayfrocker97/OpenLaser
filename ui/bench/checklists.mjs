// Opt-in production component checks. Use only with a separate loopback simulator.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';
import { settingsEdits } from '../src/lib/settings-edits.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:6px;right:6px;z-index:10000;padding:10px;background:#102730;color:white;font:12px Inter';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Checklist acceptance'; button.textContent = 'Verify checklists';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!await test()) { if (performance.now() - start > 45000) throw Error('Timed out waiting for UI'); await delay(20); } await frames(); };
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const buttons = root => [...(root ?? document).querySelectorAll('button')];
// A held control's label, without its "Hold" mark.
const label = b => [...b.childNodes].filter(n => !n.classList?.contains('hold-hint')).map(n => n.textContent).join('').trim();
// Machine actions act only after a held press (ui/DESIGN.md, Touch rules).
const hold = async (name, root) => {
  const b = buttons(root).find(b => label(b) === name); check(b && !b.disabled, `Available button: ${name}`);
  const box = b.getBoundingClientRect();
  const at = { bubbles: true, pointerId: 1, button: 0, clientX: box.x + box.width / 2, clientY: box.y + box.height / 2 };
  b.dispatchEvent(new PointerEvent('pointerdown', at));
  await new Promise(resolve => setTimeout(resolve, 1100));
  b.dispatchEvent(new PointerEvent('pointerup', at)); await frames();
};
const click = async (name, root) => { const b = buttons(root).find(b => b.textContent.trim() === name); check(b && !b.disabled, `Available button: ${name}`); b.click(); await frames(); };
const dialog = () => document.querySelector('[role="dialog"]');
const text = (input, value) => { input.value = value; input.dispatchEvent(new Event('input', { bubbles: true })); };
const close = async kind => {
  if (kind === 'footer') await click('Close', dialog());
  else if (kind === 'backdrop') {
    // A dialog closes only on a tap that starts and ends on the backdrop.
    const backdrop = document.querySelector('.modal-backdrop');
    for (const type of ['pointerdown', 'pointerup']) backdrop.dispatchEvent(new PointerEvent(type, { bubbles: true }));
  }
  else dialog().querySelector('button[aria-label="Close"]').click();
  await wait(() => !dialog());
};

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'checklists', protocol: 1, checks: [], errors: [] };
  const errors = event => result.errors.push(event.message ?? String(event.reason));
  window.addEventListener('error', errors); window.addEventListener('unhandledrejection', errors);
  const writes = [], originalFetch = window.fetch;
  window.fetch = (...args) => { if (args[1]?.method && args[1].method !== 'GET') writes.push(String(args[0])); return originalFetch(...args); };
  const measure = async (name, run) => { label.textContent = name; const detail = await run(); result.checks.push({ name, passed: true, ...detail }); };
  try {
    await wait(() => server.doc && server.link);
    await settingsEdits.ready;
    check(!settingsEdits.pending.length, 'Use an isolated browser origin without pending edits');
    ui.tab = 'machine'; ui.machinePage = 0;
    await wait(() => document.querySelector('.xml-row'));
    await measure('Machine files and XML categories', async () => {
      check(document.querySelector('.file-import summary')?.textContent === 'Import machine files', 'Import is in machine settings');
      check(![...document.querySelectorAll('.xml-nav button')].some(b => /layer/i.test(b.textContent)), 'Layer categories are absent');
      check(!document.querySelector('select'), 'Choices remain visible without dropdowns');
      check(!buttons().some(b => /^(Preflight|Postflight) defaults$/.test(b.textContent.trim())), 'Checklist controls moved out of machine settings');
      await click('Checklists');
      check(buttons().some(b => b.textContent.trim() === 'Preflight defaults') && buttons().some(b => b.textContent.trim() === 'Postflight defaults'), 'Both editors are on the Checklists tab');
      return { import_in_settings: true, dedicated_checklists_tab: true };
    });
    const original = await api.preflightPreferences();
    for (const kind of ['footer', 'x', 'backdrop']) {
      await measure(`Preflight unsaved close: ${kind}`, async () => {
        const count = writes.length;
        await click('Preflight defaults');
        await wait(() => dialog()?.querySelector('textarea'));
        text(dialog().querySelector('textarea'), `Unsaved ${kind}`);
        await frames(); await close(kind);
        check(!settingsEdits.entries.preflight, 'Closing does not stage changes');
        check(writes.length === count, 'Closing makes no writes');
        check(same(await api.preflightPreferences(), original), 'Saved defaults are unchanged');
      });
    }
    await measure('Explicit review and reopening use cloneable checklist drafts', async () => {
      await click('Preflight defaults'); await wait(() => dialog()?.querySelector('textarea'));
      await click('Fiber', dialog());
      text(dialog().querySelector('textarea'), 'First staged check'); await frames();
      await click('Review changes', dialog());
      await wait(() => dialog()?.querySelector('h2')?.textContent === 'Pending changes');
      check(settingsEdits.entries.preflight.value.fiber.steps[0].text === 'First staged check', 'First edit staged');
      await close('x');
      await click('Preflight defaults'); await wait(() => dialog()?.querySelector('textarea'));
      await click('Fiber', dialog());
      check(dialog().querySelector('textarea').value === 'First staged check', 'Staged value reopened');
      text(dialog().querySelector('textarea'), 'Second staged check'); await frames();
      await click('Review changes', dialog());
      await wait(() => settingsEdits.entries.preflight?.value.fiber.steps[0].text === 'Second staged check');
      check(!dialog().querySelector('[role="alert"]'), 'No structuredClone error');
      await close('x'); await settingsEdits.discard('preflight');
    });
    await measure('Postflight shares the editor and offers bed or XY actions', async () => {
      await click('Postflight defaults'); await wait(() => dialog()?.querySelector('textarea'));
      dialog().querySelector('details').open = true; await frames();
      const choices = [...dialog().querySelector('details').querySelectorAll('.action-fields .choices:first-child button')].map(b => b.textContent);
      check(same(choices, ['No action', 'Move to bed point', 'Move to XY']), 'Postflight action list');
      await click('Move to XY', dialog());
      check(dialog().querySelectorAll('input[type="number"]').length === 2, 'Editable XY fields');
      await close('footer');
      check(!settingsEdits.entries.preflight, 'Unsaved postflight closes without staging');
    });
    await measure('One font, consistent weights, no edge highlights', async () => {
      const fonts = new Set(), weights = new Set(), edges = [];
      for (const tab of ['parts', 'setup', 'run', 'materials', 'machine']) {
        ui.tab = tab; await frames();
        for (const el of document.querySelectorAll('#app *')) {
          if (!el.getClientRects().length) continue;
          const s = getComputedStyle(el);
          if ([...el.childNodes].some(n => n.nodeType === Node.TEXT_NODE && n.textContent.trim())) { fonts.add(s.fontFamily); weights.add(s.fontWeight); }
          if (/inset/.test(s.boxShadow) && /\b[34]px 0px/.test(s.boxShadow)) edges.push(el.className);
        }
      }
      check([...fonts].every(f => f.startsWith('Inter')), `Font families: ${[...fonts]}`);
      check([...weights].every(w => ['400', '600', '700'].includes(w)), `Weights: ${[...weights]}`);
      check(!edges.length, `Edge highlights: ${edges}`);
      return { fonts: [...fonts], weights: [...weights] };
    });
    // A tiny real compiled job completes on the isolated simulated controller.
    await measure('Real completion opens the shared postflight checklist', async () => {
      const drawing = ['0','SECTION','2','ENTITIES','0','LWPOLYLINE','90','4','70','1','10','0','20','0','10','4','20','0','10','4','20','4','10','0','20','4','0','ENDSEC','0','EOF'].join('\n');
      const part = await api.importPart('checklist-qa.dxf', new TextEncoder().encode(drawing).buffer);
      const recipe = await api.addRecipe({ name: 'Checklist QA', laser: 'fiber', thickness_mm: 1, values: { bank: 1 }, gas: null });
      await api.openPart(part.id); await api.setRecipe(recipe.id);
      await wait(() => server.doc.draft?.preview);
      await api.compile(false);
      await wait(() => server.doc.mode === 'fiber');
      if (server.doc.machine.connection.state !== 'connected') await api.machine('connect');
      await wait(async () => { const state = await api.state(); return state.machine.connection.state === 'connected' && state.bindings && state.readiness.home.ok && state.link.phase === 'idle' && !state.machine.operation; });
      await api.machine('home');
      // On reruns the previous SSE document can already say "homed". Read the
      // admitted operation's state before proceeding to a new machine action.
      await wait(async () => { const state = await api.state(); return state.machine.session.homed && !state.machine.operation && state.link.phase === 'idle'; });
      await api.setOrigin([10, 10]); await api.compile(false);
      const pref = await api.preflightPreferences();
      pref.postflight.fiber.steps = [{ text: 'Park the head', action: { kind: 'move_xy', x: 5, y: 5 }, auto_check: true }, { text: 'Inspect the parts', action: null, auto_check: false }];
      await api.savePreflightPreferences(pref);
      const review = await api.preflight('run');
      await api.machine('run', { preflight: { token: review.token, checked: review.steps.map((_, i) => i), gas_ready: true } });
      await wait(() => dialog()?.querySelector('h2')?.textContent === 'Postflight');
      check(dialog().querySelector('.preflight-progress strong')?.textContent === '0 / 2', 'Postflight progress is numeric');
      await hold('Move to 5, 5', dialog());
      await wait(() => dialog()?.textContent.includes('Auto-checked'));
      const inputs = dialog().querySelectorAll('.preflight-check input');
      inputs[1].click(); await frames();
      await click('Done', dialog()); await wait(() => !dialog() && !server.doc.postflight);
      check(server.doc.machine.feedback.outputs === 0, 'Outputs remain off after parking');
    });
    ui.tab = 'machine'; ui.machinePage = 0; await frames();
    check(!result.errors.length, `Browser errors: ${result.errors.join('; ')}`);
    result.passed = true; label.textContent = `${result.checks.length} checklist checks passed`;
  } catch (error) { result.passed = false; result.error = String(error?.stack ?? error); label.textContent = `Failed: ${error}`; }
  finally {
    window.fetch = originalFetch;
    window.removeEventListener('error', errors); window.removeEventListener('unhandledrejection', errors);
    await fetch('/bench/result', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(result) });
  }
});
