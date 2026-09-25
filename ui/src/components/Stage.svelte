<script lang="ts">
  import { toSource, toDisplay } from '../lib/units.svelte';
  // The drawing surface both canvases share: a grid, the bed, the job
  // origin, the head, and the gestures. A mouse button, a pen or one
  // finger selects: a tap picks, a drag moves whatever the parent lets it
  // grab, and a drag over empty space sweeps a marquee. Two fingers pan
  // and pinch the view, a middle or right button drags it, and the wheel
  // zooms about the cursor. Children draw in drawing millimetres, Y up.
  import { untrack, type Snippet } from 'svelte';
  import type { Viewport } from '../lib/viewport.svelte';
  import type { Box } from '../lib/svg';

  type Point = [number, number];
  let {
    view,
    bed = null,
    head = null,
    origin = null,
    grab = () => null,
    ondrag,
    ondragend,
    ontap,
    onmarquee,
    children,
    lift = null,
    lifted,
    overlay,
    variant = '',
  }: {
    view: Viewport;
    /** The machine's travel in job coordinates. */
    bed?: Box | null;
    /** The head, in machine coordinates. */
    head?: Point | null;
    /** The job origin, in machine coordinates. */
    origin?: Point | null;
    /** What a selecting pointer takes hold of at `target`, or nothing. */
    grab?: (target: Element, at: Point) => string | null;
    ondrag?: (kind: string, from: Point, to: Point) => void;
    ondragend?: (kind: string, cancelled: boolean) => void;
    /** A tap, and whether it adds to a selection. */
    ontap?: (at: Point, target: Element, additive: boolean) => void;
    /** A marquee swept over empty space, in drawing coordinates. */
    onmarquee?: (box: Box, additive: boolean) => void;
    children?: Snippet;
    /** How far the lifted layer has moved, in drawing millimetres, while it shows. */
    lift?: Point | null;
    /** Shapes drawn once on a layer of their own that `lift` slides. */
    lifted?: Snippet;
    /** HTML over the drawing, given a function from drawing points to pixel offsets inside the canvas. */
    overlay?: Snippet<[(p: Point) => Point]>;
    variant?: string;
  } = $props();

  const uid = Math.random().toString(36).slice(2, 8);
  let svg = $state<SVGSVGElement | null>(null);
  let rect: DOMRect | null = null;
  const invalidateRect = () => { rect = null; };
  // The frame, not the drawing, which a gesture moves by a transform.
  const screenRect = (): DOMRect => rect ??= svg!.parentElement!.getBoundingClientRect();

  $effect(() => { view.setLimit(bed); });
  $effect(() => {
    if (!svg) return;
    const measure = () => { rect = svg!.parentElement!.getBoundingClientRect(); view.resize(rect.width, rect.height); };
    untrack(measure);
    const observer = new ResizeObserver(measure);
    observer.observe(svg);
    window.addEventListener('scroll', invalidateRect, true);
    window.addEventListener('resize', invalidateRect);
    return () => {
      observer.disconnect();
      window.removeEventListener('scroll', invalidateRect, true);
      window.removeEventListener('resize', invalidateRect);
    };
  });

  const toSvg = (x: number, y: number): Point => {
    const r = screenRect();
    const k = Math.max(view.w / r.width, view.h / r.height);
    // Match xMidYMid meet, including a transient letterbox during a resize.
    return [view.x + (x - r.left - (r.width - view.w / k) / 2) * k,
      view.y + (y - r.top - (r.height - view.h / k) / 2) * k];
  };
  const toDrawing = (x: number, y: number): Point => { const [sx, sy] = toSvg(x, y); return [sx, -sy]; };
  const distance = (a: Point, b: Point) => Math.hypot(b[0] - a[0], b[1] - a[1]);
  const middle = (a: Point, b: Point): Point => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  const boxOf = (a: Point, b: Point): Box => ({ minX: Math.min(a[0], b[0]), minY: Math.min(a[1], b[1]), maxX: Math.max(a[0], b[0]), maxY: Math.max(a[1], b[1]) });

  const pointers = new Map<number, Point>();
  type Gesture =
    | { kind: 'pan'; last: Point }
    | { kind: 'drag'; grabbed: string; from: Point; moved: boolean }
    | { kind: 'marquee'; from: Point; moved: boolean; additive: boolean; target: Element }
    | { kind: 'pinch'; anchor: Point; width: number; span: number };
  let gesture: Gesture | null = null;
  /** The marquee being swept, for drawing. */
  let marquee = $state<Box | null>(null);
  let pendingMove: PointerEvent | null = null;
  let pendingWheels: Array<{ factor: number; x: number; y: number }> = [];
  let animation = 0;
  function flush(): void {
    if (animation) cancelAnimationFrame(animation);
    animation = 0;
    for (const wheel of pendingWheels) view.zoom(wheel.factor, toSvg(wheel.x, wheel.y));
    if (pendingWheels.length) viewMoved();
    pendingWheels = [];
    const event = pendingMove;
    pendingMove = null;
    if (event) updateGesture(event);
  }
  function schedule(): void { if (!animation) animation = requestAnimationFrame(flush); }

  // A zoom or pan moves the picture already drawn, which the graphics card
  // does for free, and draws it again a few times a second and once it
  // stops: redrawing thousands of shapes every frame is what slows a small
  // PC down.
  const REDRAW_MS = 250;
  const SETTLE_MS = 120;
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  let lastDrawn = 0;
  function viewMoved(): void {
    view.hold();
    const now = performance.now();
    if (now - lastDrawn > REDRAW_MS) { lastDrawn = now; view.settle(); }
    clearTimeout(settleTimer);
    settleTimer = setTimeout(settle, SETTLE_MS);
  }
  function settle(): void {
    clearTimeout(settleTimer);
    // A pinch or pan still under way keeps holding.
    if (gesture?.kind === 'pinch' || gesture?.kind === 'pan') { settleTimer = setTimeout(settle, SETTLE_MS); view.settle(); lastDrawn = performance.now(); return; }
    lastDrawn = performance.now();
    view.release();
  }
  $effect(() => () => { clearTimeout(settleTimer); view.release(); });
  /** Moves the drawn picture from where it was drawn to where the view is. */
  const lag = $derived.by(() => {
    const s = view.shown;
    if (s.x === view.x && s.y === view.y && s.w === view.w) return undefined;
    const k = view.pixelWidth / view.w;
    return `translate(${(s.x - view.x) * k}px, ${(s.y - view.y) * k}px) scale(${s.w / view.w})`;
  });
  const liftTransform = $derived.by(() => {
    if (!lift) return undefined;
    const k = view.pixelWidth / view.w;
    return `translate(${lift[0] * k}px, ${-lift[1] * k}px) ${lag ?? ''}`;
  });
  function cancel(): void {
    if (animation) cancelAnimationFrame(animation);
    animation = 0;
    pendingMove = null;
    pendingWheels = [];
    const current = gesture;
    gesture = null;
    pointers.clear();
    marquee = null;
    if (current?.kind === 'drag') ondragend?.(current.grabbed, true);
  }
  $effect(() => () => cancel());
  /** A drag distance that counts as movement, not a tap: a fingertip wobbles
   *  further than a mouse (DESIGN.md, Touch rules). */
  const slop = (e: PointerEvent) => (e.pointerType === 'mouse' ? 4 : 12) * view.mmPerPixel;

  function down(e: PointerEvent): void {
    if (!svg) return;
    flush();
    // A new contact also handles a canvas moved without changing its size.
    rect = svg.getBoundingClientRect();
    try { svg.setPointerCapture(e.pointerId); } catch { /* a pointer already gone */ }
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    if (e.pointerType === 'touch' && pointers.size === 2) {
      // A second finger: the view is pinched from here, whatever was happening.
      if (gesture?.kind === 'drag') ondragend?.(gesture.grabbed, true);
      marquee = null;
      const [a, b] = [...pointers.values()] as [Point, Point];
      const mid = middle(a, b);
      gesture = { kind: 'pinch', anchor: toSvg(mid[0], mid[1]), width: view.w, span: distance(a, b) };
      return;
    }
    if (pointers.size > 1) return;
    if (e.pointerType === 'mouse' && e.button !== 0) {
      gesture = { kind: 'pan', last: [e.clientX, e.clientY] };
      return;
    }
    const from = toDrawing(e.clientX, e.clientY);
    const grabbed = grab(e.target as Element, from);
    gesture = grabbed
      ? { kind: 'drag', grabbed, from, moved: false }
      : { kind: 'marquee', from, moved: false, additive: e.shiftKey, target: e.target as Element };
  }

  function move(e: PointerEvent): void {
    if (!gesture || !pointers.has(e.pointerId) || !svg) return;
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    pendingMove = e;
    schedule();
  }

  function updateGesture(e: PointerEvent): void {
    if (!gesture || !svg) return;
    if (gesture.kind === 'pinch') {
      if (pointers.size < 2) return;
      const [a, b] = [...pointers.values()] as [Point, Point];
      const scale = distance(a, b) / Math.max(gesture.span, 1);
      view.place(gesture.width / scale, gesture.anchor, middle(a, b), screenRect());
      viewMoved();
    } else if (gesture.kind === 'pan') {
      const k = view.mmPerPixel;
      view.pan(-(e.clientX - gesture.last[0]) * k, -(e.clientY - gesture.last[1]) * k);
      gesture.last = [e.clientX, e.clientY];
      viewMoved();
    } else {
      const to = toDrawing(e.clientX, e.clientY);
      gesture.moved ||= distance(gesture.from, to) > slop(e);
      if (!gesture.moved) return;
      if (gesture.kind === 'drag') ondrag?.(gesture.grabbed, gesture.from, to);
      else marquee = boxOf(gesture.from, to);
    }
  }

  function up(e: PointerEvent): void {
    if (!pointers.has(e.pointerId)) return;
    // Commit the release position even if it arrived before the next frame.
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    pendingMove = e;
    flush();
    if (!pointers.delete(e.pointerId) || !gesture) return;
    if (gesture.kind === 'pinch') {
      if (pointers.size === 0) gesture = null;
      return;
    }
    const g = gesture;
    gesture = null;
    const swept = marquee;
    marquee = null;
    if (g.kind === 'pan') return;
    const target = document.elementFromPoint(e.clientX, e.clientY) ?? (e.target as Element);
    if (g.kind === 'drag') {
      ondragend?.(g.grabbed, !g.moved);
      if (!g.moved) ontap?.(g.from, target, e.shiftKey);
    } else if (g.moved && swept) {
      onmarquee?.(swept, g.additive);
    } else {
      ontap?.(g.from, g.target, e.shiftKey);
    }
  }

  function wheel(e: WheelEvent): void {
    e.preventDefault();
    // A trackpad pinch arrives as a wheel with the control key and small deltas.
    pendingWheels.push({ factor: Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), x: e.clientX, y: e.clientY });
    schedule();
  }

  /** The pixel offset inside the canvas of a drawing point. */
  const project = (p: Point): Point => [(p[0] - view.x) / view.mmPerPixel, (-p[1] - view.y) / view.mmPerPixel];

  // Paint only the camera rectangle. Pattern tiles far outside it can become
  // enormous raster surfaces when a small part is viewed close up.
  const grid = $derived(toSource(10 ** Math.ceil(Math.log10(toDisplay(view.shownMmPerPixel * 8, 'mm'))), 'mm'));
  const coarseGrid = $derived(grid * 10);
  /** A stroke-independent size in drawing units: the smaller of the view's dimensions over 90. */
  const unit = $derived(Math.min(view.shown.w, view.shown.h) / 90);
  const shown = $derived(view.shown);
