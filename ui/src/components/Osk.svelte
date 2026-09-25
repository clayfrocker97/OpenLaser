<script lang="ts">
  import { osk } from '../lib/osk.svelte';
  import { ui } from '../stores/ui.svelte';

  const ROWS_TEXT = [['1','2','3','4','5','6','7','8','9','0'], ['q','w','e','r','t','y','u','i','o','p'], ['a','s','d','f','g','h','j','k','l'], ['z','x','c','v','b','n','m',',','-','_','.']];
  const ROWS_NUM = [['7','8','9'],['4','5','6'],['1','2','3'],['.','0','bs'],['−','clear','done']];

  // Pinned spots are remembered per page and per keyboard type.
  let element = $state<HTMLDivElement | null>(null);
  let pinned = $state(false);
  const key = () => `ol-osk-${ui.tab}-${osk.kind}`;

  function place(): void {
    if (!element) return;
    let saved: { x: number; y: number } | null = null;
    try { saved = JSON.parse(localStorage.getItem(key()) ?? 'null') as { x: number; y: number } | null; } catch { saved = null; }
    pinned = !!saved;
    element.classList.toggle('pinned', pinned);
    if (saved) { element.style.left = `${saved.x}px`; element.style.top = `${saved.y}px`; element.style.bottom = 'auto'; }
    else { element.style.left = ''; element.style.top = ''; element.style.bottom = ''; }
    clamp();
  }

  function clamp(): void {
    if (!element || !element.classList.contains('pinned')) return;
    const r = element.getBoundingClientRect();
    element.style.left = `${Math.min(Math.max(8, r.left), innerWidth - r.width - 8)}px`;
    element.style.top = `${Math.min(Math.max(8, r.top), innerHeight - r.height - 8)}px`;
  }

  function pin(): void {
    if (!element) return;
    if (pinned) { try { localStorage.removeItem(key()); } catch { /* ignore */ } ui.say('Keyboard unpinned.'); }
    else { const r = element.getBoundingClientRect(); try { localStorage.setItem(key(), JSON.stringify({ x: r.left, y: r.top })); } catch { /* ignore */ } ui.say('Keyboard pinned here for this page.'); }
    place();
  }

  let drag: { ox: number; oy: number } | null = null;
  function down(e: PointerEvent): void {
    if ((e.target as HTMLElement).closest('button') || !element) return;
    const r = element.getBoundingClientRect();
    element.classList.add('pinned', 'dragging');
    element.style.left = `${r.left}px`; element.style.top = `${r.top}px`; element.style.bottom = 'auto';
    drag = { ox: e.clientX - r.left, oy: e.clientY - r.top };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent): void {
    if (!drag || !element) return;
    element.style.left = `${e.clientX - drag.ox}px`; element.style.top = `${e.clientY - drag.oy}px`;
  }
  function up(): void {
    if (!drag || !element) return;
    drag = null; element.classList.remove('dragging'); clamp();
    if (pinned) { const r = element.getBoundingClientRect(); try { localStorage.setItem(key(), JSON.stringify({ x: r.left, y: r.top })); } catch { /* ignore */ } }
  }

  $effect(() => { if (osk.open) { document.body.classList.add('osk-open'); place(); } else document.body.classList.remove('osk-open'); });

  // Only editable fields use our keyboard. Set inputmode before focus so
  // the browser does not request a second, native touch keyboard.
  $effect(() => {
    const editable = (t: EventTarget | null): t is HTMLInputElement | HTMLTextAreaElement =>
      (t instanceof HTMLTextAreaElement || t instanceof HTMLInputElement && ['text', 'search', 'email', 'url', 'tel', 'password', 'number'].includes(t.type))
      && !t.readOnly && !t.disabled;
    const configure = (root: ParentNode) => { root.querySelectorAll('input, textarea').forEach(input => { if (editable(input)) input.inputMode = 'none'; }); };
    configure(document);
    const observer = new MutationObserver(records => { for (const record of records) for (const node of record.addedNodes) if (node instanceof HTMLElement) { if (editable(node)) node.inputMode = 'none'; configure(node); } });
    observer.observe(document.body, { childList: true, subtree: true });
    const focus = (e: FocusEvent) => { const t = e.target; if (editable(t)) osk.show({ kind: t.type === 'number' ? 'num' : 'text', target: t, value: t.value, label: t.placeholder || t.getAttribute('aria-label') || 'Type' }); };
    // A tap anywhere else puts either keyboard away, unsaved, so a numpad
    // opened for one value is not left over the jog buttons. A tap on
    // another value opens the keyboard again for that one.
    const tap = (e: PointerEvent) => { const t = e.target as HTMLElement; if (osk.open && !t.closest('#osk') && !t.closest('input, textarea') && !t.closest('[data-numpad]')) osk.close(); };
    const input = (e: Event) => { if (e.target === osk.target && osk.target) osk.value = osk.target.value; };
    const keydown = (e: KeyboardEvent) => {
      if (!osk.open || e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); osk.close(); return; }
      if (osk.target) return;
      const key = e.key === 'Enter' ? 'done' : e.key === 'Backspace' ? 'bs' : e.key === 'Delete' ? 'clear' : e.key === '-' && osk.kind === 'num' ? '−' : e.key;
      if (['done', 'bs', 'clear', '−'].includes(key) || (key.length === 1 && (osk.kind === 'text' || /^[0-9.]$/.test(key)))) { e.preventDefault(); e.stopImmediatePropagation(); osk.key(key); }
    };
    document.addEventListener('focusin', focus);
    document.addEventListener('pointerdown', tap, true);
    document.addEventListener('input', input);
    window.addEventListener('keydown', keydown, true);
    addEventListener('resize', clamp);
    return () => {
      observer.disconnect();
      document.removeEventListener('focusin', focus);
      document.removeEventListener('pointerdown', tap, true);
      document.removeEventListener('input', input);
      window.removeEventListener('keydown', keydown, true);
      removeEventListener('resize', clamp);
    };
  });

  const label = (k: string) => k === 'bs' ? '<i class="ic ic-backspace"></i>' : k === 'done' ? 'Done' : k === 'clear' ? 'Clear' : k === 'shift' ? 'Shift' : k === ' ' ? 'Space' : (osk.shift ? k.toUpperCase() : k);
