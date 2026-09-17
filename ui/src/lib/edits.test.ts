import { describe, expect, it, vi } from 'vitest';
import { RecipeEdits } from './recipe-edits.svelte';
import { FeatureEdits } from './feature-edits.svelte';
import { LatestPick, orderGroups } from './picking';
import { materialsOf } from './materials';
import type { DraftView, Features, PreviewContour, RecipeView } from '../api';

const recipe = (id: string, laser: 'fiber' | 'co2' = 'fiber'): RecipeView => ({ id, laser, name: 'shared name', attributes: { CutPower: '10' }, film: null, thickness_mm: 1, gas: 'Air', photo: null, layer: 1, key: id, summary: { speed: null, power: null, pressure: null, height: null, frequency: null, gas: null }, note: '', tags: [], file_name: null, favourite: false, updated: 0 });
const features = (): Features => ({ common: null, leads: null, joints: null, cooling: null, bridges: null, kerf: { width: 0.2, side: 'auto' }, start: { position: 'keep', direction: 'keep', spots: [] }, seam: 'seal', order: { strategy: 'as_drawn', inner_first: true, circles_first: false, spread_heat: false }, skip_layers: [] });
const drawing = (part = 'part-a', revision = 1, generation = 1): DraftView => ({ part, revision, generation, features: features() } as DraftView);

describe('recipe staging', () => {
  it('acknowledges only submitted revisions, including a return to the old value', () => {
    const edits = new RecipeEdits();
    const a = recipe('a');
    edits.set(a, 'CutPower', '20');
    edits.setFilm(a, 'film-a');
    const submitted = edits.begin(a.id)!;
    edits.set(a, 'CutPower', '10');
    edits.setFilm(a, null);
    edits.finish(submitted, true);
    expect(submitted.change).toEqual({ attributes: { CutPower: '20' }, expected_attributes: { CutPower: '10' }, film: 'film-a', expected_film: null });
    expect(edits.attributes(a.id)).toEqual({ CutPower: '10' });
    expect(edits.film(a.id)).toBeNull();
    expect(edits.count(a.id)).toBe(2);
    expect(edits.conflicts({ ...a, attributes: { CutPower: '20' }, film: 'film-a' })).toEqual([]);
  });

  it('keeps a missing base and exposes independent saved changes for explicit resolution', () => {
    const edits = new RecipeEdits();
    const a = recipe('new-field');
    edits.set(a, 'Delay', '5');
    edits.set({ ...a, attributes: { ...a.attributes, Delay: '8' } }, 'Delay', '7');
    expect(edits.conflicts({ ...a, attributes: { ...a.attributes, Delay: '8' } })).toEqual([{ key: 'Delay', base: 'removed', draft: '7', saved: '8' }]);
    edits.resolve({ ...a, attributes: { ...a.attributes, Delay: '8' } }, 'Delay', true);
    expect(edits.begin(a.id)?.change.expected_attributes).toEqual({ Delay: '8' });
  });

  it('keeps page changes separate and discards only an explicitly deleted identity', () => {
    const edits = new RecipeEdits();
    edits.set(recipe('a'), 'CutPower', '20');
    edits.set(recipe('b'), 'CutPower', '30');
    expect(edits.attributes('a')).toEqual({ CutPower: '20' });
    const save = edits.begin('a')!;
    edits.retain(['b']);
    edits.finish(save, true);
    expect(edits.attributes('a')).toEqual({});
    expect(edits.attributes('b')).toEqual({ CutPower: '30' });
  });

  it('keeps staged values after failure and prevents overlapping saves', () => {
    const edits = new RecipeEdits();
    edits.set(recipe('a'), 'CutPower', '20');
    const save = edits.begin('a')!;
    expect(edits.begin('a')).toBeNull();
    edits.finish(save, false);
    expect(edits.attributes('a')).toEqual({ CutPower: '20' });
  });
});

describe('feature edits', () => {
  it('accumulates rapid taps and posts the next snapshot after acknowledgment', async () => {
    let draft = drawing();
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const save = vi.fn(async (features: Features, revision: number) => {
      if (revision === 1) await blocked;
      draft = { ...draft, features, revision: revision + 2 };
      return draft;
    });
    const edits = new FeatureEdits(() => draft, save);
    const a = edits.change((f) => { f.kerf!.width += 0.1; });
    const b = edits.change((f) => { f.kerf!.width += 0.1; });
    const c = edits.change((f) => { f.kerf!.width += 0.1; });
    expect(edits.value(draft).kerf!.width).toBeCloseTo(0.5);
    expect(save).toHaveBeenCalledTimes(1);
    release();
    await Promise.all([a, b, c]);
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls.map((call) => call[1])).toEqual([1, 3]);
    expect(draft.features.kerf!.width).toBeCloseTo(0.5);
  });

  it('retains a refused edit for review and cannot apply it to another part', async () => {
    let draft = drawing();
    const save = vi.fn().mockRejectedValue(new Error('the draft changed'));
    const edits = new FeatureEdits(() => draft, save);
    await expect(edits.change((f) => { f.kerf!.width = 0.5; })).rejects.toThrow('changed');
    expect(edits.value(draft).kerf!.width).toBe(0.5);
    expect(edits.error).toBe('the draft changed');
    draft = drawing('part-b', 9);
    expect(edits.value(draft).kerf!.width).toBe(0.2);
    expect(save).toHaveBeenCalledTimes(1);
    edits.discard();
  });

  it('does not show a previous job’s pending edits on a newly opened job for the same part', async () => {
    let draft = drawing();
    let finish!: (draft: DraftView) => void;
    const save = vi.fn(() => new Promise<DraftView>((resolve) => { finish = resolve; }));
    const edits = new FeatureEdits(() => draft, save);
    const pending = edits.change((f) => { f.kerf!.width = 0.5; });
    draft = drawing('part-a', 4, 2);
    expect(edits.value(draft).kerf!.width).toBe(0.2);
    await expect(edits.change((f) => { f.kerf!.width = 0.6; })).rejects.toThrow('still finishing');
    finish(drawing('part-a', 3));
    await pending;
    expect(edits.value(draft).kerf!.width).toBe(0.2);
  });
});

it('drops out-of-order picks and picks made against replaced geometry', () => {
  const picks = new LatestPick();
  const older = picks.begin(10);
  const latest = picks.begin(10);
  expect(picks.accepts(older, 10)).toBe(false);
  expect(picks.accepts(latest, 10)).toBe(true);
  expect(picks.accepts(latest, 11)).toBe(false);
});

it('orders the actual copied, deleted, joined and split source instances', () => {
  const contour = (sources: number[], layer = 'cut') => ({ sources, layer } as PreviewContour);
  expect(orderGroups([contour([0]), contour([2]), contour([3]), contour([4], 'hidden')], ['hidden'])).toEqual([[0], [2], [3]]);
  expect(orderGroups([contour([0, 2]), contour([2, 3]), contour([0, 2])], [])).toEqual([[0, 2, 3]]);
});

it('same-named fiber and CO2 materials never share a group', () => {
  const groups = materialsOf([recipe('a'), recipe('b', 'co2'), recipe('c')]);
  expect(groups.map((group) => [group.laser, group.recipes.map((recipe) => recipe.id)])).toEqual([['co2', ['b']], ['fiber', ['a', 'c']]]);
});
