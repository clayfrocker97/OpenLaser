<script lang="ts">
  // A control that moves the machine, fires the beam or sets a reference
  // acts only after a press is held (DESIGN.md, "Touch rules"). The fill
  // shows the hold; releasing early, dragging off or losing the pointer does
  // nothing. Enter and Space must be held too.
  import { onMount, type Snippet } from 'svelte';
  import { HOLD_MS, HoldConfirm, type HoldPhase } from '../lib/hold-confirm';
  import { server } from '../stores/server.svelte';

  let {
    onhold,
    kind = 'move',
    disabled = false,
    class: className = 'btn btn-move',
    title = '',
    label,
    children,
    ...attributes
  }: {
    onhold: () => unknown;
    kind?: keyof typeof HOLD_MS;
    disabled?: boolean;
    class?: string;
    title?: string;
    label?: string;
    children: Snippet;
    'aria-pressed'?: boolean;
  } = $props();

  let phase = $state<HoldPhase>('idle');
  let progress = $state(0);
  // The shared setting (Settings → Display), read at every press.
  const duration = () => server.doc?.hold[kind === 'move' ? 'move_ms' : 'zero_ms'] ?? HOLD_MS[kind];
  const hold = new HoldConfirm(duration, () => { onhold(); }, (next, fill) => { phase = next; progress = fill; });
  const keys = new Set(['Enter', ' ']);
  const hint = $derived(phase === 'early' ? 'Press and hold' : phase === 'done' ? 'Done' : 'Hold');

  $effect(() => { if (disabled) hold.cancel(); });
  onMount(() => {
    const hidden = () => { if (document.hidden) hold.cancel(); };
    window.addEventListener('blur', hold.cancel);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      hold.cancel();
      window.removeEventListener('blur', hold.cancel);
      document.removeEventListener('visibilitychange', hidden);
    };
  });

  function down(event: PointerEvent & { currentTarget: HTMLButtonElement }): void {
    if (event.button !== 0 || disabled) return;
    if (!hold.begin(event.pointerId, event.clientX, event.clientY)) return;
    // Capture keeps the release even off the button; without it, leaving the
    // button abandons the hold instead.
    try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* leave cancels */ }
  }

  function leave(event: PointerEvent & { currentTarget: HTMLButtonElement }): void {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) hold.cancel();
  }

  function move(event: PointerEvent & { currentTarget: HTMLButtonElement }): void {
    const box = event.currentTarget.getBoundingClientRect();
    const inside = event.clientX >= box.left && event.clientX <= box.right && event.clientY >= box.top && event.clientY <= box.bottom;
    hold.move(event.pointerId, event.clientX, event.clientY, inside);
  }
</script>

<button
  {...attributes}
  class="{className} hold-btn hold-{phase}"
  style:--hold={progress}
  {disabled}
  {title}
  aria-label={label}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={(event) => hold.end(event.pointerId)}
  onpointerleave={leave}
  onpointercancel={hold.cancel}
  onlostpointercapture={hold.cancel}
  onkeydown={(event) => { if (!keys.has(event.key)) return; event.preventDefault(); if (!event.repeat && !disabled) hold.begin('key'); }}
  onkeyup={(event) => { if (!keys.has(event.key)) return; event.preventDefault(); hold.end('key'); }}
  onblur={hold.cancel}
  onclick={(event) => event.preventDefault()}
  oncontextmenu={(event) => event.preventDefault()}
>{@render children()}<small class="hold-hint">{hint}</small></button>
