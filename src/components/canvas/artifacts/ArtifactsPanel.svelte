<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import {
    artifactList,
    artifactOpen,
    artifactsPanel,
    closeArtifacts,
    type Artifact,
    type ArtifactKind,
  } from "$lib/canvas/artifacts/client.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  const engine = useEngine();

  let items = $state<Artifact[]>([]);
  let loading = $state(false);
  let error = $state("");
  let filter = $state("");
  let placing = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  const KIND_LABEL: Record<ArtifactKind, string> = { html: "HTML", md: "Markdown", svg: "SVG", link: "Link" };

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return items;
    return items.filter((a) => a.name.toLowerCase().includes(q) || a.kind.includes(q) || (a.url ?? "").toLowerCase().includes(q));
  });

  async function load() {
    const ws = engine.store.workspaceId;
    if (!ws) return;
    loading = true;
    error = "";
    try {
      items = await artifactList(ws);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (artifactsPanel.open) {
      void load();
      queueMicrotask(() => inputEl?.focus());
    }
  });

  async function place(a: Artifact) {
    placing = a.id;
    try {
      const at = artifactsPanel.at ?? (() => {
        const c = engine.camera.center();
        return { x: c.x - 160, y: c.y - 120 };
      })();
      const r = await artifactOpen(engine.store.workspaceId, engine.store.floorId, a.id, at);
      for (let i = 0; i < 20 && !engine.store.node(r.nodeId); i++) {
        await new Promise((res) => setTimeout(res, 50));
      }
      if (engine.store.node(r.nodeId)) {
        engine.selection.set([r.nodeId]);
        engine.reveal(r.nodeId);
        engine.invalidate();
      }
      showToast("success", $t("canvas.artifacts.placed", { default: "Placed on the canvas" }), 2500);
      closeArtifacts();
    } catch (e) {
      error = String(e);
    } finally {
      placing = "";
    }
  }

  function when(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      closeArtifacts();
    } else if (e.key === "Enter" && shown.length) {
      e.preventDefault();
      void place(shown[0]);
    }
  }
</script>

{#if artifactsPanel.open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="panel" data-canvas-chrome role="dialog" tabindex="-1" aria-label={$t("canvas.artifacts.title", { default: "Artifacts" })} onkeydown={onkey}>
    <header>
      <span class="title">{$t("canvas.artifacts.title", { default: "Artifacts" })}</span>
      <span class="count">{items.length}</span>
      <button type="button" class="close" aria-label={$t("canvas.artifacts.close", { default: "Close" })} onclick={closeArtifacts}>✕</button>
    </header>
    <input
      bind:this={inputEl}
      class="filter"
      type="text"
      placeholder={$t("canvas.artifacts.search", { default: "Filter" })}
      value={filter}
      oninput={(e) => (filter = (e.currentTarget as HTMLInputElement).value)}
    />
    <div class="list">
      {#if loading && !items.length}
        <div class="empty">…</div>
      {:else if !shown.length}
        <div class="empty">{$t("canvas.artifacts.empty", { default: "No artifacts yet. Agents save them with canvas_artifact_save." })}</div>
      {:else}
        {#each shown as a (a.id)}
          <button type="button" class="row" disabled={!!placing} onclick={() => place(a)} title={a.url ?? a.path ?? a.name}>
            <span class="kind kind-{a.kind}">{$t(`canvas.artifacts.kind.${a.kind}`, { default: KIND_LABEL[a.kind] ?? a.kind })}</span>
            <span class="meta">
              <span class="name">{a.name}</span>
              <span class="sub">{when(a.createdAt)}{a.url ? ` · ${a.url}` : ""}</span>
            </span>
            <span class="place">{placing === a.id ? "…" : $t("canvas.artifacts.place", { default: "Place" })}</span>
          </button>
        {/each}
      {/if}
      {#if error}<div class="error">{error}</div>{/if}
    </div>
  </div>
{/if}

<style>
  .panel {
    position: absolute;
    top: 56px;
    right: 16px;
    width: 340px;
    max-height: min(520px, calc(100% - 96px));
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: 12px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.32);
    z-index: 25;
    color: var(--text);
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 2px;
  }
  .title {
    font-weight: 700;
    font-size: 13px;
  }
  .count {
    font-size: 11px;
    color: var(--text-muted);
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .close {
    all: unset;
    margin-left: auto;
    cursor: pointer;
    color: var(--text-muted);
    padding: 2px 6px;
    border-radius: 6px;
  }
  .close:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .filter {
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    background: color-mix(in srgb, var(--text) 5%, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 9px;
    outline: none;
  }
  .filter:focus {
    border-color: var(--accent);
  }
  .list {
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    all: unset;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px;
    border-radius: 8px;
    cursor: pointer;
  }
  .row:hover,
  .row:focus-visible {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .row:hover .place {
    opacity: 1;
  }
  .kind {
    flex-shrink: 0;
    width: 64px;
    text-align: center;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    padding: 3px 0;
    border-radius: 6px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text-muted);
  }
  .kind-html {
    background: color-mix(in srgb, #f97316 16%, transparent);
    color: #ea580c;
  }
  .kind-md {
    background: color-mix(in srgb, #3b82f6 16%, transparent);
    color: #2563eb;
  }
  .kind-svg {
    background: color-mix(in srgb, #a855f7 16%, transparent);
    color: #9333ea;
  }
  .kind-link {
    background: color-mix(in srgb, #22c55e 16%, transparent);
    color: #16a34a;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .place {
    flex-shrink: 0;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent);
    opacity: 0.6;
  }
  .empty {
    padding: 16px 8px;
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }
  .error {
    color: #dc2626;
    font-size: 11.5px;
    padding: 4px 8px;
  }
</style>
