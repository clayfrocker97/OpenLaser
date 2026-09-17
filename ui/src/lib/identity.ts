/** Random page identity also supported at a plain HTTP address on the local network. */
export function randomId(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
}

/** One control identity per page. */
export const pageId = randomId();

/** A reload or an explicit layout switch continues this tab's previous claim. */
export const previousPageId: string | null = (() => {
  try {
    const previous = sessionStorage.getItem('ol-page-id');
    const returning = sessionStorage.getItem('ol-control-return');
    sessionStorage.removeItem('ol-control-return');
    sessionStorage.setItem('ol-page-id', pageId);
    const navigation = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined;
    return returning ?? (navigation?.type === 'reload' || navigation?.type === 'back_forward' ? previous : null);
  } catch { return null; }
})();

export function rememberControl(): void {
  try { sessionStorage.setItem('ol-control-return', pageId); } catch { /* Take control remains available. */ }
}

/** A readable device label for the screen handoff popup. */
export function deviceName(): string {
  const agent = navigator.userAgent;
  if (/iPhone/i.test(agent)) return 'iPhone';
  if (/iPad/i.test(agent) || /Macintosh/i.test(agent) && navigator.maxTouchPoints > 1) return 'iPad';
  if (/Android/i.test(agent)) return /Mobile/i.test(agent) ? 'Android phone' : 'Android tablet';
  if (/Windows/i.test(agent)) return 'Windows PC';
  if (/Macintosh|Mac OS X/i.test(agent)) return 'Mac';
  if (/Linux/i.test(agent)) return 'Linux computer';
  return 'Another device';
}
