<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { switchTo } from "$lib/canvas/palette/navigate";
  import { actorColor, aiActivity, baseOrigin, initials, isAi, VERB_DEFAULTS, type ActivityEntry } from "$lib/canvas/activity/store.svelte";
  import McpServersPanel from "./McpServersPanel.svelte";

  const engine = useEngine();

  const shown = $derived.by(() => {
    const list = aiActivity.aiOnly ? aiActivity.entries.filter(isAi) : aiActivity.entries;
    return list.slice(-200).reverse();
  });
  const actors = $derived(aiActivity.actors());

  function who(e: ActivityEntry): string {
    if (e.actorKind === "person") return $t("canvas.activity.you", { default: "You" });
    return e.actor;
  }

  function verb(e: ActivityEntry): string {
    return $t(`canvas.activity.verb.${e.verb}`, { default: VERB_DEFAULTS[e.verb] ?? e.verb });
  }

  function target(e: ActivityEntry): string {
    if (e.target) return `“${e.target}”`;
    if (e.verb === "corded" || e.verb === "uncorded") return $t("canvas.activity.cords", { default: "{{count}} cord(s)", count: e.count });
    if (e.verb === "drew" || e.verb === "erased") return $t("canvas.activity.drawings", { default: "{{count}} drawing(s)", count: e.count });
    if (e.count > 1) return $t("canvas.activity.nodes", { default: "{{count}} nodes", count: e.count });
    return $t(`canvas.activity.kind.${e.targetKind ?? "node"}`, { default: e.targetKind ?? "a node" });
  }

  function clock(at: number): string {
    return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  async function go(e: ActivityEntry) {
    if (e.floorId && e.floorId !== engine.store.floorId) await switchTo(engine, e.workspaceId, e.floorId);
    const nodes = e.nodeIds.map((id) => engine.store.node(id)).filter((n) => !!n);
    if (!nodes.length) {
      engine.say("canvas.activity.gone");
      return;
    }
    const x0 = Math.min(...nodes.map((n) => n!.x));
    const y0 = Math.min(...nodes.map((n) => n!.y));
    const x1 = Math.max(...nodes.map((n) => n!.x + n!.w));
    const y1 = Math.max(...nodes.map((n) => n!.y + n!.h));
    engine.selection.set(nodes.map((n) => n!.id));
    engine.flyToRect({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, 1);
  }
</script>

<section class="panel" aria-label={$t("canvas.activity.title", { default: "Activity" })}>
  <header>
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={aiActivity.tab === "activity"} class:on={aiActivity.tab === "activity"} onclick={() => (aiActivity.tab = "activity")}>
        {$t("canvas.activity.title", { default: "Activity" })}
      </button>
      <button role="tab" aria-selected={aiActivity.tab === "mcp"} class:on={aiActivity.tab === "mcp"} onclick={() => (aiActivity.tab = "mcp")}>
        {$t("canvas.mcpServers.title", { default: "MCP servers" })}
      </button>
    </div>
    <button class="icon" aria-label={$t("canvas.activity.close", { default: "Close" })} onclick={() => (aiActivity.open = false)}>×</button>
  </header>

  {#if aiActivity.tab === "activity"}
    <div class="controls">
      <label class="switch">
        <input type="checkbox" bind:checked={aiActivity.follow} />
        <span>{$t("canvas.activity.follow", { default: "Follow AI" })}</span>
      </label>
      <label class="switch">
        <input type="checkbox" bind:checked={aiActivity.presence} />
        <span>{$t("canvas.activity.presence", { default: "Show AI cursors" })}</span>
      </label>
      {#if aiActivity.follow && actors.length > 1}
        <select bind:value={aiActivity.followOrigin} aria-label={$t("canvas.activity.followWho", { default: "Who to follow" })}>
          <option value={null}>{$t("canvas.activity.anyAi", { default: "Any AI" })}</option>
          {#each actors as a (a.origin)}
            <option value={a.origin}>{a.name}</option>
          {/each}
        </select>
      {/if}
      <label class="switch">
        <input type="checkbox" bind:checked={aiActivity.aiOnly} />
        <span>{$t("canvas.activity.aiOnly", { default: "Only AIs" })}</span>
      </label>
    </div>
    {#if shown.length === 0}
      <p class="empty">{$t("canvas.activity.empty", { default: "Nothing yet. When an AI or an agent changes the canvas, each step shows up here." })}</p>
    {:else}
      <ol class="list">
        {#each shown as e (e.id)}
          <li>
            <button class="row" title={e.summary} onclick={() => void go(e)}>
              <span class="chip" style:--c={e.actorKind === "person" ? "var(--text-muted)" : actorColor(e.origin)} aria-hidden="true">{initials(who(e))}</span>
              <span class="text">
                <span class="line">
                  <strong>{who(e)}</strong>
                  {#if e.undo}<em>{$t("canvas.activity.undid", { default: "undid" })}</em>{/if}
                  <span>{verb(e)}</span>
                  <span class="target">{target(e)}</span>
                </span>
                <span class="meta">
                  {clock(e.at)}
                  {#if e.actorKind === "client"} · {$t("canvas.activity.viaMcp", { default: "via MCP" })}{/if}
                  {#if e.actorKind === "agent"} · {$t("canvas.activity.agent", { default: "agent" })}{/if}
                  {#if e.via === "generate"} · {$t("canvas.activity.viaGenerate", { default: "image generation" })}{/if}
                  {#if aiActivity.follow && aiActivity.followOrigin === baseOrigin(e.origin)} · {$t("canvas.activity.following", { default: "following" })}{/if}
                </span>
              </span>
            </button>
          </li>
        {/each}
      </ol>
    {/if}
  {:else}
    <McpServersPanel nodeId={aiActivity.mcpNodeId} />
  {/if}
</section>

<style>
  .panel {
    width: min(360px, calc(100vw - 32px));
    max-height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    box-sizing: border-box;
    font-size: 13px;
    color: var(--text);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 12px 36px rgb(0 0 0 / 0.3);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 9px;
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .tabs button {
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    border: 0;
    border-radius: 7px;
    padding: 4px 10px;
    color: var(--text-muted);
    background: transparent;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    background: var(--popup-bg, var(--surface));
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.15);
  }
  .icon {
    font: inherit;
    font-size: 18px;
    line-height: 1;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: 7px;
    color: var(--text-muted);
    background: transparent;
    cursor: pointer;
  }
  .icon:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
  }
  select {
    font: inherit;
    font-size: 12px;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 2px 6px;
    max-width: 150px;
  }
  .empty {
    margin: 6px 2px;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.45;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px;
    border: 0;
    border-radius: 9px;
    font: inherit;
    text-align: left;
    color: inherit;
    background: transparent;
    cursor: pointer;
  }
  .row:hover,
  .row:focus-visible {
    background: color-mix(in srgb, var(--text) 7%, transparent);
  }
  .chip {
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 10px;
    font-weight: 700;
    color: #fff;
    background: var(--c);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    gap: 0 4px;
    line-height: 1.35;
  }
  .line em {
    font-style: normal;
    color: var(--text-muted);
  }
  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 200px;
    font-weight: 500;
  }
  .meta {
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
