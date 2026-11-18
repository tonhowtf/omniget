export interface FrameHost {
  hidden(): boolean;
  requestAnimationFrame(cb: (t: number) => void): number;
  cancelAnimationFrame(id: number): void;
  setTimeout(cb: () => void, ms: number): unknown;
  clearTimeout(id: unknown): void;
  now(): number;
}

const HIDDEN_FRAME_MS = 16;

const browserHost: FrameHost = {
  hidden: () => typeof document !== 'undefined' && document.visibilityState === 'hidden',
  requestAnimationFrame: (cb) => requestAnimationFrame(cb),
  cancelAnimationFrame: (id) => cancelAnimationFrame(id),
  setTimeout: (cb, ms) => setTimeout(cb, ms),
  clearTimeout: (id) => clearTimeout(id as ReturnType<typeof setTimeout>),
  now: () => performance.now(),
};

export class FrameClock {
  private raf = 0;
  private timer: unknown = null;
  private soon = false;
  private stopped = false;

  constructor(
    private readonly tick: (now: number) => void,
    private readonly host: FrameHost = browserHost,
  ) {}

  request(): void {
    if (this.stopped) return;
    // WHY: a hidden or fully covered window gets no animation frames, so a pending one
    // never fires and changes pushed by MCP agents never reach the DOM.
    if (this.host.hidden()) {
      if (this.raf) {
        this.host.cancelAnimationFrame(this.raf);
        this.raf = 0;
      }
      if (this.timer !== null && this.soon) return;
      this.clearTimer();
      this.soon = true;
      this.timer = this.host.setTimeout(() => {
        this.timer = null;
        this.soon = false;
        this.tick(this.host.now());
      }, HIDDEN_FRAME_MS);
      return;
    }
    if (this.raf) return;
    this.clearTimer();
    this.raf = this.host.requestAnimationFrame((t) => {
      this.raf = 0;
      this.tick(t);
    });
  }

  later(ms: number): void {
    if (this.stopped || this.raf || this.timer !== null) return;
    this.timer = this.host.setTimeout(() => {
      this.timer = null;
      this.request();
    }, ms);
  }

  stop(): void {
    this.stopped = true;
    if (this.raf) this.host.cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.clearTimer();
  }

  private clearTimer(): void {
    if (this.timer !== null) this.host.clearTimeout(this.timer);
    this.timer = null;
    this.soon = false;
  }
}
