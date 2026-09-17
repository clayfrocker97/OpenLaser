import type { Folder } from '../api';

/** A bounded parent walk, even if an older server supplies a cyclic list. */
export function folderPath(folders: Folder[], id: string | null): Folder[] {
  const path: Folder[] = [];
  const seen = new Set<string>();
  let folder = folders.find((item) => item.id === id);
  while (folder && !seen.has(folder.id)) {
    seen.add(folder.id);
    path.unshift(folder);
    folder = folders.find((item) => item.id === folder!.parent);
  }
  return path;
}
