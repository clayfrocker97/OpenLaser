import type { RecipeChange, RecipeView } from '../api';
import { numeric } from './recipe';

type Value<T> = { value: T; base: T | null; revision: number };
type Edit = { attributes: Record<string, Value<string>>; film?: Value<string | null> };
export type RecipeSave = { id: string; revision: number; change: RecipeChange };

/** Staged values survive page changes and belong to exactly one recipe. */
export class RecipeEdits {
  private entries = $state<Record<string, Edit>>(restore());
  storageError = $state<string | null>(null);
  get pending(): string[] { return Object.keys(this.entries).filter(id => this.count(id) > 0); }
  private persist(): void { try { localStorage.setItem('ol-recipe-drafts', JSON.stringify($state.snapshot(this.entries))); this.storageError = null; } catch { this.storageError = 'Material edits could not be retained in this browser.'; } }
  private revision = Date.now();
  saving = $state<Record<string, boolean>>({});

  attributes(id: string): Record<string, string> {
    return Object.fromEntries(Object.entries(this.entries[id]?.attributes ?? {}).map(([key, edit]) => [key, edit.value]));
  }

  film(id: string): string | null | undefined { return this.entries[id]?.film?.value; }
  count(id: string): number { return Object.keys(this.entries[id]?.attributes ?? {}).length + (this.entries[id]?.film ? 1 : 0); }

  set(recipe: RecipeView, key: string, value: string): void {
    this.entries[recipe.id] ??= { attributes: {} };
    const edit = this.entries[recipe.id]!;
    const was = recipe.attributes[key] ?? '';
    const equal = numeric(value) && numeric(was) ? Number(value) === Number(was) : value === was;
    // While saving, even a return to the old value is a newer edit.
    if (equal && !this.saving[recipe.id]) delete edit.attributes[key];
    else edit.attributes[key] = { value, base: edit.attributes[key] ? edit.attributes[key].base : recipe.attributes[key] ?? null, revision: ++this.revision };
    this.persist();
  }

  setFilm(recipe: RecipeView, value: string | null): void {
    this.entries[recipe.id] ??= { attributes: {} };
    const edit = this.entries[recipe.id]!;
    if (value === recipe.film && !this.saving[recipe.id]) delete edit.film;
    else edit.film = { value, base: edit.film ? edit.film.base : recipe.film, revision: ++this.revision };
    this.persist();
  }

  begin(id: string): RecipeSave | null {
    if (this.saving[id]) return null;
    this.saving[id] = true;
    const film = this.film(id);
    const expected_attributes = Object.fromEntries(Object.entries(this.entries[id]?.attributes ?? {}).map(([key, edit]) => [key, edit.base]));
    return {
      id,
      revision: this.revision,
      change: { attributes: this.attributes(id), expected_attributes, ...(film === undefined ? {} : { film, expected_film: this.entries[id]!.film!.base }) },
    };
  }

  finish(save: RecipeSave, success: boolean): void {
    delete this.saving[save.id];
    const edit = this.entries[save.id];
    if (!success || !edit) return;
    for (const [key, value] of Object.entries(edit.attributes)) {
      if (value.revision <= save.revision) delete edit.attributes[key];
      else if (save.change.attributes?.[key] !== undefined) value.base = save.change.attributes[key];
    }
    if (edit.film && edit.film.revision <= save.revision) delete edit.film;
    else if (edit.film && save.change.film !== undefined) edit.film.base = save.change.film;
    this.persist();
  }

  conflicts(recipe: RecipeView): Array<{ key: string; base: string; draft: string; saved: string }> {
    const edit = this.entries[recipe.id];
    if (!edit) return [];
    const conflicts = Object.entries(edit.attributes)
      .filter(([key, e]) => (recipe.attributes[key] ?? null) !== e.base && recipe.attributes[key] !== e.value)
      .map(([key, e]) => ({ key, base: e.base ?? 'removed', draft: e.value, saved: recipe.attributes[key] ?? 'removed' }));
    if (edit.film && recipe.film !== edit.film.base && recipe.film !== edit.film.value) conflicts.push({ key: '$film', base: edit.film.base ?? 'none', draft: edit.film.value ?? 'none', saved: recipe.film ?? 'none' });
    return conflicts;
  }
  resolve(recipe: RecipeView, key: string, ours: boolean): void {
    const edit = this.entries[recipe.id];
    if (!edit) return;
    if (key === '$film') { if (ours && edit.film) edit.film.base = recipe.film; else delete edit.film; }
    else if (ours && edit.attributes[key]) edit.attributes[key].base = recipe.attributes[key] ?? null;
    else delete edit.attributes[key];
    this.persist();
  }
  discard(id: string): void { delete this.entries[id]; this.persist(); }
  /** Deletion explicitly discards that recipe's edits; a fallback never inherits them. */
  retain(ids: readonly string[]): void {
    const live = new Set(ids);
    for (const id of Object.keys(this.entries)) if (!live.has(id)) this.discard(id);
  }
}

function restore(): Record<string, Edit> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem('ol-recipe-drafts') ?? '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter(([, edit]) => edit && typeof edit === 'object' && edit.attributes
      && Object.values(edit.attributes).every((e: unknown) => e && typeof e === 'object'
        && 'value' in e && typeof e.value === 'string' && 'revision' in e && typeof e.revision === 'number' && 'base' in e)));
  } catch { return {}; }
}
export const recipeEdits = new RecipeEdits();
