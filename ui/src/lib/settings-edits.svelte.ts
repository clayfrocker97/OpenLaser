import type { Document, HoldTimes, PreflightPreferences } from '../api';
import { ui } from '../stores/ui.svelte';
import { fieldId, type XmlField } from './machine-settings';

type Entry<T> = { base: T; value: T };
type RouteKey = 'adapter' | 'controller' | 'host';
type Import = { name: string; bytes: ArrayBuffer; base: string };
type Entries = {
  route?: Partial<Record<RouteKey, Entry<string>>>;
  preflight?: Entry<PreflightPreferences>;
  theme?: Entry<'light' | 'dark'>;
  hold?: Entry<HoldTimes>;
  backup?: Import;
  soft?: Import;
  xml?: { base: string; edits: Record<string, XmlField & { original: string }> };
};
export type SettingKey = keyof Entries;
export const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);
const preferenceKeys = ['fiber.enabled', 'fiber.steps', 'co2.enabled', 'co2.steps', 'pause.fiber.enabled', 'pause.fiber.steps', 'pause.co2.enabled', 'pause.co2.steps', 'postflight.fiber.enabled', 'postflight.fiber.steps', 'postflight.co2.enabled', 'postflight.co2.steps'] as const;
type PreferenceKey = typeof preferenceKeys[number];
/** Older browser drafts predate postflight; they must not override new defaults. */
export function withPostflight(value: PreflightPreferences, saved: PreflightPreferences): PreflightPreferences {
  const copy = structuredClone($state.snapshot(value));
  copy.postflight ??= structuredClone($state.snapshot(saved.postflight));
  copy.pause ??= structuredClone($state.snapshot(saved.pause));
  if (copy.version === 1) {
    if (copy.confirm_gas) for (const mode of ['fiber', 'co2'] as const) {
      if (!copy[mode].steps.some(step => step.action?.kind === 'gas_test' || step.action?.kind === 'job_gas_test')) {
        copy[mode].steps.push({ text: 'Gas supply is ready', action: { kind: 'job_gas_test', duration_ms: 500 }, auto_check: false });
      }
    }
    copy.version = 2;
  }
  copy.confirm_gas = false;
  return copy;
}
function checklist(p: PreflightPreferences, key: PreferenceKey) {
  const root = key.startsWith('postflight.') ? p.postflight : key.startsWith('pause.') ? p.pause : p;
  return key.includes('fiber') ? root.fiber : root.co2;
}
function preference(p: PreflightPreferences, key: PreferenceKey): unknown {
  const list = checklist(p, key);
  return key.endsWith('.enabled') ? list.enabled : list.steps;
}
function copyPreference(target: PreflightPreferences, source: PreflightPreferences, key: PreferenceKey): void {
  const [to, from] = [checklist(target, key), checklist(source, key)];
  if (key.endsWith('.enabled')) to.enabled = from.enabled;
  else to.steps = structuredClone($state.snapshot(from.steps));
}
export function mergePreferences(base: PreflightPreferences, draft: PreflightPreferences, saved: PreflightPreferences) {
  base = withPostflight(base, saved);
  draft = withPostflight(draft, saved);
  const value = structuredClone($state.snapshot(saved));
  const conflicts: Array<{ key: PreferenceKey; base: unknown; draft: unknown; saved: unknown }> = [];
  for (const key of preferenceKeys) {
    const [b, d, s] = [preference(base, key), preference(draft, key), preference(saved, key)];
    if (same(b, d)) continue;
    if (!same(b, s) && !same(d, s)) conflicts.push({ key, base: b, draft: d, saved: s });
    copyPreference(value, draft, key);
  }
  return { value, conflicts };
}

/** Settings imports use the browser's blob-capable store; quota failures stay visible. */
async function storage(value?: Entries): Promise<Entries> {
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('openlaser-drafts', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('settings');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    return await new Promise<Entries>((resolve, reject) => {
      const transaction = db.transaction('settings', value ? 'readwrite' : 'readonly');
      const store = transaction.objectStore('settings');
      const request = value ? store.put(value, 'draft') : store.get('draft');
      transaction.oncomplete = () => resolve(value ?? request.result ?? {});
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error);
    });
  } finally { db.close(); }
}

