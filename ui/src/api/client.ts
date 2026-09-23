// The typed fetch client and the event stream. Every physical action is a
// POST the server admits; the UI only asks.
import type { PostflightReview, ExecutionView, RecoveryChange, TableRequest, Preview, EditHistory, PendingDraft, MergeReview, PreflightConfirmation, PreflightIntent, PreflightPreferences, PreflightReview, JobPreflight, HistoryPage, HoldTimes, Document, DraftView, Lease, Features, LeadOverride, ItemChange, JogRequest, LaserMode, NewRecipe, RecipeImport, RecipePreview, OutputRequest, Anchor, PickView, RecipeChange, RouteChange, Transform } from './index';
import { server } from '../stores/server.svelte';
import type { SheetPage, SheetView, SaveRemnant, NestSheetPreview, CorrectionView, CorrectionChange, NestRequest, NestView, StockChoice } from './index';
import type { PlacementChange, SimplifyView, ImportOptions, ImportReview } from './index';
import type { Calibration, FlowEstimate, GasCosts, GasKind, Nozzle, RunRecord } from './index';

/** A flow test: gas, gauge pressure, nozzle and seconds open. */
export type FlowTest = { gas: GasKind; pressure: number; nozzle: Nozzle; seconds: number };
import type { MachineSettings, XmlField } from '../lib/machine-settings';
import { deviceName, pageId } from '../lib/identity';

/** A refusal, with the server's reason. */
export class ApiError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
  }
}

/** The query of a reviewed import: every option that is set. */
export function importQuery(options: RecipeImport): string {
  return Object.entries(options).filter(([, v]) => v !== undefined && v !== null && v !== false).map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join('&');
}

async function request<T>(method: 'GET' | 'POST' | 'DELETE', path: string, body?: unknown, raw?: BodyInit): Promise<T> {
  const headers: Record<string, string> = { 'x-openlaser-client': pageId, 'x-openlaser-name': deviceName() };
  const init: RequestInit = { method, headers };
  if (raw !== undefined) {
    init.body = raw;
  } else if (body !== undefined) {
    init.body = JSON.stringify(body);
    headers['content-type'] = 'application/json';
  }
  const response = await fetch(path, init);
  const text = await response.text();
  let json: unknown = null;
  try { json = text ? JSON.parse(text) : null; } catch { json = null; }
  if (!response.ok) {
    const reason = (json && typeof json === 'object' && 'error' in json) ? String((json as { error: unknown }).error) : response.statusText;
    throw new ApiError(response.status, reason);
  }
  return json as T;
}

const post = <T = { ok: true }>(path: string, body?: unknown) => request<T>('POST', path, body);
const del = (path: string) => request<{ ok: true }>('DELETE', path);

type DraftReply = { ok: true; draft: DraftView | null; draft_revision: number };
async function draftPost<T = object>(path: string, body?: unknown, revision = server.doc?.draft?.revision): Promise<DraftReply & T> {
  if (revision === undefined) throw new ApiError(409, 'Open a part first.');
  const reply = await post<DraftReply & T>(`${path}?revision=${revision}`, body);
  server.applyDraft(reply.draft, reply.draft_revision);
  return reply;
}

async function openDraft(path: string, body?: unknown): Promise<DraftReply> {
  const reply = await post<DraftReply>(path, body);
  server.applyDraft(reply.draft, reply.draft_revision);
  return reply;
}

const partImportQuery = (name: string, options?: ImportOptions): string =>
  `name=${encodeURIComponent(name)}${options && (options.layers || options.scale) ? `&options=${encodeURIComponent(JSON.stringify(options))}` : ''}`;

/** The machine actions the server accepts. */
export type MachineAction = 'pulse' | 'gas-test' | 'gas-calibration' | 'table' | 'connect' | 'cancel' | 'disconnect' | 'home' | 'calibrate' | 'jog' | 'go-origin' | 'go-xy' | 'frame' | 'release' | 'heartbeat' | 'outputs' | 'mode' | 'relieve' | 'run' | 'resume' | 'hold' | 'stop';

export interface TextOptions {
  value: string;
  family: 'sans' | 'serif' | 'mono';
  size: number;
  bold: boolean;
  alignment: 'left' | 'center' | 'right';
  font?: string;
}
export interface FontFace { id: string; family: string; name: string; weight: number; style: string; stretch: string }
export interface TextPreview { outline: number[][][]; width: number; height: number; contours: number; warnings: string[] }

