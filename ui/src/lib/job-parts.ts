// The library parts a job cuts. A job's drawing is its parts' drawings
// joined in order; the server lays them out side by side and keeps each
// part's contours in one range of that drawing (DraftView.parts).
import type { PartView } from '../api';

/** What a job refers to its parts by. */
type Parts = { parts: ReadonlyArray<string | { id: string }> } | null | undefined;

const ids = (job: Parts): string[] => (job?.parts ?? []).map((part) => (typeof part === 'string' ? part : part.id));

/** The library parts a draft or saved job cuts, in its order. */
export function partsOf(job: Parts, library: readonly PartView[]): PartView[] {
  return ids(job).flatMap((id) => library.find((part) => part.id === id) ?? []);
}

/** The one part a draft or saved job cuts, when it cuts exactly one. */
export function onlyPart(job: Parts, library: readonly PartView[]): PartView | undefined {
  const list = ids(job);
  return list.length === 1 ? library.find((part) => part.id === list[0]) : undefined;
}

/** Whether every part a saved job cuts is still in the library. */
export function hasAllParts(job: Parts, library: readonly PartView[]): boolean {
  return ids(job).every((id) => library.some((part) => part.id === id));
}

/** The files a job's parts came from, for a header: one file, or the first and a count. */
export function sourceLabel(parts: readonly PartView[]): string {
  if (parts.length <= 1) return parts[0]?.file_name ?? '';
  return `${parts[0]!.file_name} + ${parts.length - 1} more`;
}

/** Picks with `id` added at the end, or taken out; the order picked is the order laid out. */
export function togglePick(picks: readonly string[], id: string): string[] {
  return picks.includes(id) ? picks.filter((pick) => pick !== id) : [...picks, id];
}

/** The picks still in the library, in the order picked. */
export function livePicks(picks: readonly string[], library: readonly PartView[]): string[] {
  return picks.filter((id) => library.some((part) => part.id === id));
}
