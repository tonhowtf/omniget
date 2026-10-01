<script lang="ts">
  import { t } from "$lib/i18n";
  import { toBody } from "$lib/canvas/to-body";
  import {
    errorText,
    landFloor,
    landPreview,
    type FloorView,
    type LandOutcome,
    type LandPreview,
  } from "$lib/canvas/floors/api";

  interface Props {
    workspaceId: string;
    floor: FloorView;
    onClose: () => void;
    onLanded: (outcome: LandOutcome) => void;
  }
  let { workspaceId, floor, onClose, onLanded }: Props = $props();

  let preview = $state<LandPreview | null>(null);
  let target = $state("");
  let deleteBranch = $state(false);
  let keepFloor = $state(false);
  let message = $state("");
  let loading = $state(false);
  let landing = $state(false);
  let error = $state("");

  async function load(into?: string) {
    loading = true;
    error = "";
    try {
      preview = await landPreview(workspaceId, floor.id, into);
      target = preview.target;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void floor.id;
    load();
  });

  const added = $derived(preview?.files.reduce((a, f) => a + (f.added ?? 0), 0) ?? 0);
  const deleted = $derived(preview?.files.reduce((a, f) => a + (f.deleted ?? 0), 0) ?? 0);
  const canLand = $derived(!!preview && !preview.blocker && !loading && !landing);

  async function land() {
    if (!canLand) return;
    landing = true;
    error = "";
    try {
      const o = await landFloor(workspaceId, floor.id, {
        into: target || undefined,
        deleteBranch,
        keepFloor,
        message: message.trim() || undefined,
      });
      onLanded(o);
    } catch (e) {
      error = errorText(e);
      load(target);
    } finally {
      landing = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="scrim" use:toBody role="presentation" onclick={(e) => e.target === e.currentTarget && onClose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={$t("canvas.floors.landTitle", { default: "Land floor" })}>
    <header>
      <h3>{$t("canvas.floors.landTitle", { default: "Land floor" })} · {floor.name}</h3>
      <button type="button" class="icon" onclick={onClose} aria-label={$t("canvas.floors.close", { default: "Close" })}>✕</button>
    </header>

    <div class="route">
      <div class="side">
        <span class="muted">{$t("canvas.floors.from", { default: "Floor" })}</span>
        <code>{preview?.floorBranch ?? floor.branch ?? "…"}</code>
      </div>
      <span class="plane" aria-hidden="true">✈</span>
      <div class="side">
        <span class="muted">{$t("canvas.floors.into", { default: "Into" })}</span>
        <select bind:value={target} onchange={() => load(target)} disabled={loading || landing}>
          {#if preview}
            {#each preview.branches.includes(target) ? preview.branches : [target, ...preview.branches] as b (b)}
              <option value={b}>{b}</option>
            {/each}
          {/if}
        </select>
      </div>
    </div>

    {#if loading}
      <p class="muted">{$t("canvas.floors.checking", { default: "Checking the merge…" })}</p>
    {:else if preview}
      <p class="summary">
        {#if preview.upToDate}
          {$t("canvas.floors.upToDate", { default: "Nothing to land: the target already has these commits." })}
        {:else}
          {$t("canvas.floors.landSummary", { default: "{{commits}} commits, {{files}} files (+{{added}} −{{deleted}})", commits: preview.commits.length, files: preview.files.length, added, deleted })}
          {#if preview.fastForward}· {$t("canvas.floors.fastForward", { default: "fast-forward" })}{/if}
          {#if preview.createsTarget}· {$t("canvas.floors.createsTarget", { default: "creates the branch" })}{/if}
          {#if preview.targetCheckedOut}· {$t("canvas.floors.movesGround", { default: "updates the ground's files" })}{/if}
        {/if}
      </p>
      {#if preview.blocker}
        <p class="blocker">{preview.blocker}</p>
      {/if}
      {#if preview.conflicts.length}
        <details open>
          <summary>{$t("canvas.floors.conflicts", { default: "Conflicts" })} ({preview.conflicts.length})</summary>
          <ul>{#each preview.conflicts as c (c)}<li><code>{c}</code></li>{/each}</ul>
        </details>
      {/if}
      {#if preview.commits.length}
        <details>
          <summary>{$t("canvas.floors.commits", { default: "Commits" })} ({preview.commits.length})</summary>
          <ul>{#each preview.commits as c (c)}<li><code>{c}</code></li>{/each}</ul>
        </details>
      {/if}
      {#if preview.files.length}
        <details>
          <summary>{$t("canvas.floors.files", { default: "Files" })} ({preview.files.length})</summary>
          <ul class="files">
            {#each preview.files as f (f.path)}
              <li><code>{f.path}</code><span class="add">+{f.added ?? "bin"}</span><span class="del">−{f.deleted ?? "bin"}</span></li>
            {/each}
          </ul>
        </details>
      {/if}
      {#if !preview.fastForward && !preview.upToDate}
        <label>
          <span class="muted">{$t("canvas.floors.message", { default: "Merge message" })}</span>
          <input bind:value={message} placeholder={`Land floor '${floor.name}' (${preview.floorBranch}) into ${target}`} />
        </label>
      {/if}
      <label class="check"><input type="checkbox" bind:checked={keepFloor} />{$t("canvas.floors.keepFloor", { default: "Keep the floor after landing" })}</label>
      {#if preview.floorBranch !== target}
        <label class="check"><input type="checkbox" bind:checked={deleteBranch} disabled={keepFloor} />{$t("canvas.floors.deleteBranch", { default: "Delete the floor branch" })}</label>
      {/if}
    {/if}
    {#if error}
      <p class="blocker">{error}</p>
    {/if}
    <footer>
      <button type="button" onclick={onClose}>{$t("canvas.floors.cancel", { default: "Cancel" })}</button>
      <button type="button" class="primary" disabled={!canLand} onclick={land}>
        {landing ? $t("canvas.floors.landing", { default: "Landing…" }) : $t("canvas.floors.land", { default: "Land" })}
      </button>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 16px;
  }
  .dialog {
    width: min(520px, 100%);
    max-height: 85vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 13px;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  footer {
    justify-content: flex-end;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .route {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .side code {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .plane {
    font-size: 22px;
    color: var(--accent);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .summary {
    margin: 0;
  }
  .blocker {
    margin: 0;
    color: var(--danger);
    white-space: pre-wrap;
  }
  ul {
    margin: 4px 0;
    padding-left: 18px;
    max-height: 160px;
    overflow: auto;
  }
  .files li {
    display: flex;
    gap: 8px;
  }
  .files code {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .add {
    color: #46a758;
  }
  .del {
    color: var(--danger);
  }
  code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]),
  select {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
  }
  button {
    font: inherit;
    color: inherit;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 12px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .icon {
    border: none;
    padding: 2px 6px;
  }
  .primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
    border-color: var(--accent);
  }
</style>
