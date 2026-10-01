import { StyleTable, TermGrid, applySections } from './grid';
import { decodeFrame, toArrayBufferView, type WireFrame, type WireSection } from './wire';
import {
  Channel,
  errorText,
  onTermExit,
  onTermState,
  termFrameDone,
  termSetVisible,
  termSubscribe,
  termUnsubscribe,
  type SlotInfo,
  type Visible,
} from './client';
import { AGENT_SOURCES, agentStateFromName, agentStateFromWire, termLive } from './state.svelte';
import { META_ALIVE, MODE_ALT_SCREEN, PROGRESS_INDETERMINATE, PROGRESS_NONE } from './wire';

function publishLive(g: TermGrid): void {
  g.liveDirty = false;
  const live = termLive(g.id);
  const m = g.meta;
  if (m) {
    live.modes = m.modes;
    if (m.modes & META_ALIVE) {
      if (live.exited) live.exited = null;
      if (live.error.includes('PTY_NOT_FOUND')) live.error = '';
    }
    const alt = (m.modes & MODE_ALT_SCREEN) !== 0;
    if (live.altScreen !== alt) live.altScreen = alt;
    if (live.displayOffset !== m.displayOffset) live.displayOffset = m.displayOffset;
    if (live.historyLen !== m.historyLen) live.historyLen = m.historyLen;
    if (m.title && live.title !== m.title) live.title = m.title;
    if (m.cwd && live.cwd !== m.cwd) live.cwd = m.cwd;
  }
  const a = g.agent;
  if (a) {
    const state = agentStateFromWire(a.state);
    if (live.agent !== state) live.agent = state;
    const src = AGENT_SOURCES[a.source] ?? 'none';
    if (live.agentSource !== src) live.agentSource = src;
    if (live.agentSince !== a.sinceMs) live.agentSince = a.sinceMs;
    if (live.agentMessage !== a.message) live.agentMessage = a.message;
    const progress =
      a.progress === PROGRESS_NONE ? null : a.progress === PROGRESS_INDETERMINATE ? -1 : Math.min(100, a.progress);
    if (live.progress !== progress) live.progress = progress;
  }
  const has = g.hasSelection();
  if (live.hasSelection !== has) live.hasSelection = has;
}

const CREDIT_FALLBACK_MS = 120;
const BACKLOG_MAX = 32;

type Backlog = { frame: WireFrame; sections: WireSection[] };

class TermHub {
  readonly grids = new Map<string, TermGrid>();
  readonly styles = new StyleTable();
  private slotIds: (string | undefined)[] = [];
  private channel: Channel<unknown> | null = null;
  private subscribed = false;
  private wantKey = '';
  private sentKey = '';
  private want: Visible[] = [];
  private inflight: Promise<void> | null = null;
  private backlog: Backlog[] = [];
  private creditSeq = 0;
  private creditTimer: ReturnType<typeof setTimeout> | null = null;
  private unlisten: Array<() => void> = [];
  private listening = false;
  redraw: () => void = () => {};
  lastError = '';
  version = 0;

  grid(id: string): TermGrid | undefined {
    return this.grids.get(id);
  }

  private gridFor(slot: number): TermGrid | null {
    const id = this.slotIds[slot];
    if (!id) return null;
    let g = this.grids.get(id);
    if (!g) {
      g = new TermGrid(id, this.styles);
      this.grids.set(id, g);
    }
    return g;
  }

  setVisible(list: Visible[]): void {
    const key = list.map((v) => `${v.id}:${v.lod}`).join('|');
    if (key === this.wantKey) return;
    this.wantKey = key;
    this.want = list;
    this.flush();
  }

  private flush(): void {
    if (this.inflight || this.wantKey === this.sentKey) return;
    const list = this.want;
    const key = this.wantKey;
    if (!this.subscribed && list.length === 0) {
      this.sentKey = key;
      return;
    }
    this.listen();
    this.inflight = (async () => {
      try {
        let slots: SlotInfo[];
        if (!this.subscribed) {
          this.channel = new Channel<unknown>();
          this.channel.onmessage = (m) => this.onMessage(m);
          slots = await termSubscribe(this.channel, list);
          this.subscribed = true;
        } else {
          slots = await termSetVisible(list);
        }
        this.sentKey = key;
        this.lastError = '';
        this.setSlots(slots);
      } catch (e) {
        this.lastError = errorText(e);
        this.sentKey = key;
      } finally {
        this.inflight = null;
        this.flush();
      }
    })();
  }