export const api = {
  placement: (change: PlacementChange) => draftPost('/api/draft/placement', change),
  preparePlacement: () => draftPost('/api/draft/placement/prepare'),
  sheets: (query: { remnants?: boolean; before?: string } = {}) => request<SheetPage>('GET', `/api/sheets?remnants=${query.remnants ?? false}${query.before ? `&before=${encodeURIComponent(query.before)}` : ''}`),
  sheet: (id: string) => request<SheetView>('GET', `/api/sheets/${encodeURIComponent(id)}`),
  saveRemnant: (id: string, change: SaveRemnant) => post(`/api/sheets/${encodeURIComponent(id)}`, change),
  reportCutSheet: (id: string) => post<{ id: string }>(`/api/jobs/${encodeURIComponent(id)}/cut-sheet`),
  selectSheet: (index: number) => draftPost(`/api/draft/sheet/${index}`),
  nestSheet: (id: number, index: number) => request<NestSheetPreview>('GET', `/api/draft/nest/${id}/sheet/${index}`),
  correction: (mode: LaserMode) => request<CorrectionView>('GET', `/api/machine/correction?mode=${mode}`),
  saveCorrection: (change: CorrectionChange) => post('/api/machine/correction', change),
  correctionCoupon: async (mode: LaserMode) => {
    const reply = await post<DraftReply>('/api/machine/correction/coupon', { mode });
    server.applyDraft(reply.draft, reply.draft_revision);
    return reply;
  },
  machineSettings: () => request<MachineSettings>('GET', '/api/machine/settings'),
  saveMachineSettings: (expected: string, edits: XmlField[]) => post('/api/machine/settings', { expected, edits }),
  readMachineSettings: () => post('/api/machine/settings/read'),
  writeMachineSettings: (expected: string) => post('/api/machine/settings/write', { expected }),
  setStock: (choice: StockChoice, revision?: number) => draftPost('/api/draft/stock', choice, revision),
  nest: (choice: NestRequest, revision: number) => post<NestView>(`/api/draft/nest?revision=${revision}`, choice),
  /** `since` leaves out a live arrangement the caller already has. */
  nestStatus: (id: number, since?: number) => request<NestView>('GET', `/api/draft/nest/${id}${since === undefined ? '' : `?since=${since}`}`),
  cancelNest: (id: number) => del(`/api/draft/nest/${id}`),
  applyNest: async (id: number) => {
    const reply = await post<DraftReply>(`/api/draft/nest/${id}/apply`);
    server.applyDraft(reply.draft, reply.draft_revision);
    return reply;
  },
  recoveryProgram: () => request<{ execution: ExecutionView }>('GET', '/api/recovery/program'),
  recoveryChange: (change: RecoveryChange, revision: number) => post(`/api/recovery?revision=${revision}`, change),
  prepareRecovery: (revision: number, clearance: boolean) => post(`/api/recovery/prepare?revision=${revision}`, { clearance }),
  moveRestart: (revision: number) => post(`/api/recovery/move?revision=${revision}`),
  editHistory: () => request<EditHistory>('GET', '/api/library/history'),
  undoSaved: (back: boolean, revision: number) => post(`/api/library/history/${back ? 'undo' : 'redo'}`, { revision }),
  pendingDrafts: () => request<PendingDraft[]>('GET', '/api/drafts'),
  openRetained: (key: string) => openDraft(`/api/drafts/${encodeURIComponent(key)}`),
  discardDraft: (key: string) => del(`/api/drafts/${encodeURIComponent(key)}`),
  mergeReview: (name?: string) => request<MergeReview>('GET', `/api/draft/merge${name === undefined ? '' : `?name=${encodeURIComponent(name)}`}`),
  resolveMerge: (token: string, choices: Record<string, boolean>, name?: string) => draftPost<{ name: string }>('/api/draft/merge', { token, choices, name }),
  duplicateJob: (id: string) => post<{ ok: true; id: string }>(`/api/jobs/${id}/duplicate`),
  state: () => request<Document>('GET', '/api/state'),
  alarmHistory: (before?: string) => request<HistoryPage>('GET', `/api/alarms/history${before ? `?before=${encodeURIComponent(before)}` : ''}`),
  postflight: () => request<PostflightReview | null>('GET', '/api/postflight'),
  postflightAction: (token: string, step: number) => post('/api/postflight/action', { token, step }),
  dismissPostflight: (token: string) => post('/api/postflight/dismiss', { token }),
  preflight: (intent: PreflightIntent) => request<PreflightReview>('GET', `/api/preflight?intent=${intent}`),
  preflightPreferences: () => request<PreflightPreferences>('GET', '/api/preflight/preferences'),
  savePreflightPreferences: (preferences: PreflightPreferences) => post('/api/preflight/preferences', preferences),
  /** Hold times for every screen, refused if they changed since `expected` was read. */
  saveHoldTimes: (hold: HoldTimes, expected: HoldTimes) => post('/api/touch', { hold, expected }),
  saveGasCosts: (costs: GasCosts, expected: GasCosts) => post('/api/gas/costs', { costs, expected }),
  gasFlow: (test: FlowTest) => post<FlowEstimate>('/api/gas/flow', test),
  calibrateGas: (test: FlowTest & { measured: number }) => post<Calibration>('/api/gas/calibrate', test),
  gasRuns: (key: string, job: string | null) => request<RunRecord[]>('GET', `/api/gas/runs?key=${encodeURIComponent(key)}${job ? `&job=${encodeURIComponent(job)}` : ''}`),
  setPreflight: (policy: JobPreflight, revision?: number) => draftPost('/api/draft/preflight', policy, revision),
  preflightAction: (token: string, step: number) => post('/api/preflight/action', { token, step }),
  importSoft: (name: string, bytes: ArrayBuffer, expected?: string) => request<{ ok: true }>('POST', `/api/machine/soft?name=${encodeURIComponent(name)}${expected === undefined ? '' : `&expected=${encodeURIComponent(expected)}`}`, undefined, bytes),

  importPart: (name: string, bytes: ArrayBuffer, options?: ImportOptions) => request<{ ok: true; id: string; warnings?: string[] }>('POST', `/api/parts?${partImportQuery(name, options)}`, undefined, bytes),
  /** What importing a drawing would make, with its layers, scale, repairs and problems, without keeping it. */
  reviewPart: (name: string, bytes: ArrayBuffer, options?: ImportOptions) => request<ImportReview>('POST', `/api/parts/review?${partImportQuery(name, options)}`, undefined, bytes),
  previewText: (text: TextOptions) => post<TextPreview>('/api/text/preview', text),
  fonts: () => request<{ fonts: FontFace[] }>('GET', '/api/fonts'),
  importFont: (name: string, bytes: ArrayBuffer) => request<{ ok: true; fonts: FontFace[]; existing: boolean }>('POST', `/api/fonts?name=${encodeURIComponent(name)}`, undefined, bytes),
  createText: (name: string, text: TextOptions) => post<{ ok: true; id: string; warnings: string[] }>('/api/parts/text', { name, text }),
  updatePart: (id: string, change: ItemChange) => post(`/api/parts/${id}`, change),
  duplicatePart: (id: string) => post<{ ok: true; id: string }>(`/api/parts/${id}/duplicate`),
  /** What simplifying a part's drawing within `tolerance` mm does; `save` keeps it as a new part. */
  simplifyPart: (id: string, tolerance: number, save: boolean) => post<SimplifyView>(`/api/parts/${id}/simplify`, { tolerance, save }),
  removePart: (id: string) => del(`/api/parts/${id}`),
  addFolder: (name: string, parent: string | null) => post<{ ok: true; id: string }>('/api/folders', { name, parent }),
  updateFolder: (id: string, change: ItemChange) => post(`/api/folders/${id}`, change),
  removeFolder: (id: string) => del(`/api/folders/${id}`),
  addRecipe: (recipe: NewRecipe) => post<{ ok: true; id: string }>('/api/recipes', recipe),
  /** A vendor recipe file; `existing` when the same file was imported before. */
  /** Saves a vendor recipe file as the operator reviewed it; `existing` when the same file was already there. */
  importRecipe: (options: RecipeImport, bytes: ArrayBuffer) => request<{ ok: true; id: string; existing: boolean }>('POST', `/api/recipes/import?${importQuery(options)}`, undefined, bytes),
  /** What a vendor recipe file would add; nothing is saved. */
  previewRecipe: (name: string, bytes: ArrayBuffer) => request<{ ok: true; preview: RecipePreview }>('POST', `/api/recipes/import/preview?name=${encodeURIComponent(name)}`, undefined, bytes).then((r) => r.preview),
  setPhoto: (id: string, bytes: ArrayBuffer) => request<{ ok: true }>('POST', `/api/recipes/${id}/photo`, undefined, bytes),
  photoUrl: (sha256: string) => `/api/photos/${sha256}`,
  updateRecipe: (id: string, change: RecipeChange) => post(`/api/recipes/${id}`, change),
  /** A copy of the recipe at another thickness, with the same values. */
  duplicateRecipe: (id: string, thickness_mm: number) => post<{ ok: true; id: string }>(`/api/recipes/${id}/duplicate`, { thickness_mm }),
  removeRecipe: (id: string) => del(`/api/recipes/${id}`),
  /** The vendor's machine backup, 1390backup.xml; it replaces the last one. */
  importMachineFile: (name: string, bytes: ArrayBuffer, expected?: string) => request<{ ok: true }>('POST', `/api/machine/files?name=${encodeURIComponent(name)}${expected === undefined ? '' : `&expected=${encodeURIComponent(expected)}`}`, undefined, bytes),
  updateJob: (id: string, change: ItemChange) => post(`/api/jobs/${id}`, change),
  removeJob: (id: string) => del(`/api/jobs/${id}`),

  openPart: (id: string) => openDraft(`/api/draft/part/${id}`),
  /** A new job cutting these parts, side by side in this order. */
  openParts: (parts: string[]) => openDraft('/api/draft/parts', { parts }),
  /** Adds parts to the open job beside what is on the sheet; one undo step. */
  addParts: (parts: string[]) => draftPost('/api/draft/parts/add', { parts }),
  openJob: (id: string) => openDraft(`/api/draft/job/${id}`),
  setRecipe: (id: string) => draftPost(`/api/draft/recipe/${id}`),
  setFeatures: (features: Features, revision?: number) => draftPost('/api/draft/features', features, revision),
  copyFeatures: (jobId: string) => draftPost(`/api/draft/copy/${jobId}`),
  /** Moves, turns or mirrors placed contours by `matrix`, applied after what they have; `null` puts them back. */
  transform: (contours: number[], matrix: Transform | null, revision?: number) => draftPost('/api/draft/transform', { contours, matrix }, revision),
  /** Puts drawing contours on the sheet as one more copy; their indices. */
  add: (contours: Array<{ source: number; transform: Transform }>, lead_overrides: LeadOverride[] = [], grouping: number[][] = []) => draftPost<{ contours: number[] }>('/api/draft/add', { contours, lead_overrides, grouping }),
  /** Takes placed contours off the sheet, with the features on them. */
  remove: (contours: number[]) => draftPost('/api/draft/remove', { contours }),
  group: (contours: number[], together: boolean) => draftPost(together ? '/api/draft/group' : '/api/draft/ungroup', { contours }),
  undo: () => draftPost('/api/draft/undo'),
  redo: () => draftPost('/api/draft/redo'),
  /** A tap on the drawing snapped to a contour within `tolerance` millimetres, or `null`. */
  pick: (point: [number, number], tolerance: number, bridging: boolean, revision: number, features?: Features) => post<{ ok: true; pick: PickView | null; revision: number }>(`/api/draft/pick?revision=${revision}`, { point, tolerance, bridging, features }),
  previewFeatures: (features: Features, revision: number) => post<{ preview: Preview; revision: number }>(`/api/draft/preview?revision=${revision}`, features),
  /** Which point of the part the origin stands for. */
  setAnchor: (anchor: Anchor) => draftPost('/api/draft/anchor', { anchor }),
  /** Puts the part's anchor point at machine coordinates, or where the head is. */
  setOrigin: (origin: [number, number] | 'head', anchor?: Anchor) => draftPost('/api/draft/origin', origin === 'head' ? { anchor } : { origin, anchor }),
  compile: (dryRun: boolean) => draftPost('/api/draft/compile', { dry_run: dryRun }),
  saveJob: (name: string) => draftPost<{ id: string }>('/api/draft/save', { name }),

  /** The adapter, controller address or computer address the next Connect uses. */
  setRoute: (change: RouteChange) => post('/api/machine/route', change),
  machine: (action: MachineAction, body: { pulse?: { duration_ms: number; power: number }; gas_test?: { selector: number; pressure: number; duration_ms: number }; gas_calibration?: { selector: number; pressure: number }; table?: TableRequest; preflight?: PreflightConfirmation; lease?: Lease; jog?: JogRequest; output?: OutputRequest; mode?: LaserMode; alarm?: number | null; xy?: [number, number]; fast?: boolean } = {}) => post(`/api/machine/${action}`, body),
};

/**
 * Subscribes to the event stream. Each event is the document with the
 * library, draft and bindings sections present only when they changed.
 */
export function subscribe(onEvent: (partial: Partial<Document>, first: boolean) => void, onLink: (up: boolean) => void): () => void {
  const source = new EventSource('/api/events');
  // The first event after the stream comes up is the whole document; the
  // server may have restarted and forgotten what the page still shows.
  let first = true;
  source.onmessage = (event: MessageEvent<string>) => {
    onLink(true);
    onEvent(JSON.parse(event.data) as Partial<Document>, first);
    first = false;
  };
  source.onerror = () => { onLink(false); first = true; };
  return () => source.close();
}
