import { describe, expect, it } from 'vitest';
import type { PartView } from '../api';
import { hasAllParts, livePicks, onlyPart, partsOf, sourceLabel, togglePick } from './job-parts';

const part = (id: string, layers: [string, number][] = [['0', 1]]): PartView => ({
  tags: [], notes: '', quantity: 1, id, name: id.toUpperCase(), folder: null, file_name: `${id}.dxf`, bounds: null,
  contours: layers.reduce((n, [, c]) => n + c, 0), outline: [], layers: layers.map(([name, contours]) => ({ name, contours })), favourite: false, updated: 0,
});
const library = [part('a', [['0', 2], ['Etch', 1]]), part('b', [['0', 1]]), part('c', [['Etch', 3]])];

describe('job parts', () => {
  it('finds the parts a draft or a saved job cuts, in its order', () => {
    const draft = { parts: [{ id: 'c' }, { id: 'a' }] };
    expect(partsOf(draft, library).map(p => p.id)).toEqual(['c', 'a']);
    expect(partsOf({ parts: ['b', 'gone'] }, library).map(p => p.id)).toEqual(['b']);
    expect(onlyPart({ parts: ['b'] }, library)?.id).toBe('b');
    expect(onlyPart(draft, library)).toBeUndefined();
    expect(hasAllParts({ parts: ['a', 'b'] }, library)).toBe(true);
    expect(hasAllParts({ parts: ['a', 'gone'] }, library)).toBe(false);
    expect(partsOf(null, library)).toEqual([]);
  });

  it('labels the source files', () => {
    expect(sourceLabel([])).toBe('');
    expect(sourceLabel([library[0]!])).toBe('a.dxf');
    expect(sourceLabel(library)).toBe('a.dxf + 2 more');
  });

  it('keeps picks in the order picked', () => {
    let picks: string[] = [];
    for (const id of ['b', 'a', 'c', 'a']) picks = togglePick(picks, id);
    expect(picks).toEqual(['b', 'c']);
    expect(livePicks(['gone', 'c', 'b'], library)).toEqual(['c', 'b']);
  });
});
