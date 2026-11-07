export type SaveStatus =
  | { kind: "clean" }
  | { kind: "dirty" }
  | { kind: "saving" }
  | { kind: "saved"; at: number }
  | { kind: "error"; reason: string; retryInMs: number | null; failures: number };

export type SaveState = {
  status: SaveStatus;
  revision: number;
  savedRevision: number;
  failures: number;
};

export type SaveEvent =
  | { type: "reset" }
  | { type: "change" }
  | { type: "start"; revision: number }
  | { type: "ok"; revision: number; at: number }
  | { type: "fail"; revision: number; reason: string; retryInMs: number | null };

export const RETRY_DELAYS_MS = [2_000, 5_000, 15_000, 30_000];

export function initialSaveState(): SaveState {
  return { status: { kind: "clean" }, revision: 0, savedRevision: 0, failures: 0 };
}

export function hasUnsaved(state: SaveState): boolean {
  return state.revision > state.savedRevision;
}

export function reduceSave(state: SaveState, event: SaveEvent): SaveState {
  switch (event.type) {
    case "reset":
      return initialSaveState();
    case "change": {
      const revision = state.revision + 1;
      const status: SaveStatus = state.status.kind === "error" || state.status.kind === "saving" ? state.status : { kind: "dirty" };
      return { ...state, revision, status };
    }
    case "start":
      return { ...state, status: { kind: "saving" } };
    case "ok": {
      const savedRevision = Math.max(state.savedRevision, event.revision);
      const status: SaveStatus = state.revision > savedRevision ? { kind: "dirty" } : { kind: "saved", at: event.at };
      return { ...state, savedRevision, failures: 0, status };
    }
    case "fail": {
      const failures = state.failures + 1;
      return { ...state, failures, status: { kind: "error", reason: event.reason, retryInMs: event.retryInMs, failures } };
    }
  }
}

export function retryDelay(failures: number, delays: readonly number[] = RETRY_DELAYS_MS): number {
  return delays[Math.min(Math.max(failures, 1), delays.length) - 1];
}

type Options = {
  delayMs: number;
  save: (revision: number) => Promise<void>;
  onState: (state: SaveState) => void;
  describe?: (error: unknown) => string;
  now?: () => number;
  retryDelays?: readonly number[];
};

export class Autosaver {
  private state: SaveState = initialSaveState();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight: Promise<void> | null = null;
  private disposed = false;

  constructor(private readonly opts: Options) {}

  get current(): SaveState {
    return this.state;
  }

  private dispatch(event: SaveEvent) {
    this.state = reduceSave(this.state, event);
    if (!this.disposed) this.opts.onState(this.state);
  }

  private schedule(ms: number) {
    if (this.disposed) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.run();
    }, ms);
  }

  reset() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.dispatch({ type: "reset" });
  }

  change() {
    this.dispatch({ type: "change" });
    if (!this.inFlight && this.state.status.kind !== "error") this.schedule(this.opts.delayMs);
  }

  private async run(): Promise<void> {
    if (this.inFlight) return this.inFlight;
    if (!hasUnsaved(this.state)) return;
    const revision = this.state.revision;
    this.dispatch({ type: "start", revision });
    const attempt = (async () => {
      try {
        await this.opts.save(revision);
        this.dispatch({ type: "ok", revision, at: (this.opts.now ?? Date.now)() });
      } catch (error) {
        const reason = this.opts.describe ? this.opts.describe(error) : String(error);
        const wait = retryDelay(this.state.failures + 1, this.opts.retryDelays);
        this.dispatch({ type: "fail", revision, reason, retryInMs: wait });
        this.schedule(wait);
        return;
      }
      if (hasUnsaved(this.state)) this.schedule(this.opts.delayMs);
    })();
    this.inFlight = attempt;
    try {
      await attempt;
    } finally {
      this.inFlight = null;
    }
  }

  async flush(): Promise<boolean> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.inFlight) await this.inFlight;
    if (hasUnsaved(this.state)) await this.run();
    if (this.timer && !hasUnsaved(this.state)) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    return !hasUnsaved(this.state);
  }

  async idle(): Promise<void> {
    while (this.inFlight) await this.inFlight;
  }

  retryNow(): Promise<boolean> {
    return this.flush();
  }

  dispose() {
    this.disposed = true;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}
