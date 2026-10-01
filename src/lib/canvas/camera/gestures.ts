// WHY: WKWebView sends pinch as gesturestart/change/end with an absolute scale, never as ctrl+wheel.

interface GestureHandlers {
  pan(dx: number, dy: number): void;
  zoom(factor: number, sx: number, sy: number): void;
  pinch?(phase: 'start' | 'change' | 'end', scale: number, sx: number, sy: number): void;
  activity?(): void;
  passThrough?(e: WheelEvent): boolean;
  wheelZooms?(): boolean;
  zoomModifier?(): 'alt' | 'ctrl' | 'cmd';
  notch?(dir: number): number;
}

function isNotch(e: WheelEvent, dx: number, dy: number): boolean {
  if (e.deltaMode !== 0) return true;
  return dx === 0 && Math.abs(dy) >= 50 && Number.isInteger(e.deltaY);
}

interface WebKitGestureEvent extends UIEvent {
  scale: number;
  clientX: number;
  clientY: number;
}

const LINE_PX = 16;
const PAGE_PX = 800;

function normaliseDelta(e: WheelEvent): { dx: number; dy: number } {
  const k = e.deltaMode === 1 ? LINE_PX : e.deltaMode === 2 ? PAGE_PX : 1;
  return { dx: e.deltaX * k, dy: e.deltaY * k };
}

export const WORKSPACE_STEP_EVENT = 'canvas:workspace-step';
const STEP_PX = 60;
const STEP_COOLDOWN_MS = 450;

export function attachGestures(el: HTMLElement, h: GestureHandlers): () => void {
  let pinching = false;
  let stepAcc = 0;
  let lastStep = -Infinity;
  let lastWheel = -Infinity;

  const local = (cx: number, cy: number) => {
    const r = el.getBoundingClientRect();
    return { x: cx - r.left, y: cy - r.top };
  };

  const onWheel = (e: WheelEvent) => {
    if (pinching) {
      e.preventDefault();
      return;
    }
    if (h.passThrough?.(e)) return;
    e.preventDefault();
    h.activity?.();
    const { dx, dy } = normaliseDelta(e);
    const p = local(e.clientX, e.clientY);
    const notch = isNotch(e, dx, dy) && h.notch;
    if (e.ctrlKey) {
      const factor = notch ? h.notch!(dy < 0 ? 1 : -1) : Math.exp(-dy * 0.01);
      h.zoom(factor, p.x, p.y);
      return;
    }
    const mod = h.zoomModifier?.() ?? 'alt';
    if (e.metaKey && mod !== 'cmd') {
      const d = Math.abs(dy) >= Math.abs(dx) ? dy : dx;
      const now = performance.now();
      if (now - lastWheel > 250) stepAcc = 0;
      lastWheel = now;
      stepAcc += notch ? Math.sign(d) * STEP_PX : d;
      if (Math.abs(stepAcc) >= STEP_PX && now - lastStep > STEP_COOLDOWN_MS) {
        lastStep = now;
        const dir = stepAcc > 0 ? 1 : -1;
        stepAcc = 0;
        window.dispatchEvent(new CustomEvent(WORKSPACE_STEP_EVENT, { detail: dir }));
      }
      return;
    }
    const zoomKey = mod === 'cmd' ? e.metaKey : mod === 'alt' ? e.altKey : false;
    if (zoomKey || h.wheelZooms?.()) {
      const d = dy || dx;
      const factor = notch
        ? h.notch!(d < 0 ? 1 : -1)
        : h.notch
          ? Math.pow(h.notch(d < 0 ? 1 : -1), Math.min(4, Math.abs(d) / 40))
          : Math.exp(-d * 0.0025);
      h.zoom(factor, p.x, p.y);
      return;
    }
    if (e.shiftKey && dx === 0) {
      h.pan(-dy, 0);
      return;
    }
    h.pan(-dx, -dy);
  };

  let anchor = { x: 0, y: 0 };
  const onGestureStart = (e: Event) => {
    e.preventDefault();
    const g = e as WebKitGestureEvent;
    pinching = true;
    anchor = local(g.clientX, g.clientY);
    h.activity?.();
    if (h.pinch) h.pinch('start', 1, anchor.x, anchor.y);
  };
  const onGestureChange = (e: Event) => {
    e.preventDefault();
    const g = e as WebKitGestureEvent;
    h.activity?.();
    if (h.pinch) h.pinch('change', g.scale, anchor.x, anchor.y);
  };
  const onGestureEnd = (e: Event) => {
    e.preventDefault();
    const g = e as WebKitGestureEvent;
    pinching = false;
    h.activity?.();
    if (h.pinch) h.pinch('end', g.scale, anchor.x, anchor.y);
  };

  el.addEventListener('wheel', onWheel, { passive: false });
  el.addEventListener('gesturestart', onGestureStart, { passive: false } as AddEventListenerOptions);
  el.addEventListener('gesturechange', onGestureChange, { passive: false } as AddEventListenerOptions);
  el.addEventListener('gestureend', onGestureEnd, { passive: false } as AddEventListenerOptions);
  return () => {
    el.removeEventListener('wheel', onWheel);
    el.removeEventListener('gesturestart', onGestureStart);
    el.removeEventListener('gesturechange', onGestureChange);
    el.removeEventListener('gestureend', onGestureEnd);
  };
}
