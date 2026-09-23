import { expect, it } from 'vitest';
import { pasteBatch, pasteable, type CopiedShapes, type PasteDirection } from './copy-paste';

const copied = (): CopiedShapes => ({
  parts: ['part-a'], width: 20, height: 10, offset: [0, 0],
  contours: [
    { source: 8, transform: [0, 1, -1, 0, 10, 20] },
    { source: 2, transform: [-2, 0, 0, 2, 15, 25] },
    { source: 8, transform: [1, 0, 0, 1, 30, 20] },
  ],
  grouping: [[0, 1], [2]],
  leads: [{ location: { contour: 1, fraction: .25 }, entry: { shape: 'line', length: 4, radius: 1, angle: 70 }, exit: null }],
});

it('pastes into jobs whose parts begin with the parts copied from', () => {
  const job = (...ids: string[]) => ({ parts: ids.map((id, i) => ({ id, first: i * 10, contours: 10 })) });
  expect(pasteable(copied(), job('part-a'))).toBe(true);
  expect(pasteable(copied(), job('part-a', 'part-b'))).toBe(true);
  expect(pasteable(copied(), job('part-b', 'part-a'))).toBe(false);
  expect(pasteable({ ...copied(), parts: ['part-a', 'part-b'] }, job('part-a'))).toBe(false);
  expect(pasteable(null, job('part-a'))).toBe(false);
  expect(pasteable(copied(), null)).toBe(false);
});

it('batches complete selections and remaps groups and local leads for every copy', () => {
  const source = copied(), before = structuredClone(source);
  const batch = pasteBatch(source, { count: 3, gap: 5, direction: 'right' }, 10);
  expect(batch.contours.map(c => c.transform)).toEqual([
    [0, 1, -1, 0, 35, 20], [-2, 0, 0, 2, 40, 25], [1, 0, 0, 1, 55, 20],
    [0, 1, -1, 0, 60, 20], [-2, 0, 0, 2, 65, 25], [1, 0, 0, 1, 80, 20],
    [0, 1, -1, 0, 85, 20], [-2, 0, 0, 2, 90, 25], [1, 0, 0, 1, 105, 20],
  ]);
  expect(batch.contours.map(c => c.source)).toEqual([8, 2, 8, 8, 2, 8, 8, 2, 8]);
  expect(batch.grouping).toEqual([[0, 1], [2], [3, 4], [5], [6, 7], [8]]);
  expect(batch.lead_overrides.map(lead => lead.location)).toEqual([
    { contour: 1, fraction: .25 }, { contour: 4, fraction: .25 }, { contour: 7, fraction: .25 },
  ]);
  expect(batch.lead_overrides.every(lead => lead.entry?.length === 4 && lead.exit === null)).toBe(true);
  expect(batch.offset).toEqual([75, 0]);
  expect(source).toEqual(before);
});

it.each<[PasteDirection, [number, number]]>([
  ['right', [40, 0]], ['left', [-40, 0]], ['up', [0, 20]], ['down', [0, -20]],
])('places copies %s using the selection bounds and supports zero gap', (direction, offset) => {
  expect(pasteBatch(copied(), { count: 2, gap: 0, direction }, 3).offset).toEqual(offset);
});

it('continues at the last accepted copy when spacing or direction changes', () => {
  const source = copied();
  source.offset = pasteBatch(source, { count: 3, gap: 5, direction: 'right' }, 3).offset;
  const next = pasteBatch(source, { count: 1, gap: 2, direction: 'up' }, 12);
  expect(next.offset).toEqual([75, 12]);
  expect(next.contours[0]!.transform).toEqual([0, 1, -1, 0, 85, 32]);
});

it('rejects invalid counts, gaps and oversized layouts before constructing a batch', () => {
  const source = copied();
  for (const count of [0, -1, 1.5, 501, NaN, Infinity]) expect(() => pasteBatch(source, { count, gap: 10, direction: 'right' }, 0)).toThrow('whole copy count');
  for (const gap of [-1, NaN, Infinity]) expect(() => pasteBatch(source, { count: 1, gap, direction: 'right' }, 0)).toThrow('finite gap');
  expect(() => pasteBatch(source, { count: 2, gap: 0, direction: 'right' }, 99995)).toThrow('100,000');
  expect(pasteBatch(source, { count: 2, gap: 0, direction: 'right' }, 99994).contours).toHaveLength(6);
  expect(() => pasteBatch({ ...source, contours: [] }, { count: 1, gap: 0, direction: 'right' }, 0)).toThrow('Copy shapes');
  expect(source.offset).toEqual([0, 0]);
});
