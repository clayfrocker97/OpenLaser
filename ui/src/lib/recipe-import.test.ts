import { describe, expect, it } from 'vitest';
import { choicesFor, duplicatesOf, importRequest, matchMaterial, needsChoice, picked, recipeFiles, reviewItem } from './recipe-import';
import { importQuery } from '../api/client';
import type { RecipePreview, RecipeSummary, RecipeView } from '../api';

const summary: RecipeSummary = { speed: '20', peak: '90', duty: '100', frequency: '5000', gas: '5', pressure: '12', height: '0.8', setup: { nozzle_diameter_mm: null, nozzle: null, focus_mm: null, lens_mm: null }, pierce_stages: 0, smooth_pierce: false };
const recipe = (id: string, name: string, thickness_mm: number, gas = 'N2', laser: 'fiber' | 'co2' = 'fiber'): RecipeView => ({ id, name, laser, thickness_mm, gas, layer: 1, key: id, film: null, summary, attributes: {}, note: '', tags: [], file_name: null, photo: null, favourite: false, updated: 0 });
const preview = (name: string, thickness_mm: number, gas = 'N2', existing: string | null = null): RecipePreview => ({ file_name: `${name}-${thickness_mm}MM_${gas} CUTTING.xml`, name, laser: 'fiber', thickness_mm, gas, layer: 1, tags: [], note: '', setup: { nozzle_diameter_mm: '1.5', nozzle: 'single', focus_mm: '-2', lens_mm: null }, summary, sha256: 'x', existing });
const file = (path: string) => new File(['<ParameterRoot/>'], path.split('/').pop()!);

describe('choosing files', () => {
  it('keeps recipe files and pairs each with the photo of its name in its folder', () => {
    const files = [
      { file: file('SS-2MM_N2 CUTTING.xml'), path: 'lib/steel/SS-2MM_N2 CUTTING.xml' },
      { file: file('SS-2MM_N2 CUTTING.png'), path: 'lib/steel/SS-2MM_N2 CUTTING.png' },
      { file: file('SS-2MM_N2 CUTTING.png'), path: 'lib/other/SS-2MM_N2 CUTTING.png' },
      { file: file('AL-1MM_AIR.XML'), path: 'lib/AL-1MM_AIR.XML' },
      { file: file('readme.txt'), path: 'lib/readme.txt' },
    ];
    const found = recipeFiles(files);
    expect(found.map((f) => f.path)).toEqual(['lib/AL-1MM_AIR.XML', 'lib/steel/SS-2MM_N2 CUTTING.xml']);
    expect(found[0]!.photo).toBeNull();
    expect(found[1]!.photo).toBe(files[1]!.file);
    expect(picked([file('a.xml')])[0]!.path).toBe('a.xml');
  });
});

describe('matching materials', () => {
  const library = [recipe('a', 'Stainless Steel', 1), recipe('b', 'Stainless steel 304', 2), recipe('c', 'Aluminum', 1), recipe('d', 'Basswood', 3, 'High Air', 'co2')];

  it('takes the same name, then the same kind, preferring one with the thickness', () => {
    expect(matchMaterial('stainless steel', 'fiber', 3, library)).toBe('Stainless Steel');
    expect(matchMaterial('SS', 'fiber', 2, library)).toBe('Stainless steel 304');
    expect(matchMaterial('SS', 'fiber', 5, library)).toBe('Stainless Steel');
    expect(matchMaterial('AL', 'fiber', 1, library)).toBe('Aluminum');
    expect(matchMaterial('Basswood', 'fiber', 3, library)).toBeNull();
    expect(matchMaterial('Unobtainium', 'fiber', 3, library)).toBeNull();
  });

  it('names a new material after its card', () => {
    const item = reviewItem('k', file('CS-3MM_O2.xml'), null, preview('CS', 3, 'O2'), library);
    expect(item).toMatchObject({ material: 'Mild steel', isNew: true, thickness_mm: 3 });
    expect(item.setup).toEqual({ nozzle_diameter_mm: '1.5', nozzle: 'single', focus_mm: '-2', lens_mm: null });
  });
});

describe('duplicates', () => {
  const library = [recipe('a', 'Stainless Steel', 2), recipe('b', 'Stainless Steel', 2, 'Air')];

  it('are the same material, thickness and gas, in the library or earlier in the batch', () => {
    const one = reviewItem('1', file('a.xml'), null, preview('SS', 2, 'N2', 'a'), library);
    const two = reviewItem('2', file('b.xml'), null, preview('SS', 3), library);
    const three = reviewItem('3', file('c.xml'), null, preview('SS', 3), library);
    const items = [one, two, three];
    expect(duplicatesOf(one, items, library)).toMatchObject({ identical: true });
    expect(duplicatesOf(one, items, library).recipes.map((r) => r.id)).toEqual(['a']);
    expect(needsChoice(two, items, library)).toBe(false);
    expect(needsChoice(three, items, library)).toBe(true);
    expect(choicesFor(one, items, library)).toEqual(['replace', 'keep', 'skip']);
    expect(choicesFor(three, items, library)).toEqual(['keep', 'skip']);
    two.choice = 'skip';
    expect(needsChoice(three, items, library)).toBe(false);
  });

  it('wait for a choice, then replace, keep both or skip', () => {
    const item = reviewItem('1', file('a.xml'), null, preview('SS', 2), library);
    const items = [item];
    expect(importRequest(item, items, library)).toBeNull();
    item.choice = 'replace';
    expect(importRequest(item, items, library)).toMatchObject({ material: 'Stainless Steel', replace: 'a', keep_both: false, setup: true, nozzle_diameter_mm: '1.5', nozzle: 'single', focus_mm: '-2' });
    item.choice = 'keep';
    expect(importRequest(item, items, library)).toMatchObject({ keep_both: true });
    expect(importRequest(item, items, library)?.replace).toBeUndefined();
    item.choice = 'skip';
    expect(importRequest(item, items, library)).toBeNull();
  });

  it('sends only the options that are set', () => {
    const item = reviewItem('1', file('a.xml'), null, preview('SS', 4), library);
    item.setup.focus_mm = null;
    const request = importRequest(item, [item], library)!;
    expect(importQuery(request)).toBe('name=SS-4MM_N2%20CUTTING.xml&material=Stainless%20Steel&thickness_mm=4&setup=true&nozzle_diameter_mm=1.5&nozzle=single');
  });
});