</script>

<div class="canvas {variant}">
  <svg
    bind:this={svg} viewBox={view.viewBox} style:transform={lag} preserveAspectRatio="xMidYMid meet" role="application" aria-label="Drawing"
    onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={cancel}
    onlostpointercapture={(e) => { if (pointers.has(e.pointerId)) cancel(); }} onwheel={wheel} oncontextmenu={(e) => e.preventDefault()}>
    <defs>
      <pattern id="fine-{uid}" width={grid} height={grid} patternUnits="userSpaceOnUse"><path d="M{grid} 0H0V{grid}" fill="none" stroke="var(--grid)" stroke-width={view.shownMmPerPixel * 0.5}/></pattern>
      <pattern id="coarse-{uid}" width={coarseGrid} height={coarseGrid} patternUnits="userSpaceOnUse"><rect
        width={coarseGrid} height={coarseGrid} fill="url(#fine-{uid})"/><path
        d="M{coarseGrid} 0H0V{coarseGrid}" fill="none" stroke="var(--grid-strong)" stroke-width={view.shownMmPerPixel}/></pattern>
    </defs>
    <rect x={shown.x} y={shown.y} width={shown.w} height={shown.h} fill="url(#coarse-{uid})"/>
    <g transform="scale(1 -1)">
      {#if bed}<rect class="bed" x={bed.minX} y={bed.minY} width={bed.maxX - bed.minX} height={bed.maxY - bed.minY} vector-effect="non-scaling-stroke"/>{/if}
      {#if origin}
        <g class="zero" transform="translate({origin[0]} {origin[1]})">
          <circle r={unit / 3}/>
          <path d="M0 0H{unit * 4}M0 0V{unit * 4}" vector-effect="non-scaling-stroke"/>
        </g>
      {/if}
      {@render children?.()}
      {#if head}
        <g class="crosshair">
          <line x1={head[0]} y1={-shown.y - shown.h} x2={head[0]} y2={-shown.y} vector-effect="non-scaling-stroke"/>
          <line x1={shown.x} y1={head[1]} x2={shown.x + shown.w} y2={head[1]} vector-effect="non-scaling-stroke"/>
          <circle cx={head[0]} cy={head[1]} r={unit} vector-effect="non-scaling-stroke"/>
        </g>
      {/if}
      {#if marquee}<rect class="marquee" x={marquee.minX} y={marquee.minY} width={marquee.maxX - marquee.minX} height={marquee.maxY - marquee.minY} vector-effect="non-scaling-stroke"/>{/if}
    </g>
  </svg>
  {#if lift && lifted}
    <svg class="lift" viewBox={view.viewBox} style:transform={liftTransform} preserveAspectRatio="xMidYMid meet" aria-hidden="true">
      <g transform="scale(1 -1)">{@render lifted()}</g>
    </svg>
  {/if}
  {@render overlay?.(project)}
</div>
