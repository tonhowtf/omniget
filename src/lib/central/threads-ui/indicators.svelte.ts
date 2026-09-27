// Sidebar indicators that live outside the engine: which threads have a
// terminal with a running process (from the TerminalPanel layout saved per
// thread + `pty_list` / `pty://activity`), and the PR of each thread's branch
// (looked up only when a thread is opened or its PR tab loads).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ptyList, type PtySessionInfo } from "$lib/central/pty";
import type { PrInfo } from "$lib/central/vcs";

type Layout = { tabs?: { panes?: string[] }[] };

export function terminalIdsOf(threadId: string): string[] {
  try {
    const raw = localStorage.getItem(`omniget.central.terminal.${threadId}`);
    if (!raw) return [];
    const l = JSON.parse(raw) as Layout;
    return (l.tabs ?? []).flatMap((t) => t.panes ?? []);
  } catch {
    return [];
  }
}

class Indicators {
  /** threadId → "running" (foreground process) | "idle" (shell alive). */
  terminals = $state<Record<string, "running" | "idle">>({});
  prs = $state<Record<string, PrInfo | null>>({});
  #sessions = new Map<string, PtySessionInfo>();
  #unlisten: UnlistenFn | null = null;
  #threadIds: string[] = [];

  async start(threadIds: string[]) {
    this.#threadIds = threadIds;
    try {
      for (const s of await ptyList()) this.#sessions.set(s.id, s);
    } catch {
      /* pty host missing */
    }
    this.#recompute();
    if (!this.#unlisten) {
      this.#unlisten = await listen<{ id: string; running: boolean; label: string; pid: number | null }>("pty://activity", (e) => {
        const cur = this.#sessions.get(e.payload.id);
        if (cur) cur.foreground = { running: e.payload.running, label: e.payload.label, pid: e.payload.pid };
        else
          this.#sessions.set(e.payload.id, {
            id: e.payload.id,
            title: "",
            cwd: null,
            command: [],
            pid: null,
            cols: 0,
            rows: 0,
            alive: true,
            exit_code: null,
            exit_signal: null,
            created_at: 0,
            foreground: { running: e.payload.running, label: e.payload.label, pid: e.payload.pid },
          });
        this.#recompute();
      });
    }
  }

  setThreads(threadIds: string[]) {
    this.#threadIds = threadIds;
    this.#recompute();
  }

  #recompute() {
    const out: Record<string, "running" | "idle"> = {};
    for (const id of this.#threadIds) {
      const panes = terminalIdsOf(id).map((p) => this.#sessions.get(p)).filter((s): s is PtySessionInfo => !!s && s.alive);
      if (!panes.length) continue;
      out[id] = panes.some((s) => s.foreground.running) ? "running" : "idle";
    }
    this.terminals = out;
  }

  setPr(threadId: string, pr: PrInfo | null) {
    this.prs = { ...this.prs, [threadId]: pr };
  }

  stop() {
    this.#unlisten?.();
    this.#unlisten = null;
  }
}

export const indicators = new Indicators();
