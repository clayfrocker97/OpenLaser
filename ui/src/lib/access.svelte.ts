import { deviceName, pageId, previousPageId, rememberControl } from './identity';
import { ui } from '../stores/ui.svelte';

export interface AccessStatus {
  can_control: boolean;
  owner: string | null;
  address: string;
  discovery: { enabled: boolean; ready: boolean; error: string | null };
}

class Access {
  info = $state.raw<AccessStatus | null>(null);
  error = $state('');
  busy = $state(false);
  manage = $state(false);
  private refreshing = false;

  get canControl(): boolean { return this.info?.can_control === true && !this.error; }

  private async request<T>(path: string, body?: unknown): Promise<T> {
    const response = await fetch(`/api/access${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { 'content-type': 'application/json', 'x-openlaser-client': pageId, 'x-openlaser-name': deviceName(), ...(previousPageId ? {'x-openlaser-previous':previousPageId} : {}) },
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: 'no-store',
    });
    const result = await response.json() as T & { error?: string };
    if (!response.ok) throw new Error(result.error ?? 'Could not reach OpenLaser.');
    return result;
  }

  async refresh(): Promise<void> {
    if (this.refreshing) return;
    this.refreshing = true;
    try { this.info = await this.request<AccessStatus>(''); this.error = ''; }
    catch (error) { this.error = error instanceof Error ? error.message : String(error); }
    finally { this.refreshing = false; }
  }

  async act(action: 'control' | 'release', body: unknown = {}): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    try { await this.request(`/${action}`, body); await this.refresh(); }
    finally { this.busy = false; }
  }

  mount(): () => void {
    const refresh = () => { if (!document.hidden) void this.refresh(); };
    refresh();
    const timer = setInterval(refresh, 2000);
    document.addEventListener('visibilitychange', refresh);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', refresh); };
  }
}

export const access = new Access();

export function chooseLayout(): 'mobile' | 'full' {
  if (location.pathname.startsWith('/mobile')) return 'mobile';
  if (location.pathname.startsWith('/app')) return 'full';
  try {
    const preferred = localStorage.getItem('ol-layout');
    if (preferred === 'mobile' || preferred === 'full') return preferred;
  } catch { /* A private browser can still use either explicit address. */ }
  return matchMedia('(max-width: 699px), (pointer: coarse) and (max-height: 500px) and (max-width: 950px)').matches ? 'mobile' : 'full';
}

export function rememberLayout(layout: 'mobile' | 'full'): void {
  if (access.canControl) rememberControl();
  try {
    sessionStorage.setItem('ol-tab-return', ui.tab);
    sessionStorage.setItem('ol-settings-return', String(ui.machinePage));
  } catch { /* Page choice is optional. */ }
  try { localStorage.setItem('ol-layout', layout); } catch { /* The explicit address remains usable. */ }
}
