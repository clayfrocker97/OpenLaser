// The app: the mockup's stylesheet, the theme, then the event stream and
// the shell.
import './app.css';
import { mount } from 'svelte';
import Root from './Root.svelte';
import { ui } from './stores/ui.svelte';
import { keepsMenu } from './lib/touch';

ui.applyTheme(ui.theme);
// A held finger on a touchscreen is a right-click, and the web view's own
// menu (Back, Refresh, Print) would open over the jog buttons. Text fields
// keep theirs, for paste. iOS does not send contextmenu for its copy/paste
// callout, so cancel the touch on a button that is held.
window.addEventListener('contextmenu', (event) => { if (!keepsMenu(event.target)) event.preventDefault(); });
const stopCallout = (event: TouchEvent) => { const node = event.target as Element | null; if (event.cancelable && node?.closest?.('.hold-btn, .deadman, .jog > button:not(.hub), .zcol > button:not(.hub)')) event.preventDefault(); };
document.addEventListener('touchstart', stopCallout, { capture: true, passive: false });
document.addEventListener('touchmove', stopCallout, { capture: true, passive: false });
mount(Root, { target: document.getElementById('app')! });
