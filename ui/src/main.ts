// The app: the mockup's stylesheet, the theme, then the event stream and
// the shell.
import './app.css';
import { mount } from 'svelte';
import Root from './Root.svelte';
import { ui } from './stores/ui.svelte';

ui.applyTheme(ui.theme);
mount(Root, { target: document.getElementById('app')! });
