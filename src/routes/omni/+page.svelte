<script lang="ts">
  /**
   * Omni's own window (Maestri: the Ombro window, ⌘⇧O): floats above every
   * app, shows what each agent just did with three next steps, and answers
   * questions about a workspace. Built in Rust by
   * `crate::canvas::companion::toggle_window`; ⌘⇧O or Esc hides it.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { WebviewWindow, getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { t } from "$lib/i18n";
  import CompanionPanel from "$components/canvas/companion/CompanionPanel.svelte";
  import "$lib/canvas/companion/live-language";

  interface WorkspaceRow {
    id: string;
    name: string;
    activeFloorId?: string;
    floors?: Array<{ id: string; name?: string }>;
  }

  const LAST_KEY = "omni.window.workspace";

  let workspaces = $state<WorkspaceRow[]>([]);
  let workspaceId = $state("");
  let terminals = $state<Array<{ id: string; title: string }>>([]);

  function remember(id: string) {
    try {
      localStorage.setItem(LAST_KEY, id);
    } catch {
      /* private storage: fine */
    }
  }

  function recall(): string {
    try {
      return localStorage.getItem(LAST_KEY) ?? "";
    } catch {
      return "";
    }
  }

  async function loadTerminals(id: string) {
    const ws = workspaces.find((w) => w.id === id);
    const floors = ws?.floors?.map((f) => f.id) ?? [ws?.activeFloorId ?? "ground"];
    const out: Array<{ id: string; title: string }> = [];
    for (const floorId of floors) {
      try {
        const snap = await invoke<{ nodes?: Array<{ id: string; kind: string; title?: string }> }>("canvas_load", { workspaceId: id, floorId });
        for (const n of snap.nodes ?? []) {
          if (n.kind === "terminal" || n.kind === "agent") out.push({ id: n.id, title: n.title ?? n.id.slice(0, 6) });
        }
      } catch {
        /* a floor that does not load is skipped */
      }
    }
    terminals = out;
  }

  async function refresh() {
    try {
      workspaces = await invoke<WorkspaceRow[]>("canvas_workspaces");
    } catch {
      workspaces = [];
    }
    const last = recall();
    workspaceId = workspaces.some((w) => w.id === last) ? last : (workspaces[0]?.id ?? "");
    if (workspaceId) await loadTerminals(workspaceId);
  }

  async function pick(id: string) {
    workspaceId = id;
    remember(id);
    await loadTerminals(id);
  }

  async function focusMain() {
    const main = await WebviewWindow.getByLabel("main");
    await main?.show();
    await main?.setFocus();
  }

  function hide() {
    void getCurrentWebviewWindow().hide();
  }

  function onkey(e: KeyboardEvent) {
    const combo = (e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "o";
    if (e.key === "Escape" || combo) {
      e.preventDefault();
      hide();
    }
  }

  onMount(() => {
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });
</script>

<svelte:window onkeydown={onkey} />

<main class="omni">
  <header data-tauri-drag-region>
    <strong data-tauri-drag-region>Omni</strong>
    {#if workspaces.length > 1}
      <select value={workspaceId} onchange={(e) => void pick(e.currentTarget.value)} aria-label={$t("canvas.companion.window.workspace", { default: "Workspace" }) as string}>
        {#each workspaces as w (w.id)}<option value={w.id}>{w.name}</option>{/each}
      </select>
    {/if}
    <button type="button" class="ghost" title={$t("canvas.companion.window.hide", { default: "Hide (⌘⇧O)" }) as string} onclick={hide}>✕</button>
  </header>
  <div class="body">
    <CompanionPanel fill {workspaceId} {terminals} onFocusNode={() => void focusMain()} />
  </div>
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    background: var(--surface, #1c1c1e);
  }
  .omni {
    display: flex;
    flex-direction: column;
    height: 100vh;
    color: var(--text);
    background: var(--surface, #1c1c1e);
    font-size: 13px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  header strong {
    flex: 1;
    font-size: 14px;
  }
  select {
    max-width: 160px;
  }
  .ghost {
    all: unset;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 6px;
    color: var(--text-muted);
  }
  .ghost:hover {
    background: var(--hover, rgba(127, 127, 127, 0.15));
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
</style>
