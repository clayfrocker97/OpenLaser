// What the UI remembers for itself: the page, selections, and the bar.
import type { Features, NestLive, Pick, Preview } from '../api';
import { SETTINGS_PAGES } from '../lib/navigation';

export type Tab = 'parts' | 'setup' | 'run' | 'materials' | 'machine';
/** A question before a destructive or machine-changing action (DESIGN.md, Touch rules). */
export type ConfirmRequest = {
  title: string;
  /** What will happen, in the operator's terms. */
  body: string;
  /** The confirming button's label: the consequence, such as "Delete part". */
  confirm: string;
  /** Loses work or changes the machine: shown in the danger colour. */
  danger?: boolean;
};
type Confirmation = ConfirmRequest & { resolve: (ok: boolean) => void };

const FEATURE_IDS = ['leads', 'joints', 'cooling', 'kerf', 'bridges', 'order', 'start', 'common'] as const;
export type FeatureId = (typeof FEATURE_IDS)[number];

/** What a tap on the drawing places while a panel asks for it. */
export interface Picking {
  features: Features;
  preview?: Preview;
  marks: [number, number][];
  feature: 'joints' | 'cooling' | 'start' | 'bridges' | 'order';
  /** The first end of the bridge being placed. */
  first: Pick | null;
  /** The contours tapped so far, for a manual order. */
  order: number[];
  /** The geometry on which the pending picks were made. */
  revision: number;
  /** Local point owner for an evolving bridge contour. */
  firstOwner?: number;
}

/** The drawing bar's default order: view tools, then editing tools, then history. */
const DRAW_BAR = ['fit', 'zoom-in', 'zoom-out', 'layers', 'snap', 'grid', 'mirror-x', 'mirror-y', 'turn', 'scale', 'center', 'reset', 'group', 'ungroup', 'copy', 'paste', 'delete', 'deselect', 'undo', 'redo'];

/** A saved order keeps only known tools and gains tools added since it was saved. */
function withAll(order: string[], all: string[]): string[] {
  const known = order.filter((id, i) => all.includes(id) && order.indexOf(id) === i);
  return [...known, ...all.filter((id) => !known.includes(id))];
}

function remembered<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

function remember(key: string, value: unknown): void {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* private mode */ }
}

function returningTab(): Tab {
  try {
    const tab = sessionStorage.getItem('ol-tab-return');
    sessionStorage.removeItem('ol-tab-return');
    if (tab === 'parts' || tab === 'setup' || tab === 'run' || tab === 'materials' || tab === 'machine') return tab;
  } catch { /* Opening a layout does not require browser storage. */ }
  return 'parts';
}

function returningSettingsPage(): number {
  try {
    const page = Number(sessionStorage.getItem('ol-settings-return'));
    sessionStorage.removeItem('ol-settings-return');
    if (SETTINGS_PAGES.some(item => item.id === page)) return page;
  } catch { /* The default settings section remains available. */ }
  return 0;
}

class Ui {
  tab = $state<Tab>(returningTab());
  search = $state('');
  folder = $state<string | null>(null);
  /** The selected library card: a part or a job id. */
  selected = $state<string | null>(null);
  /** Parts picked to set up as one job, in the order picked; null when
   *  the library is not picking. */
  partPicks = $state<string[] | null>(null);
  selectedRecipe = $state<string | null>(null);
  setupPanel = $state<FeatureId | 'copy' | 'clipboard' | 'nest' | null>(null);
  nestPreview = $state<Preview | null>(null);
  nestStock = $state<{ outline: number[][]; cutouts: number[][][] } | null>(null);
  /** Where a running nesting search has the parts, for the canvas. */
  nestLive = $state<NestLive | null>(null);
  /** A nesting result or a running search is on the canvas instead of the
   *  drawing, which cannot be edited meanwhile. */
  get nestShown(): boolean { return !!this.nestPreview || !!this.nestLive; }
  nestPicking = $state(false);
  selectionEpoch = $state(0);
  picking = $state<Picking | null>(null);
  /** Drawing layers hidden on the canvas, for this session. */
  hiddenDrawingLayers = $state<string[]>([]);
  favTools = $state<string[]>(remembered('ol-bar', FEATURE_IDS.filter(id => id !== 'common')));
  editBar = $state(false);
  /** The drawing bar's tools in the operator's order; see Canvas.svelte. */
  drawBar = $state<string[]>(withAll(remembered('ol-draw-bar', DRAW_BAR), DRAW_BAR));
  editDrawBar = $state(false);
  jogFast = $state(false);
  /** The X/Y step a tap on a jog key moves, in millimetres. Holding a key jogs continuously. */
  jogStep = $state<number>(remembered<number | null>('ol-jog-step', 1) ?? 1);
  snap = $state(remembered('ol-snap', true));
  grid = $state(remembered('ol-grid', 1));
  /** Canvas layers the operator switched off. */
  hiddenLayers = $state<string[]>(remembered('ol-hidden-layers', []));
  /** Whether the run view shows every travel move or only the next one. */
  travelMode = $state<'next' | 'all'>(remembered('ol-travel', 'next'));
  machinePage = $state(returningSettingsPage());
  checklistEditor = $state<'defaults' | 'pause' | 'postflight' | null>(null);
  theme = $state<'light' | 'dark'>(remembered('ol-theme', matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));
  toast = $state<{ text: string; error: boolean; at: number } | null>(null);
  /** The confirmation on screen, if one is asked. */
  confirmation = $state<Confirmation | null>(null);
  modal = $state<'pending' | 'alarms' | 'material' | 'recipe' | 'outputs' | null>(null);
  pendingJobName = $state<string | null>(null);

