// Importing vendor recipe files with a review step. Files come from a file
// picker, a folder picker or a drop of files and folders; every recipe
// file is read by the server without saving (a preview), assigned to a
// material of the library, checked against the recipes already there, and
// only saved once the operator has looked at the list.

import type { HeadSetup, LaserMode, RecipeImport, RecipePreview, RecipeView } from '../api';
import { cardFor, materialsOf } from './materials';

/** What happens to one file when the reviewed import runs. */
export type Choice = 'replace' | 'keep' | 'skip';

/** One recipe file under review. */
export interface ImportItem {
  /** A stable key for the list. */
  key: string;
  file: File;
  /** The sample cut photo of the same name beside it. */
  photo: File | null;
  preview: RecipePreview;
  /** The material it is filed under. */
  material: string;
  /** Whether that material is new to the library. */
  isNew: boolean;
  thickness_mm: number;
  /** The nozzle, focus and lens, as read and as the operator corrected them. */
  setup: HeadSetup;
  /** What to do with a duplicate; null until the operator chooses. */
  choice: Choice | null;
}

/** A file that could not be read, with the reason. */
export interface ImportFailure { name: string; reason: string }

/** The most recipe files one import takes. */
export const MAX_FILES = 500;

/** A file and the folder path it was found under, `/`-separated. */
export interface Picked { file: File; path: string }

const stem = (name: string): string => name.replace(/\.[^.]+$/, '');
const folderOf = (path: string): string => path.split('/').slice(0, -1).join('/');

/** Files from a file or folder picker, with the folder each came from. */
export const picked = (files: Iterable<File>): Picked[] => [...files].map((file) => ({ file, path: file.webkitRelativePath || file.name }));

/** The recipe files among `files`, each with the photo of the same name
 * in the same folder, sorted by path. */
export function recipeFiles(files: readonly Picked[]): Array<{ file: File; path: string; photo: File | null }> {
  const key = (p: Picked) => `${folderOf(p.path)}/${stem(p.file.name)}`;
  const photos = new Map(files.filter((p) => /\.(png|jpe?g)$/i.test(p.file.name)).map((p) => [key(p), p.file]));
  return files
    .filter((p) => /\.xml$/i.test(p.file.name))
    .map((p) => ({ file: p.file, path: p.path, photo: photos.get(key(p)) ?? null }))
    .sort((a, b) => a.path.localeCompare(b.path, undefined, { numeric: true }));
}

/** Every file in a drop, folders read to the bottom. The entries must be
 * taken while the drop event runs, so call this from the handler itself. */
