// Visit session as the UI sees it: the state (host, visitor or none), the
// latest scene when visiting, the room chat and the visitors' reactions.
// One listener set, reference counted.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  EVENT_VISIT_CHAT,
  EVENT_VISIT_CLOSED,
  EVENT_VISIT_REACTION,
  EVENT_VISIT_SCENE,
  EVENT_VISIT_STATE,
  visitStatus,
  type ChatLine,
  type Reaction,
  type Scene,
  type VisitState,
} from './api';
import { isScene } from './scene';

const CHAT_CAP = 100;

class VisitSession {
  state = $state.raw<VisitState>({ role: 'none', shared: [], visitors: [] });
  scene = $state.raw<Scene | null>(null);
  chat = $state.raw<ChatLine[]>([]);
  /** Reactions received as host (visitors waving or pointing). */
  reactions = $state.raw<Reaction[]>([]);
  closedReason = $state<string | null>(null);
  private unlisten: UnlistenFn[] = [];
  private refs = 0;

  async start(): Promise<void> {
    this.refs++;
    if (this.refs > 1) return;
    try {
      this.unlisten.push(
        await listen<VisitState>(EVENT_VISIT_STATE, (e) => {
          if (e.payload) this.takeState(e.payload);
        }),
        await listen(EVENT_VISIT_SCENE, (e) => {
          if (isScene(e.payload)) this.scene = e.payload;
        }),
        await listen<ChatLine>(EVENT_VISIT_CHAT, (e) => {
          if (e.payload && typeof e.payload.text === 'string') {
            this.chat = [...this.chat, e.payload].slice(-CHAT_CAP);
          }
        }),
        await listen<Reaction>(EVENT_VISIT_REACTION, (e) => {
          if (!e.payload) return;
          const now = Date.now();
          this.reactions = [...this.reactions.filter((r) => now - r.at < 6000), e.payload].slice(-16);
        }),
        await listen<{ reason?: string }>(EVENT_VISIT_CLOSED, (e) => {
          this.closedReason = e.payload?.reason ?? 'left';
          this.scene = null;
        }),
      );
      this.takeState(await visitStatus());
    } catch {
      // Outside Tauri.
    }
  }

  /** A state (visit_join from MCP or the CLI included) carries the last scene: the visitor sees it right away. */
  takeState(st: VisitState): void {
    this.state = st;
    const scene = (st as VisitState & { scene?: unknown }).scene;
    if (st.role === 'visitor' && !this.scene && isScene(scene)) this.scene = scene;
    if (st.role === 'none') this.scene = null;
  }

  stop(): void {
    this.refs = Math.max(0, this.refs - 1);
    if (this.refs > 0) return;
    for (const u of this.unlisten) u();
    this.unlisten = [];
  }

  reset(): void {
    this.scene = null;
    this.chat = [];
    this.reactions = [];
    this.closedReason = null;
  }
}

export const visitSession = new VisitSession();
