import type { Lead, LeadOverride, Leads, Spot } from '../api';

export type LeadRole = 'entry' | 'exit';
const same = (a: Spot, b: Spot) => a.contour === b.contour && a.fraction === b.fraction;
const key = (target: Spot) => `${target.contour}:${target.fraction}`;

/** The role switch is shared; an enabled role can have a local definition. */
export function leadFor(leads: Leads | null, target: Spot, role: LeadRole): Lead | null {
  const base = leads?.[role];
  if (!base) return null;
  return leads?.overrides?.find(edited => same(edited.location, target))?.[role] ?? base;
}

/** Change one lead on an already cloned feature snapshot. */
export function setLeadOverride(leads: Leads, target: Spot, role: LeadRole, lead: Lead): void {
  setLeadOverrides(leads, [target], role, lead);
}

/** One selection edit, with duplicate split-path handles collapsed by owner. */
export function setLeadOverrides(leads: Leads, targets: Spot[], role: LeadRole, lead: Lead): void {
  const pending = new Map(targets.map(target => [key(target), target]));
  const overrides = (leads.overrides ?? []).map(edited => pending.delete(key(edited.location))
    ? { ...edited, [role]: { ...lead } } : edited);
  for (const target of pending.values()) {
    overrides.push({ location: { ...target }, entry: null, exit: null, [role]: { ...lead } });
  }
  leads.overrides = overrides.sort((a, b) => a.location.contour - b.location.contour || a.location.fraction - b.location.fraction);
}

/** Resolve many visible handles without scanning every edit for every contour. */
export function leadLookup(leads: Leads | null): (target: Spot, role: LeadRole) => Lead | null {
  const overrides = new Map((leads?.overrides ?? []).map(edited => [key(edited.location), edited]));
  return (target, role) => leads?.[role] ? overrides.get(key(target))?.[role] ?? leads[role] : null;
}

/** Clipboard locations index the pasted contour list, preserving copy identity. */
export function copiedLeadOverrides(leads: Leads | null, contours: number[]): LeadOverride[] {
  const positions = new Map(contours.map((contour, index) => [contour, index]));
  return (leads?.overrides ?? []).flatMap(edited => {
    const contour = positions.get(edited.location.contour);
    return contour === undefined ? [] : [{
      location: { ...edited.location, contour },
      entry: edited.entry ? { ...edited.entry } : null,
      exit: edited.exit ? { ...edited.exit } : null,
    }];
  });
}
