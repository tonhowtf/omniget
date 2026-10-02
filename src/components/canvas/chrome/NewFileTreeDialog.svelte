<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const engine = useEngine();
  const tr = (key: string, fallback: string) => $t(key, { default: fallback }) as string;

  let dir = $state(workspaces.byId(engine.store.workspaceId)?.workingDirectory ?? "");
  let input = $state<HTMLInputElement | null>(null);

  onMount(() => input?.focus());

  async function browse() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, defaultPath: dir || undefined });
      if (typeof picked === "string") dir = picked;
    } catch {
    }
  }

  function create() {
    const root = dir.trim();
    if (!root) return;
    const name = root.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || root;
    engine.spawn("filetree", null, { data: { rootPath: root }, title: name });
    onclose();
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      create();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="dialog" role="dialog" aria-modal="true" aria-label={tr("canvas.newFileTree.title", "New File Tree")} tabindex="-1" onkeydown={key}>
  <h2>{tr("canvas.newFileTree.title", "New File Tree")}</h2>
  <label for="nft-dir">{tr("canvas.newFileTree.dir", "Working Directory")}</label>
  <div class="row">
    <input id="nft-dir" bind:this={input} bind:value={dir} placeholder="~/projects/app" spellcheck="false" autocomplete="off" />
    <button type="button" class="btn" onclick={() => void browse()}>{tr("canvas.newFileTree.browse", "Browse…")}</button>
  </div>
  <div class="actions">
    <button type="button" class="btn" onclick={onclose}>{tr("canvas.newPortal.cancel", "Cancel")}</button>
    <button type="button" class="btn primary" disabled={!dir.trim()} onclick={create}>{tr("canvas.newPortal.create", "Create")}</button>
  </div>
</div>

<style>
  .dialog {
    width: min(440px, calc(100vw - 32px));
    box-sizing: border-box;
    padding: 20px 22px 18px;
    border-radius: 16px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.28);
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 10px;
    outline: none;
  }
  h2 {
    margin: 0 0 4px;
    text-align: center;
    font-size: 15px;
    font-weight: 600;
  }
  label {
    font-size: 13px;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  input {
    flex: 1;
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    min-width: 0;
    outline: none;
  }
  input:focus {
    border-color: var(--accent);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .btn {
    font: inherit;
    font-size: 13px;
    padding: 5px 14px;
    border-radius: 7px;
    border: none;
    background: color-mix(in srgb, var(--text) 9%, transparent);
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
