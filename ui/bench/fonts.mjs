// Opt-in font acceptance through production controls on the current simulator.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';
import italic from '../../fixtures/fonts/LobsterTwo-Italic.ttf?url';
import bold from '../../fixtures/fonts/LobsterTwo-Bold.ttf?url';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:6px;right:6px;z-index:10000;padding:10px;background:#102730;color:white;font:12px Inter';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Font import acceptance'; button.textContent = 'Verify font import';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!await test()) { if (performance.now() - start > 45000) throw Error('Timed out waiting for UI'); await delay(20); } await frames(); };
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const dialog = () => document.querySelector('[role="dialog"]');
const buttons = root => [...(root ?? document).querySelectorAll('button')];
const click = async name => { const b = buttons().find(b => b.textContent.trim() === name); check(b && !b.disabled, 'Available button: ' + name); b.click(); await frames(); };
const ready = () => !document.querySelector('.text-preview[aria-busy="true"]') && dialog()?.querySelector('svg[aria-label="Cuttable text outlines"]');
const outlines = () => dialog().querySelector('.text-preview path').getAttribute('d');
const fontButtons = () => [...dialog().querySelectorAll('.font-list button')];
const upload = async files => {
  const input = dialog().querySelector('input[aria-label="Import font files"]');
  check(input?.accept === '.ttf,.otf,.ttc,.otc', 'Font file picker accepts supported formats');
  const transfer = new DataTransfer();
  for (const file of files) transfer.items.add(new File([file.bytes], file.name));
  input.files = transfer.files; input.dispatchEvent(new Event('change', { bubbles: true })); await frames();
  await wait(() => !buttons().some(b => b.textContent.trim() === 'Importing…'));
  await wait(ready);
};

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'fonts', checks: [], errors: [] };
  const errors = event => result.errors.push(event.message ?? String(event.reason));
  window.addEventListener('error', errors); window.addEventListener('unhandledrejection', errors);
  const measure = async (name, run) => { label.textContent = name; const detail = await run(); result.checks.push({ name, passed: true, ...detail }); };
  try {
    await wait(() => server.doc?.library && server.link);
    ui.tab = 'parts'; await frames();
    await click('+ Text'); await wait(ready);
    const input = dialog().querySelector('textarea'); input.value = 'Lobster\nB & <O>'; input.dispatchEvent(new Event('input', { bubbles: true })); await frames(); await wait(ready);
    const prior = outlines();
    const files = await Promise.all([{ name: 'LobsterTwo-Italic.ttf', url: italic }, { name: 'LobsterTwo-Bold.ttf', url: bold }].map(async file => ({ name: file.name, bytes: await (await fetch(file.url)).arrayBuffer() })));
    await measure('Import multiple fonts and preview the selected italic face', async () => {
      await upload(files);
      check(!dialog().querySelector('select'), 'Font choices are visible buttons');
      check(fontButtons().length >= 2, 'Imported faces shown');
      check(fontButtons().some(b => b.textContent.includes('italic') && b.getAttribute('aria-pressed') === 'true'), 'First imported face selected');
      check(outlines() !== prior, 'Preview uses actual imported font');
      check(!dialog().querySelector('.text-editor > .warn-text'), 'No false italic substitution warning');
      check(!dialog().querySelector('.text-weight'), 'No synthetic font weight controls');
      return { fonts: (await api.fonts()).fonts, dimensions: dialog().querySelector('.text-dimensions').textContent };
    });
    await measure('Duplicate and malformed font files report their outcomes', async () => {
      const count = (await api.fonts()).fonts.length;
      await upload([{ ...files[0], name: 'renamed.ttf' }, { name: 'invalid.ttf', bytes: 'not a font' }]);
      check(dialog().textContent.includes('already imported'), 'Duplicate is identified');
      check(dialog().querySelector('[role="alert"]')?.textContent.includes('invalid.ttf'), 'Bad file is named');
      check((await api.fonts()).fonts.length === count, 'Invalid and duplicate uploads add no faces');
    });
    await measure('Face and bundled-family changes regenerate their own outlines', async () => {
      const before = outlines();
      const boldButton = fontButtons().find(b => b.textContent.includes('Bold'));
      check(boldButton, 'Bold face available'); boldButton.click(); await frames(); await wait(ready);
      check(before !== outlines(), 'Bold face outlines differ');
      await click('Sans'); await wait(ready);
      check(dialog().querySelector('.text-weight'), 'Bundled font weight choices return');
      const italicButton = fontButtons().find(b => b.textContent.includes('italic'));
      italicButton.click(); await frames(); await wait(ready);
    });
    await measure('Imported font lettering creates a part and compiles', async () => {
      await click('Add text'); await wait(() => !dialog() && ui.tab === 'setup');
      const recipe = server.doc.library.recipes.find(r => r.name.includes('Basswood') && r.name.includes('High Air')) ?? server.doc.library.recipes[0];
      await api.setRecipe(recipe.id); await wait(() => server.doc.draft?.preview && !server.doc.draft.preparing);
      check(server.doc.draft.preview.contours.every(c => c.closed), 'Letter contours remain closed');
      await api.compile(false); await wait(() => server.doc.draft.compiled && !server.doc.draft.error);
      return { part: server.doc.draft.part, contours: server.doc.draft.preview.contours.length, seconds: server.doc.draft.compiled.seconds };
    });
    check(!result.errors.length, 'No browser runtime errors');
    result.passed = true; label.textContent = result.checks.length + ' font checks passed';
  } catch (error) { result.passed = false; result.error = String(error?.stack ?? error); label.textContent = 'Failed: ' + error; }
  finally {
    window.removeEventListener('error', errors); window.removeEventListener('unhandledrejection', errors);
    await fetch('/bench/result', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(result) });
  }
});
