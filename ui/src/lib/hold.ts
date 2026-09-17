import type { Lease } from '../api';
import { randomId } from './identity';

let client: string | undefined;
let sequence = 0;
const nextLease = (): Lease => ({ client: client ??= randomId(), sequence: ++sequence });

type Capture = Pick<HTMLElement, 'setPointerCapture' | 'hasPointerCapture' | 'releasePointerCapture'>;
type Press = { lease: Lease; pointer: number; target: Capture; timer?: ReturnType<typeof setTimeout> };
type Transport = { heartbeat: (lease: Lease) => Promise<unknown>; release: (lease: Lease) => Promise<unknown> };

/** A press owns its start, renewal and terminal release, even across delayed replies. */
export class Hold {
  private active: Press | null = null;

  constructor(private transport: Transport, private error: (error: unknown) => void, private available = () => true) {}

  press(event: Pick<PointerEvent, 'button' | 'pointerId' | 'currentTarget'>, start: (lease: Lease) => Promise<unknown>): void {
    if (event.button !== 0 || this.active || !this.available()) return;
    const press: Press = { lease: nextLease(), pointer: event.pointerId, target: event.currentTarget as HTMLElement };
    this.active = press;
    try { press.target.setPointerCapture(press.pointer); }
    catch (error) { this.cancel(); this.error(error); return; }
    void start(press.lease).then(() => {
      if (this.active === press) this.renew(press);
    }).catch((error: unknown) => {
      if (this.active === press) { this.cancel(); this.error(error); }
    });
  }

  private renew(press: Press): void {
    // The shortest supported head-jog lease is 300 ms. Never overlap renewals.
    press.timer = setTimeout(() => {
      if (this.active !== press) return;
      void this.transport.heartbeat(press.lease).then(() => {
        if (this.active === press) this.renew(press);
      }).catch((error: unknown) => {
        if (this.active === press) { this.cancel(); this.error(error); }
      });
    }, 75);
  }

  release = (event: Pick<PointerEvent, 'pointerId'>): void => {
    if (this.active?.pointer === event.pointerId) this.cancel();
  };

  cancel = (): void => {
    const press = this.active;
    if (!press) return;
    this.active = null;
    clearTimeout(press.timer);
    try {
      if (press.target.hasPointerCapture(press.pointer)) press.target.releasePointerCapture(press.pointer);
    } catch { /* A detached target still needs its terminal release. */ }
    // Send immediately, even if admission is pending. The server remembers the release.
    void this.transport.release(press.lease).catch(this.error);
  };

  /** Installed once per component; disposal releases before removing listeners. */
  mount(win: Window = window, doc: Document = document): () => void {
    const hidden = () => { if (doc.hidden) this.cancel(); };
    win.addEventListener('pointerup', this.release);
    win.addEventListener('pointercancel', this.release);
    win.addEventListener('lostpointercapture', this.release);
    win.addEventListener('blur', this.cancel);
    doc.addEventListener('visibilitychange', hidden);
    return () => {
      this.cancel();
      win.removeEventListener('pointerup', this.release);
      win.removeEventListener('pointercancel', this.release);
      win.removeEventListener('lostpointercapture', this.release);
      win.removeEventListener('blur', this.cancel);
      doc.removeEventListener('visibilitychange', hidden);
    };
  }
}
