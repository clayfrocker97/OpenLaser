// The app: the mockup's stylesheet, the theme, then the event stream and
// the shell.
import './app.css';
import { mount } from 'svelte';
import Root from './Root.svelte';
import { ui } from './stores/ui.svelte';
import { keepsMenu, suppressHeldCallout } from './lib/touch';

ui.applyTheme(ui.theme);
// A held finger on a touchscreen is a right-click, and the web view's own
// menu (Back, Refresh, Print) would open over the jog buttons. Text fields
// keep theirs, for paste. iOS never sends this event for the copy/paste
// callout; suppressHeldCallout covers that gesture.
window.addEventListener('contextmenu', (event) => { if (!keepsMenu(event.target)) event.preventDefault(); });
const heldTouch = { capture: true, passive: false } as const;
document.addEventListener('touchstart', suppressHeldCallout, heldTouch);
document.addEventListener('touchmove', suppressHeldCallout, heldTouch);
mount(Root, { target: document.getElementById('app')! });