</script>

{#if osk.open}
  <div class="osk {osk.kind}" id="osk" bind:this={element}>
    <div class="osk-head" role="toolbar" tabindex="-1" aria-label="Keyboard" onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}>
      <i class="ic ic-grip grip"></i>
      <span class="osk-label">{osk.label}</span>
      <div class="osk-echo">{#if osk.value}{osk.value}{:else}<span class="muted">…</span>{/if}{#if osk.unit}<span class="unit">{osk.unit}</span>{/if}</div>
      <button class="btn btn-ghost pin" class:on={pinned} onclick={pin} title="Pin position for this page"><i class="ic ic-pin"></i></button>
      <button class="btn btn-ghost" onclick={() => osk.close()} aria-label="Close keyboard"><i class="ic ic-x"></i></button>
    </div>
    <div class="osk-rows">
      {#if osk.kind === 'num'}
        {#each ROWS_NUM as row}
          <div class="osk-row">{#each row as k}<button class="osk-key" class:done={k === 'done'} class:wide={k === 'clear'} aria-label={k} onclick={() => osk.key(k)}>{@html label(k)}</button>{/each}</div>
        {/each}
      {:else}
        {#each ROWS_TEXT as row, i}
          <div class="osk-row" class:indent={i === 2}>
            {#if i === 3}<button class="osk-key wide" class:on={osk.shift} onclick={() => osk.key('shift')}>Shift</button>{/if}
            {#each row as k}<button class="osk-key" onclick={() => osk.key(k)}>{osk.shift ? k.toUpperCase() : k}</button>{/each}
            {#if i === 3}<button class="osk-key wide" aria-label="Backspace" onclick={() => osk.key('bs')}><i class="ic ic-backspace"></i></button>{/if}
          </div>
        {/each}
        <div class="osk-row"><button
            class="osk-key wide" onclick={() => osk.key('clear')}>Clear</button><button
            class="osk-key space" onclick={() => osk.key(' ')}>Space</button>{#if osk.multiline}<button
            class="osk-key wide" onclick={() => osk.key('\n')}>New line</button>{/if}<button
            class="osk-key done wide" onclick={() => osk.key('done')}>Done</button></div>
      {/if}
    </div>
  </div>
{/if}