  editChecklist(scope: 'defaults' | 'pause' | 'postflight'): void {
    this.machinePage = 1;
    this.checklistEditor = scope;
    this.tab = 'machine';
  }

  previewTheme(theme: 'light' | 'dark'): void {
    this.theme = theme;
    document.documentElement.dataset['theme'] = theme;
  }

  applyTheme(theme: 'light' | 'dark'): void {
    this.previewTheme(theme);
    remember('ol-theme', theme);
  }

  setGrid(value: number): void { this.grid = value; remember('ol-grid', value); }
  /** Sheet sizes saved on this screen for the stock chooser, width by height in millimetres. */
  sheetSizes = $state<Array<[number, number]>>(remembered('ol-sheet-sizes', []));
  saveSheetSize(size: [number, number]): void {
    if (this.sheetSizes.some(([w, h]) => w === size[0] && h === size[1])) return;
    this.sheetSizes = [...this.sheetSizes, size];
    remember('ol-sheet-sizes', this.sheetSizes);
  }
  forgetSheetSize(size: [number, number]): void {
    this.sheetSizes = this.sheetSizes.filter(([w, h]) => w !== size[0] || h !== size[1]);
    remember('ol-sheet-sizes', this.sheetSizes);
  }
  setDrawBar(order: string[]): void { this.drawBar = withAll(order, DRAW_BAR); remember('ol-draw-bar', this.drawBar); }
  setJogStep(step: number): void { this.jogStep = step; remember('ol-jog-step', step); }
  toggleSnap(): void { this.snap = !this.snap; remember('ol-snap', this.snap); }

  setBar(tools: string[]): void {
    this.favTools = tools;
    remember('ol-bar', tools);
  }

  layerShown(layer: string): boolean {
    return !this.hiddenLayers.includes(layer);
  }

  toggleLayer(layer: string): void {
    this.hiddenLayers = this.layerShown(layer) ? [...this.hiddenLayers, layer] : this.hiddenLayers.filter((l) => l !== layer);
    remember('ol-hidden-layers', this.hiddenLayers);
  }

  setTravelMode(mode: 'next' | 'all'): void {
    this.travelMode = mode;
    remember('ol-travel', mode);
  }

  private toastTimer: ReturnType<typeof setTimeout> | null = null;

  /** Shows a note; an error stays until dismissed or replaced. */
  say(text: string, error = false): void {
    this.toast = { text, error, at: Date.now() };
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastTimer = error ? null : setTimeout(() => { this.toast = null; }, 2600);
  }

  dismissToast(): void {
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toast = null;
  }

  /** Asks before something that loses work or changes the machine; true to go ahead. */
  confirm(request: ConfirmRequest): Promise<boolean> {
    this.answer(false);
    return new Promise((resolve) => { this.confirmation = { ...request, resolve }; });
  }

  answer(ok: boolean): void {
    const asked = this.confirmation;
    this.confirmation = null;
    asked?.resolve(ok);
  }
}

export const ui = new Ui();
