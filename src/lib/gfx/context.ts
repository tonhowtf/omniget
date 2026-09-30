// WebGL context lifecycle as an explicit state, not an accident.
// The webview reclaims WebGL after inactivity in Tauri (pixijs#11731), so the
// renderer drops the context on purpose when the route hides and rebuilds it on wake.

export type ContextState = 'new' | 'live' | 'sleeping' | 'lost' | 'restoring' | 'destroyed';

export type ContextEvent = 'init' | 'sleep' | 'wake' | 'lost' | 'restored' | 'destroy';

/** Pure transition table; every backend and the tests share it. */
export function nextState(state: ContextState, event: ContextEvent): ContextState {
  if (event === 'destroy') return 'destroyed';
  if (state === 'destroyed') return 'destroyed';
  switch (event) {
    case 'init':
      return 'live';
    case 'sleep':
      return state === 'live' || state === 'lost' || state === 'restoring' ? 'sleeping' : state;
    case 'wake':
      return state === 'sleeping' || state === 'lost' ? 'restoring' : state;
    case 'lost':
      return state === 'sleeping' ? 'sleeping' : 'lost';
    case 'restored':
      return state === 'sleeping' ? 'sleeping' : 'live';
    default:
      return state;
  }
}

/**
 * WEBGL_lose_context.restoreContext() is asynchronous: the context object stays
 * lost for a few frames, and a canvas never hands out a second context, so
 * rebuilding immediately fails with a null createShader. Polls until the same
 * context reports itself alive again.
 */
export async function waitForRestore(
  gl: WebGLRenderingContext | WebGL2RenderingContext | null,
  timeoutMs = 2000,
  stepMs = 16,
): Promise<boolean> {
  if (!gl) return false;
  const deadline = Date.now() + timeoutMs;
  while (gl.isContextLost()) {
    if (Date.now() >= deadline) return false;
    await new Promise((r) => setTimeout(r, stepMs));
  }
  return true;
}

export interface ContextCallbacks {
  onLost?: () => void;
  onRestored?: () => void;
}

export class ContextTracker {
  state: ContextState = 'new';
  /** True when the loss came from our own sleep() and must not be reported. */
  private intentional = false;
  private canvas: HTMLCanvasElement | null = null;
  private cbs: ContextCallbacks = {};
  private readonly lostHandler = (e: Event) => {
    e.preventDefault(); // without this the browser never restores
    this.state = nextState(this.state, 'lost');
    if (!this.intentional) this.cbs.onLost?.();
  };
  private readonly restoredHandler = () => {
    this.state = nextState(this.state, 'restored');
    if (!this.intentional) this.cbs.onRestored?.();
  };

  attach(canvas: HTMLCanvasElement, cbs: ContextCallbacks = {}): void {
    this.detach();
    this.canvas = canvas;
    this.cbs = cbs;
    canvas.addEventListener('webglcontextlost', this.lostHandler as EventListener, false);
    canvas.addEventListener('webglcontextrestored', this.restoredHandler as EventListener, false);
    this.state = nextState(this.state, 'init');
  }

  detach(): void {
    if (!this.canvas) return;
    this.canvas.removeEventListener('webglcontextlost', this.lostHandler as EventListener);
    this.canvas.removeEventListener('webglcontextrestored', this.restoredHandler as EventListener);
    this.canvas = null;
  }

  apply(event: ContextEvent): ContextState {
    this.state = nextState(this.state, event);
    return this.state;
  }

  /** Releases the GPU context on purpose (sleep). */
  loseDeliberately(gl: WebGLRenderingContext | WebGL2RenderingContext | null): void {
    this.intentional = true;
    this.state = nextState(this.state, 'sleep');
    const ext = gl?.getExtension('WEBGL_lose_context') as
      | { loseContext(): void; restoreContext(): void }
      | null
      | undefined;
    try {
      ext?.loseContext();
    } catch {
      // Some drivers throw when the context is already gone; the state already says so.
    }
  }

  /** Asks the driver to give the context back; a full rebuild follows anyway. */
  restoreDeliberately(gl: WebGLRenderingContext | WebGL2RenderingContext | null): void {
    this.state = nextState(this.state, 'wake');
    const ext = gl?.getExtension('WEBGL_lose_context') as
      | { loseContext(): void; restoreContext(): void }
      | null
      | undefined;
    try {
      ext?.restoreContext();
    } catch {
      // Ignored: wake() rebuilds from the ImageBitmaps kept in memory.
    }
    this.intentional = false;
    this.state = nextState(this.state, 'restored');
  }

  get isLive(): boolean {
    return this.state === 'live';
  }
}
