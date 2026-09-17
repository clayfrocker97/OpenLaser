import { expect, it } from 'vitest';
import type { Lead, Leads } from '../api';
import { copiedLeadOverrides, leadFor, leadLookup, setLeadOverride, setLeadOverrides } from './lead-overrides';

const line = (length: number, angle = 90): Lead => ({ shape: 'line', length, angle, radius: 1 });
const defaults = (): Leads => ({ entry: line(2), exit: line(1), closed_only: true, side: 'auto', overrides: [] });

it('repeated entry and exit drags preserve defaults and other contours, including split pieces', () => {
  const leads = defaults(), target = { contour: 8, fraction: .2 };
  setLeadOverride(leads, target, 'entry', line(4, 70));
  setLeadOverride(leads, target, 'exit', line(3, 60));
  setLeadOverride(leads, target, 'entry', line(5, 120));
  expect(leads.entry).toEqual(line(2));
  expect(leads.exit).toEqual(line(1));
  expect(leads.overrides).toHaveLength(1);
  expect(leadFor(leads, target, 'entry')).toEqual(line(5, 120));
  expect(leadFor(leads, target, 'exit')).toEqual(line(3, 60));
  for (const other of [{ contour: 7, fraction: .2 }, { contour: 8, fraction: .7 }]) {
    expect(leadFor(leads, other, 'entry')).toEqual(line(2));
    expect(leadFor(leads, other, 'exit')).toEqual(line(1));
  }
  leads.entry = line(6);
  expect(leadFor(leads, target, 'entry')).toEqual(line(5, 120));
  leads.entry = null;
  expect(leadFor(leads, target, 'entry')).toBeNull();
});

it('copies only selected placed identities and keeps a clipboard snapshot independent', () => {
  const leads = defaults();
  setLeadOverride(leads, { contour: 8, fraction: .2 }, 'entry', line(4));
  setLeadOverride(leads, { contour: 2, fraction: .5 }, 'exit', line(3));
  const copied = copiedLeadOverrides(leads, [8, 3]);
  expect(copied).toEqual([{ location: { contour: 0, fraction: .2 }, entry: line(4), exit: null }]);
  leads.overrides[1]!.entry!.length = 9;
  expect(copied[0]!.entry!.length).toBe(4);
  copied[0]!.location.contour = 20;
  expect(leads.overrides[1]!.location.contour).toBe(8);
});

it('a selection drag sets the same role on selected owners, preserving other roles and unselected edits', () => {
  const leads = defaults();
  const a = { contour: 1, fraction: .2 }, b = { contour: 4, fraction: .6 }, outside = { contour: 7, fraction: .2 };
  setLeadOverride(leads, a, 'exit', line(3, 50));
  setLeadOverride(leads, b, 'entry', line(6, 80));
  setLeadOverride(leads, outside, 'entry', line(9, 100));
  setLeadOverrides(leads, [a, b, { ...b }], 'entry', line(4, 70));
  const resolve = leadLookup(leads);
  expect(leads.overrides).toHaveLength(3);
  expect(resolve(a, 'entry')).toEqual(line(4, 70));
  expect(resolve(b, 'entry')).toEqual(line(4, 70));
  expect(resolve(a, 'exit')).toEqual(line(3, 50));
  expect(resolve(b, 'exit')).toEqual(line(1));
  expect(resolve(outside, 'entry')).toEqual(line(9, 100));
  expect(resolve({ contour: 9, fraction: .2 }, 'entry')).toEqual(line(2));
  expect(leads.entry).toEqual(line(2));
  expect(leads.exit).toEqual(line(1));
});

it('a whole-sheet drag creates one independent edit per selected contour', () => {
  const leads = defaults(), targets = Array.from({ length: 4998 }, (_, contour) => ({ contour, fraction: .125 }));
  setLeadOverrides(leads, targets, 'entry', line(4, 70));
  setLeadOverrides(leads, targets, 'exit', line(3, 60));
  setLeadOverrides(leads, targets, 'entry', line(5, 110));
  const resolve = leadLookup(leads);
  expect(leads.overrides).toHaveLength(4998);
  for (const target of targets) {
    expect(resolve(target, 'entry')).toEqual(line(5, 110));
    expect(resolve(target, 'exit')).toEqual(line(3, 60));
  }
  leads.overrides[0]!.entry!.length = 8;
  expect(resolve(targets[1]!, 'entry')!.length).toBe(5);
  expect(leads.entry).toEqual(line(2));
});