export function droppedFiles(transfer: DataTransfer): Promise<Picked[]> {
  const entries = [...transfer.items].map((item) => (item.kind === 'file' ? item.webkitGetAsEntry?.() ?? null : null));
  if (!entries.some(Boolean)) return Promise.resolve(picked(transfer.files));
  const files: Picked[] = [];
  const read = async (entry: FileSystemEntry, depth: number): Promise<void> => {
    if (files.length > MAX_FILES * 4) return;
    if (entry.isFile) {
      const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
      files.push({ file, path: entry.fullPath.replace(/^\//, '') });
    } else if (entry.isDirectory && depth < 8) {
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      for (;;) {
        const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
        if (!batch.length) break;
        for (const child of batch) await read(child, depth + 1);
      }
    }
  };
  return (async () => {
    for (const entry of entries) if (entry) await read(entry, 0);
    return files;
  })();
}

const norm = (name: string): string => name.trim().toLowerCase().replace(/[\s_-]+/g, ' ');

/** The library material a file's material name means for `laser`: the
 * same name, else the same kind of material (`SS` is Stainless steel),
 * preferring one that already has the thickness. Null for a new one. */
export function matchMaterial(name: string, laser: LaserMode, thickness_mm: number, recipes: readonly RecipeView[]): string | null {
  const materials = materialsOf(recipes).filter((m) => m.laser === laser);
  const exact = materials.find((m) => norm(m.name) === norm(name));
  if (exact) return exact.name;
  const card = cardFor(name);
  if (!card) return null;
  const alike = materials.filter((m) => cardFor(m.name)?.id === card.id);
  const sized = alike.find((m) => m.recipes.some((r) => Math.abs(r.thickness_mm - thickness_mm) < 1e-6));
  return (sized ?? alike[0])?.name ?? null;
}

/** The name a new material gets: the card's name for a vendor's short name. */
export function newMaterialName(name: string): string {
  return cardFor(name)?.name ?? name;
}

/** The first review of a previewed file: matched to a material, its setup as read. */
export function reviewItem(key: string, file: File, photo: File | null, preview: RecipePreview, recipes: readonly RecipeView[]): ImportItem {
  const matched = matchMaterial(preview.name, preview.laser, preview.thickness_mm, recipes);
  return {
    key, file, photo, preview,
    material: matched ?? newMaterialName(preview.name),
    isNew: matched === null,
    thickness_mm: preview.thickness_mm,
    setup: { ...preview.setup },
    choice: null,
  };
}

/** Whether a recipe has the identity an item would be saved with. */
const same = (item: ImportItem, r: Pick<RecipeView, 'laser' | 'name' | 'thickness_mm' | 'gas'>): boolean =>
  r.laser === item.preview.laser && norm(r.name) === norm(item.material) && Math.abs(r.thickness_mm - item.thickness_mm) < 1e-6 && r.gas === item.preview.gas;

/** What an item duplicates: library recipes of the same material,
 * thickness and gas, and earlier files of the batch that will be saved
 * with the same identity. */
export function duplicatesOf(item: ImportItem, items: readonly ImportItem[], recipes: readonly RecipeView[]): { recipes: RecipeView[]; earlier: ImportItem[]; identical: boolean } {
  const found = recipes.filter((r) => same(item, r));
  const before = items.slice(0, items.indexOf(item)).filter((other) => other.choice !== 'skip' && same(item, { laser: other.preview.laser, name: other.material, thickness_mm: other.thickness_mm, gas: other.preview.gas }));
  return { recipes: found, earlier: before, identical: !!item.preview.existing && found.some((r) => r.id === item.preview.existing) };
}

/** Whether an item is a duplicate that needs a choice. */
export const needsChoice = (item: ImportItem, items: readonly ImportItem[], recipes: readonly RecipeView[]): boolean => {
  const d = duplicatesOf(item, items, recipes);
  return d.recipes.length > 0 || d.earlier.length > 0;
};

/** The choices a duplicate offers: Replace only a library recipe. */
export function choicesFor(item: ImportItem, items: readonly ImportItem[], recipes: readonly RecipeView[]): Choice[] {
  return duplicatesOf(item, items, recipes).recipes.length ? ['replace', 'keep', 'skip'] : ['keep', 'skip'];
}

/** The request that saves an item, or null when it is skipped. */
export function importRequest(item: ImportItem, items: readonly ImportItem[], recipes: readonly RecipeView[]): RecipeImport | null {
  const dup = needsChoice(item, items, recipes);
  if (dup && (item.choice === null || item.choice === 'skip')) return null;
  const target = duplicatesOf(item, items, recipes).recipes[0];
  const request: RecipeImport = {
    name: item.preview.file_name || item.file.name,
    material: item.material,
    thickness_mm: item.thickness_mm,
    keep_both: dup && item.choice === 'keep',
    setup: true,
  };
  if (dup && item.choice === 'replace' && target) request.replace = target.id;
  if (item.setup.nozzle_diameter_mm) request.nozzle_diameter_mm = item.setup.nozzle_diameter_mm;
  if (item.setup.nozzle) request.nozzle = item.setup.nozzle;
  if (item.setup.focus_mm) request.focus_mm = item.setup.focus_mm;
  if (item.setup.lens_mm) request.lens_mm = item.setup.lens_mm;
  return request;
}
