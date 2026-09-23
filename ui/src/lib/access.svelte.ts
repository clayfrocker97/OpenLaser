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

type Layout = 'mobile' | 'full';
/** Windows narrower than this, or a small touch landscape, get the phone layout. */
const PHONE_QUERY = '(max-width: 699px), (pointer: coarse) and (max-height: 500px) and (max-width: 950px)';

/** A layout the address fixes: /mobile or /app. */
function pinnedLayout(): Layout | null {
  if (location.pathname.startsWith('/mobile')) return 'mobile';
  if (location.pathname.startsWith('/app')) return 'full';
  return null;
}

export function chooseLayout(): Layout {
  const pinned = pinnedLayout();
  if (pinned) return pinned;
  try {
    const preferred = localStorage.getItem('ol-layout');
    if (preferred === 'mobile' || preferred === 'full') return preferred;
  } catch { /* A private browser can still use either explicit address. */ }
  return matchMedia(PHONE_QUERY).matches ? 'mobile' : 'full';
}

/**
 * The layout in use. It follows the window: resizing across the phone
 * width switches layouts at once, without a reload, unless the address
 * pins one. Crossing the width also drops a remembered choice, since the
 * window now says which layout fits.
 */
class LayoutChoice {
  current = $state<Layout>(chooseLayout());

  mount(): () => void {
    const query = matchMedia(PHONE_QUERY);
    const follow = (event: MediaQueryListEvent) => {
      if (pinnedLayout()) return;
      try { localStorage.removeItem('ol-layout'); } catch { /* Nothing was remembered. */ }
      this.current = event.matches ? 'mobile' : 'full';
    };
    query.addEventListener('change', follow);
    return () => query.removeEventListener('change', follow);
  }
}

export const layout = new LayoutChoice();

export function rememberLayout(layout: 'mobile' | 'full'): void {
  if (access.canControl) rememberControl();
  try {
    sessionStorage.setItem('ol-tab-return', ui.tab);
    sessionStorage.setItem('ol-settings-return', String(ui.machinePage));
  } catch { /* Page choice is optional. */ }
  try { localStorage.setItem('ol-layout', layout); } catch { /* The explicit address remains usable. */ }
}