  private setSlots(slots: SlotInfo[]): void {
    for (const s of slots) this.slotIds[s.slot] = s.id;
    if (!this.backlog.length) return;
    const pending = this.backlog;
    this.backlog = [];
    const touched = new Set<TermGrid>();
    for (const b of pending) {
      const rest = applySections(b.frame, b.sections, this.styles, (slot) => this.gridFor(slot), touched);
      if (rest.length) this.pushBacklog(b.frame, rest);
    }
    for (const g of touched) if (g.liveDirty) publishLive(g);
    if (touched.size) {
      this.version++;
      this.redraw();
    }
  }

  private pushBacklog(frame: WireFrame, sections: WireSection[]): void {
    this.backlog.push({ frame, sections });
    if (this.backlog.length > BACKLOG_MAX) this.backlog.shift();
  }

  private onMessage(message: unknown): void {
    let frame: WireFrame;
    try {
      frame = decodeFrame(toArrayBufferView(message));
    } catch (e) {
      this.lastError = errorText(e);
      return;
    }
    const touched = new Set<TermGrid>();
    try {
      const rest = applySections(frame, frame.sections, this.styles, (slot) => this.gridFor(slot), touched);
      if (rest.length) this.pushBacklog(frame, rest);
    } catch (e) {
      this.lastError = errorText(e);
    }
    for (const g of touched) if (g.liveDirty) publishLive(g);
    this.version++;
    if (frame.seq > this.creditSeq) this.creditSeq = frame.seq;
    this.armCredit();
    this.redraw();
  }

  private armCredit(): void {
    if (this.creditTimer) return;
    this.creditTimer = setTimeout(() => {
      this.creditTimer = null;
      this.sendCredit();
    }, CREDIT_FALLBACK_MS);
  }

  private sendCredit(): void {
    if (!this.creditSeq) return;
    const seq = this.creditSeq;
    this.creditSeq = 0;
    termFrameDone(seq).catch((e) => (this.lastError = errorText(e)));
  }

  afterDraw(): void {
    if (!this.creditSeq) return;
    if (this.creditTimer) {
      clearTimeout(this.creditTimer);
      this.creditTimer = null;
    }
    this.sendCredit();
  }

  private listen(): void {
    if (this.listening) return;
    this.listening = true;
    onTermExit((e) => {
      const live = termLive(e.id);
      live.exited = { code: e.code, signal: e.signal };
      this.redraw();
    })
      .then((u) => this.unlisten.push(u))
      .catch(() => {});
    onTermState((e) => {
      const live = termLive(e.id);
      const state = agentStateFromName(e.state);
      if (live.agent !== state) live.agent = state;
      if (live.agentSource !== e.source) live.agentSource = e.source;
      if (live.agentSince !== e.since_ms) live.agentSince = e.since_ms;
      const msg = e.message ?? '';
      if (live.agentMessage !== msg) live.agentMessage = msg;
      const progress = e.progress == null ? null : e.progress;
      if (live.progress !== progress) live.progress = progress;
    })
      .then((u) => this.unlisten.push(u))
      .catch(() => {});
  }

  async stop(): Promise<void> {
    if (this.creditTimer) clearTimeout(this.creditTimer);
    this.creditTimer = null;
    for (const u of this.unlisten.splice(0)) u();
    this.listening = false;
    if (this.channel) this.channel.onmessage = () => {};
    this.channel = null;
    const was = this.subscribed;
    this.subscribed = false;
    this.slotIds = [];
    this.backlog = [];
    this.wantKey = '';
    this.sentKey = '';
    this.styles.clear();
    this.grids.clear();
    if (was) await termUnsubscribe().catch(() => {});
  }

  forget(id: string): void {
    this.grids.delete(id);
  }
}

export const termHub = new TermHub();
