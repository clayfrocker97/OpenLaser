// Opt-in component acceptance. Mutates fixture parts on a loopback simulator.
import { tick } from 'svelte';
import { api } from '../src/api/client';
import { server } from '../src/stores/server.svelte';
import { ui } from '../src/stores/ui.svelte';

const panel = document.createElement('aside');
panel.style.cssText = 'position:fixed;top:6px;right:6px;z-index:10000;padding:10px;background:#102730;color:white;font:12px Inter';
const label = document.createElement('div'), button = document.createElement('button');
label.textContent = 'Vector import acceptance'; button.textContent = 'Verify text and imports';
panel.append(label, button); document.body.append(panel);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const frames = async () => { await tick(); await new Promise(requestAnimationFrame); };
const wait = async test => { const start = performance.now(); while (!await test()) { if (performance.now() - start > 45000) throw Error('Timed out waiting for UI'); await delay(20); } await frames(); };
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const buttons = root => [...(root ?? document).querySelectorAll('button')];
const click = async (name, root) => { const b = buttons(root).find(b => b.textContent.trim() === name); check(b && !b.disabled, 'Available button: ' + name); b.click(); await frames(); };
const dialog = () => document.querySelector('[role="dialog"]');
const text = async value => { const input = dialog().querySelector('textarea'); input.value = value; input.dispatchEvent(new Event('input', { bubbles: true })); await frames(); };
const readyText = () => !document.querySelector('.text-preview[aria-busy="true"]') && dialog()?.querySelector('svg[aria-label="Cuttable text outlines"]');
const addButton = () => buttons(dialog()).find(b => b.textContent.trim() === 'Add text');
const stamp = Date.now();
const svg = body => '<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="50mm" viewBox="0 0 80 50">' + body + '</svg>';
const dxf = body => '0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n' + body + '0\nENDSEC\n0\nEOF\n';
const fixtures = [
  { name: 'open-' + stamp + '.dxf', body: dxf('0\nLINE\n10\n0\n20\n0\n11\n30\n21\n0\n0\nLWPOLYLINE\n90\n3\n70\n0\n10\n0\n20\n10\n10\n15\n20\n20\n10\n30\n20\n10\n'), open: 2 },
  { name: 'curves-' + stamp + '.SVG', body: svg('<g fill="none" stroke="black"><path d="M2 2Q15 25 28 2 M35 5C40 30 60 30 65 5"/><polyline points="2,35 12,45 22,35"/></g>'), open: 3 },
  { name: 'svg-text-' + stamp + '.svg', body: svg('<text x="2" y="24" font-family="Noto Sans" font-size="18">BO</text><path d="M2 35L30 35" stroke="black"/>'), open: 1 },
  { name: 'dxf-text-' + stamp + '.dxf', body: dxf('0\nTEXT\n10\n0\n20\n10\n40\n12\n50\n15\n1\nBO\n0\nMTEXT\n10\n45\n20\n30\n40\n6\n71\n1\n1\nH\\PH\n0\nLINE\n10\n0\n20\n0\n11\n30\n21\n0\n'), open: 1 },
];
const upload = async (files, folder = false) => {
  const input = document.querySelector(folder ? 'input[webkitdirectory]' : '.import-parts input[type="file"]:not([webkitdirectory])');
  check(input && input.accept === '.dxf,.svg', 'DXF and SVG file picker');
  const transfer = new DataTransfer();
  for (const file of files) transfer.items.add(new File([file.body], file.name));
  input.files = transfer.files; input.dispatchEvent(new Event('change', { bubbles: true })); await frames();
  await wait(() => !buttons().some(b => b.textContent.trim() === 'Stop importing'));
};
const closeNotice = async title => {
  if (dialog()?.querySelector('h2')?.textContent === title) { await click('Close', dialog()); await wait(() => !dialog()); }
};