class SettingsEdits {
  entries = $state<Entries>({});
  error = $state<string | null>(null);
  private writes: Promise<unknown> = Promise.resolve();
  readonly ready = storage().then((entries) => {
    this.entries = entries;
    if (entries.theme) ui.previewTheme(entries.theme.value);
  }).catch(() => { this.error = 'Settings drafts could not be read from this browser.'; });
  get pending(): SettingKey[] { return Object.keys(this.entries) as SettingKey[]; }
  readTheme(): 'light' | 'dark' {
    try { const saved: unknown = JSON.parse(localStorage.getItem('ol-theme') ?? 'null'); if (saved === 'light' || saved === 'dark') return saved; } catch { /* use the retained base */ }
    return this.entries.theme?.base ?? ui.theme;
  }
  async rebaseTheme(): Promise<void> {
    if (this.entries.theme) this.entries.theme.base = this.readTheme();
    await this.persist();
  }
  private async persist(): Promise<void> {
    const value = structuredClone($state.snapshot(this.entries));
    this.writes = this.writes.catch(() => {}).then(() => storage(value));
    try { await this.writes; this.error = null; }
    catch { this.error = 'Settings drafts could not be retained in this browser. Keep this page open until saved.'; }
  }
  async route(key: RouteKey, value: string, saved: string): Promise<void> {
    await this.ready;
    this.entries.route ??= {};
    const base = this.entries.route[key]?.base ?? saved;
    if (value === saved) delete this.entries.route[key];
    else this.entries.route[key] = { base, value };
    if (!Object.keys(this.entries.route).length) delete this.entries.route;
    await this.persist();
  }
  async theme(value: 'light' | 'dark'): Promise<void> {
    await this.ready;
    const base = this.entries.theme?.base ?? ui.theme;
    if (value === base) delete this.entries.theme;
    else this.entries.theme = { base, value };
    ui.previewTheme(value);
    await this.persist();
  }
  /** Stages hold times against the ones every screen uses now. */
  async hold(value: HoldTimes, saved: HoldTimes): Promise<void> {
    await this.ready;
    const base = this.entries.hold?.base ?? saved;
    if (same(value, base)) delete this.entries.hold;
    else this.entries.hold = { base: { ...base }, value: { ...value } };
    await this.persist();
  }
  /** Keeps the staged hold times over ones saved elsewhere since. */
  async rebaseHold(saved: HoldTimes): Promise<void> {
    if (this.entries.hold) this.entries.hold.base = { ...saved };
    await this.persist();
  }
  async preflight(base: PreflightPreferences, value: PreflightPreferences): Promise<void> {
    await this.ready;
    const original = withPostflight(this.entries.preflight?.base ?? base, base);
    if (same(original, value)) delete this.entries.preflight;
    else this.entries.preflight = { base: structuredClone($state.snapshot(original)), value: structuredClone($state.snapshot(value)) };
    await this.persist();
  }
  async file(key: 'backup' | 'soft', file: File, base: string): Promise<void> {
    await this.ready;
    if (file.size > (key === 'soft' ? 1 : 10) * 1024 * 1024) throw new Error('The selected file is too large.');
    this.entries[key] = { name: file.name, bytes: await file.arrayBuffer(), base: this.entries[key]?.base ?? base };
    await this.persist();
  }
  async xml(field: XmlField, value: string, hash: string): Promise<void> {
    await this.ready;
    this.entries.xml ??= { base: hash, edits: {} };
    const id = fieldId(field);
    const original = this.entries.xml.edits[id]?.original ?? field.value;
    if (value === original) delete this.entries.xml.edits[id];
    else this.entries.xml.edits[id] = { ...field, value, original };
    if (!Object.keys(this.entries.xml.edits).length) delete this.entries.xml;
    await this.persist();
  }
  async discard(key: SettingKey, saved = false): Promise<void> {
    if (key === 'theme' && this.entries.theme) {
      const current = this.readTheme();
      if (saved && current !== this.entries.theme.base && current !== this.entries.theme.value) throw new Error('Display settings changed; choose which theme to keep.');
      if (saved) localStorage.setItem('ol-theme', JSON.stringify(this.entries.theme.value));
      ui.previewTheme(saved ? this.entries.theme.value : current);
    }
    delete this.entries[key];
    await this.persist();
  }
  async rebaseFile(key: 'backup' | 'soft', hash: string): Promise<void> {
    if (this.entries[key]) this.entries[key].base = hash;
    await this.persist();
  }
  async resolveRoute(key: RouteKey, saved: string, ours: boolean): Promise<void> {
    const entry = this.entries.route?.[key];
    if (!entry) return;
    if (ours) entry.base = saved; else delete this.entries.route![key];
    if (!Object.keys(this.entries.route!).length) delete this.entries.route;
    await this.persist();
  }
  async resolvePreflight(key: PreferenceKey, saved: PreflightPreferences, ours: boolean): Promise<void> {
    const entry = this.entries.preflight;
    if (!entry) return;
    entry.base = withPostflight(entry.base, saved);
    entry.value = withPostflight(entry.value, saved);
    copyPreference(entry.base, saved, key);
    if (!ours) copyPreference(entry.value, saved, key);
    await this.persist();
  }
}

export const routeValues = (doc: Document): Record<RouteKey, string> => ({ adapter: doc.link.remembered_adapter, controller: doc.link.controller, host: doc.link.computer });
export const settingsEdits = new SettingsEdits();
