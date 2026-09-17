import type { PreviewContour } from '../api';

/** The newest tap may use a reply only while its geometry still exists. */
export class LatestPick {
  private sequence = 0;
  begin(revision: number): { sequence: number; revision: number } { return { sequence: ++this.sequence, revision }; }
  accepts(ticket: { sequence: number; revision: number }, revision: number | undefined): boolean {
    return ticket.sequence === this.sequence && ticket.revision === revision;
  }
}

/** A joined path takes all its source instances together; split paths share a rank. */
export function orderGroups(contours: readonly PreviewContour[], hidden: readonly string[]): number[][] {
  const groups: number[][] = [];
  for (const contour of contours) {
    if (hidden.includes(contour.layer)) continue;
    const joined = new Set(contour.sources);
    for (let i = groups.length - 1; i >= 0; i--) {
      const group = groups[i]!;
      if (group.some((source) => joined.has(source))) {
        for (const source of group) joined.add(source);
        groups.splice(i, 1);
      }
    }
    if (joined.size) groups.push([...joined].sort((a, b) => a - b));
  }
  return groups;
}