button.addEventListener('click', async () => {
  button.disabled = true;
  const result = { variant: 'imports', protocol: 1, checks: [], errors: [] };
  const errors = event => result.errors.push(event.message ?? String(event.reason));
  window.addEventListener('error', errors); window.addEventListener('unhandledrejection', errors);
  const measure = async (name, run) => { label.textContent = name; const detail = await run(); result.checks.push({ name, passed: true, ...detail }); };
  try {
    await wait(() => server.doc?.library && server.link);
    const recipe = server.doc.library.recipes.find(r => r.name.includes('Basswood') && r.name.includes('High Air')) ?? server.doc.library.recipes[0];
    check(recipe, 'A fixture material exists');
    ui.tab = 'parts'; await frames();
    await measure('Mixed DXF and SVG files use the production file picker', async () => {
      await upload(fixtures);
      await wait(() => fixtures.every(f => server.doc.library.parts.some(p => p.file_name === f.name)));
      check(dialog()?.querySelector('h2')?.textContent === 'Import notes', 'DXF font substitution is visible');
      check(dialog().textContent.includes('Noto Sans'), 'Replacement font named');
      await closeNotice('Import notes');
      return { files: fixtures.map(f => f.name) };
    });
    await measure('Every fixture prepares and compiles with its open contours intact', async () => {
      const compiled = [];
      for (const fixture of fixtures) {
        const part = server.doc.library.parts.find(p => p.file_name === fixture.name);
        await api.openPart(part.id); await api.setRecipe(recipe.id);
        await wait(() => server.doc.draft?.part === part.id && server.doc.draft.preview && !server.doc.draft.preparing);
        const preview = server.doc.draft.preview;
        check(preview.contours.filter(c => !c.closed).length === fixture.open, 'Open contours retained for ' + fixture.name);
        await api.compile(false);
        await wait(() => server.doc.draft.compiled && !server.doc.draft.error);
        compiled.push({ file: fixture.name, open: fixture.open, contours: preview.contours.length, seconds: server.doc.draft.compiled.seconds });
      }
      return { compiled };
    });
    ui.tab = 'parts'; await frames();
    await measure('Folder import deduplicates existing files', async () => {
      const before = server.doc.library.parts.length;
      await upload([fixtures[0], fixtures[1], { name: 'readme.txt', body: 'Ignored non-vector' }], true);
      check(!dialog(), 'No unexpected folder failure');
      check(server.doc.library.parts.length === before, 'Identical imports reuse parts');
    });
    await measure('Malformed SVG and missing glyphs fail visibly without partial parts', async () => {
      const before = server.doc.library.parts.length;
      await upload([{name:'invalid.svg', body:svg('<path d="M0 0L10 0"/><path d="M0 0L5 X"/>')}, {name:'missing-glyph.svg',body:svg('<text y="20">A🚧B</text>')}]);
      check(dialog()?.querySelector('h2')?.textContent === 'Files that did not import', 'Failure dialog');
      check(dialog().textContent.includes('invalid.svg') && dialog().textContent.includes('missing-glyph.svg'), 'Every failure named');
      check(server.doc.library.parts.length === before, 'No partial imports');
      await closeNotice('Files that did not import');
    });
    await measure('Text editor fonts, weight, alignment, empty input and missing glyphs', async () => {
      await click('+ Text'); await wait(readyText);
      check(!dialog().querySelector('select'), 'All font choices are visible');
      for (const font of ['Serif', 'Mono', 'Sans']) { await click(font, dialog()); await wait(readyText); }
      await click('Bold', dialog()); await wait(readyText);
      await text('B & <O>\nCafé'); await wait(readyText);
      for (const alignment of ['Right', 'Center', 'Left']) { await click(alignment, dialog()); await wait(readyText); }
      await text(''); check(addButton().disabled, 'Empty text cannot be added');
      await text('A🚧B'); await wait(() => dialog()?.querySelector('[role="alert"]'));
      check(addButton().disabled && dialog().textContent.includes('cannot draw'), 'Missing characters are errors');
      await text('B & <O>\nCafé'); await wait(readyText);
      check(dialog().querySelector('path').getAttribute('fill-rule') === 'evenodd', 'Holes are previewed');
      return { preview: dialog().querySelector('.text-dimensions').textContent };
    });
    await measure('Adding multiline text creates a part and compiles it', async () => {
      await click('Add text', dialog()); await wait(() => !dialog() && ui.tab === 'setup');
      await api.setRecipe(recipe.id); await wait(() => server.doc.draft?.preview && !server.doc.draft.preparing);
      check(server.doc.draft.preview.contours.every(c => c.closed), 'Text outlines are closed');
      await api.compile(false); await wait(() => server.doc.draft.compiled && !server.doc.draft.error);
      return { part: server.doc.draft.part, contours: server.doc.draft.preview.contours.length };
    });
    check(!result.errors.length, 'No browser errors: ' + result.errors.join('; '));
    result.passed = true; label.textContent = result.checks.length + ' import checks passed';
  } catch (error) { result.passed = false; result.error = String(error?.stack ?? error); label.textContent = 'Failed: ' + error; }
  finally {
    window.removeEventListener('error', errors); window.removeEventListener('unhandledrejection', errors);
    await fetch('/bench/result', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(result) });
  }
});
