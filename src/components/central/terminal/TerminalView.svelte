<script lang="ts">
  // One xterm bound to one PTY session. The session outlives this component:
  // on mount it attaches (sanitized history + cursor) and follows the live
  // stream from there, so remounting never duplicates or loses output.
  import { onMount } from "svelte";
  import type { Terminal as XTerm } from "@xterm/xterm";
  import type { FitAddon as XFitAddon } from "@xterm/addon-fit";
  import "@xterm/xterm/css/xterm.css";
  import { t } from "$lib/i18n";
  import {
    decodeBase64,
    errorCode,
    ptyAck,
    ptyAttach,
    ptyDetach,
    ptyOpen,
    ptyResize,
    ptyWrite,
    subscribePty,
    type PtyDataEvent,
    type PtyForeground,
    type PtySessionInfo,
    type TerminalSelection,
  } from "$lib/central/pty";
  import { terminalFont, terminalTheme } from "./theme";

  let {
    id,
    cwd = null,
    command = [],
    env = undefined,
    initialInput = undefined,
    fontSize = 12,
    focused = false,
    onFocus,
    onSelectionChange,
    onForeground,
    onExit,
    onInfo,
    api = $bindable(),
  }: {
    id: string;
    cwd?: string | null;
    command?: string[];
    env?: Record<string, string>;
    initialInput?: string;
    fontSize?: number;
    focused?: boolean;
    onFocus?: () => void;
    onSelectionChange?: (hasSelection: boolean) => void;
    onForeground?: (fg: PtyForeground) => void;
    onExit?: (code: number | null, signal: string | null) => void;
    onInfo?: (info: PtySessionInfo) => void;
    /** Imperative handle for the panel (selection, focus, clear). */
    api?: {
      selection: () => TerminalSelection | null;
      focus: () => void;
      clearScreen: () => void;
    };
  } = $props();

  let host: HTMLDivElement;
  let term: XTerm | null = null;
  let fit: XFitAddon | null = null;
  let status = $state<"connecting" | "live" | "ended" | "error">("connecting");
  let errorText = $state("");

  // Stream cursor.
  let generation = 0;
  let expected = 0;
  let attached = false;
  let early: PtyDataEvent[] = [];
  let reattaching = false;
  let disposed = false;

  api = {
    selection() {
      if (!term) return null;
      const text = term.getSelection();
      const pos = term.getSelectionPosition();
      if (!text || !pos) return null;
      return { text, lines: [pos.start.y + 1, pos.end.y + 1], terminal_id: id };
    },
    focus() {
      term?.focus();
    },
    clearScreen() {
      term?.clear();
    },
  };

  $effect(() => {
    if (focused) term?.focus();
  });

  function onData(ev: PtyDataEvent) {
    if (!attached) {
      early.push(ev);
      return;
    }
    if (ev.generation !== generation) {
      void reattach();
      return;
    }
    const bytes = decodeBase64(ev.data);
    const end = ev.start + bytes.length;
    if (end <= expected) {
      void ptyAck(id, ev.seq).catch(() => {});
      return;
    }
    if (ev.start > expected) {
      // Bytes were dropped (we were detached for a stall): start over.
      void reattach();
      return;
    }
    const slice = ev.start < expected ? bytes.subarray(expected - ev.start) : bytes;
    expected = end;
    term?.write(slice, () => {
      void ptyAck(id, ev.seq).catch(() => {});
    });
  }

  async function attach(): Promise<boolean> {
    attached = false;
    early = [];
    let res;
    try {
      res = await ptyAttach(id);
    } catch (e) {
      if (errorCode(e) !== "PTY_NOT_FOUND") throw e;
      res = null;
    }
    if (disposed) return false;
    if (res && !res.alive) {
      // A transcript from a previous run of the app: show it, then start fresh.
      term?.reset();
      term?.write(decodeBase64(res.history));
      term?.write(`\r\n\x1b[2m${$t("llm.central.terminal.previous_session")}\x1b[0m\r\n`);
      res = null;
    }
    if (!res) {
      const dims = fit?.proposeDimensions();
      const info = await ptyOpen({
        id,
        cwd,
        command,
        env,
        cols: dims?.cols ?? term?.cols,
        rows: dims?.rows ?? term?.rows,
        initial_input: initialInput,
      });
      onInfo?.(info);
      res = await ptyAttach(id);
      if (disposed) return false;
    } else {
      term?.reset();
    }
    if (res.info) {
      onInfo?.(res.info);
      onForeground?.(res.info.foreground);
      if (!res.info.alive) {
        status = "ended";
      }
    }
    generation = res.generation;
    expected = res.offset;
    const hist = decodeBase64(res.history);
    if (hist.length) term?.write(hist);
    attached = true;
    const pending = early;
    early = [];
    for (const ev of pending) onData(ev);
    if (status !== "ended") status = "live";
    syncSize();
    return true;
  }

  async function reattach() {
    if (reattaching || disposed) return;
    reattaching = true;
    try {
      await ptyDetach(id).catch(() => {});
      await attach();
    } catch (e) {
      status = "error";
      errorText = String(e);
    } finally {
      reattaching = false;
    }
  }

  let lastSize = "";
  function syncSize() {
    if (!term || !fit || disposed) return;
    try {
      fit.fit();
    } catch {
      return;
    }
    const key = `${term.cols}x${term.rows}`;
    if (key === lastSize || !attached) return;
    lastSize = key;
    void ptyResize(id, term.cols, term.rows).catch(() => {});
  }

  function applyTheme() {
    if (!term) return;
    term.options.theme = terminalTheme();
    term.options.fontFamily = terminalFont();
  }

  onMount(() => {
    let unsub: (() => void) | null = null;
    let ro: ResizeObserver | null = null;
    let mo: MutationObserver | null = null;
    let raf = 0;

    (async () => {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
      ]);
      if (disposed) return;
      term = new Terminal({
        fontFamily: terminalFont(),
        fontSize,
        lineHeight: 1.15,
        cursorBlink: true,
        scrollback: 10000,
        allowProposedApi: false,
        theme: terminalTheme(),
        macOptionIsMeta: true,
        rightClickSelectsWord: false,
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      term.open(host);
      // WebGL when available; the DOM renderer otherwise or after a context loss.
      try {
        const { WebglAddon } = await import("@xterm/addon-webgl");
        if (!disposed && term) {
          const gl = new WebglAddon();
          gl.onContextLoss(() => gl.dispose());
          term.loadAddon(gl);
        }
      } catch {
        // DOM renderer stays.
      }
      term.onData((d) => {
        void ptyWrite(id, d).catch(() => {});
      });
      term.onBinary((d) => {
        void ptyWrite(id, d).catch(() => {});
      });
      term.onSelectionChange(() => onSelectionChange?.(!!term?.hasSelection()));
      term.textarea?.addEventListener("focus", () => onFocus?.());
      term.onResize(() => {
        if (!attached) return;
        const key = `${term!.cols}x${term!.rows}`;
        if (key === lastSize) return;
        lastSize = key;
        void ptyResize(id, term!.cols, term!.rows).catch(() => {});
      });

      ro = new ResizeObserver(() => {
        cancelAnimationFrame(raf);
        raf = requestAnimationFrame(syncSize);
      });
      ro.observe(host);
      mo = new MutationObserver(applyTheme);
      mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "class", "style"] });

      try {
        fit.fit();
      } catch {
        // Hidden host: sized on the first ResizeObserver tick.
      }

      unsub = await subscribePty(id, {
        data: onData,
        exit: (e) => {
          status = "ended";
          term?.write(
            `\r\n\x1b[2m${$t("llm.central.terminal.exited", { code: e.code ?? e.signal ?? "?" })}\x1b[0m\r\n`,
          );
          onExit?.(e.code, e.signal);
        },
        activity: (e) => onForeground?.({ running: e.running, label: e.label, pid: e.pid }),
        stall: () => {
          void reattach();
        },
      });
      if (disposed) {
        unsub();
        return;
      }
      try {
        await attach();
        if (focused) term?.focus();
      } catch (e) {
        status = "error";
        errorText = String(e);
      }
    })();

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      ro?.disconnect();
      mo?.disconnect();
      unsub?.();
      if (attached) void ptyDetach(id).catch(() => {});
      attached = false;
      term?.dispose();
      term = null;
      fit = null;
    };
  });
</script>

<div class="term-view" class:focused data-status={status}>
  <div class="xterm-host" bind:this={host}></div>
  {#if status === "connecting"}
    <div class="overlay">{$t("llm.central.terminal.connecting")}</div>
  {:else if status === "error"}
    <div class="overlay error">{errorText}</div>
  {/if}
</div>

<style>
  .term-view {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    min-width: 0;
    background: var(--pane-bg);
    overflow: hidden;
  }
  .xterm-host {
    position: absolute;
    inset: var(--space-1) 0 0 var(--space-2);
  }
  .xterm-host :global(.xterm) {
    height: 100%;
  }
  .xterm-host :global(.xterm-viewport) {
    background-color: transparent !important;
  }
  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: var(--text-sm);
    pointer-events: none;
  }
  .overlay.error {
    color: var(--danger);
    padding: var(--space-4);
    text-align: center;
    pointer-events: auto;
    user-select: text;
  }
</style>
