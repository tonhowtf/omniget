<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { addRoots, baseName } from "$lib/washmybelly/clean";
  import { tildify } from "$lib/washmybelly/format";

  /// The folders a scan looks in: removable chips plus "Choose folder"
  /// (the system folder picker, several at once). With no folder chosen,
  /// `placeholder` says what the scan falls back to.
  let {
    roots = $bindable<string[]>([]),
    placeholder = "",
    disabled = false,
  }: { roots?: string[]; placeholder?: string; disabled?: boolean } = $props();

  async function choose() {
    try {
      const picked = await openDialog({ directory: true, multiple: true });
      roots = addRoots(roots, picked as string | string[] | null);
    } catch {
      // The picker was closed or is not available: nothing changes.
    }
  }

  function remove(p: string) {
    roots = roots.filter((r) => r !== p);
  }
</script>

<div class="wmb-roots">
  <span class="label">{$t("washmybelly.clean.roots.label")}</span>
  <ul>
    {#each roots as r (r)}
      <li class="chip" title={r}>
        <span>{baseName(r) || tildify(r)}</span>
        <button type="button" class="x" {disabled} aria-label={$t("washmybelly.clean.roots.remove", { folder: baseName(r) }) as string} onclick={() => remove(r)}>×</button>
      </li>
    {:else}
      {#if placeholder}<li class="ph">{placeholder}</li>{/if}
    {/each}
  </ul>
  <button type="button" class="btn btn-secondary btn-sm" {disabled} onclick={choose}>{$t("washmybelly.clean.roots.choose")}</button>
</div>

<style>
  .wmb-roots {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    min-width: 0;
  }
  .label {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1, 4px);
    flex: 1;
    min-width: 0;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 22px;
    padding: 0 2px 0 var(--space-2);
    border-radius: 999px;
    background: var(--fill-2);
    font-size: var(--text-sm);
    color: var(--text);
    max-width: 220px;
  }
  .chip span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .x {
    all: unset;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .x:hover {
    background: var(--fill-3, var(--fill-2));
    color: var(--text);
  }
  .x:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
  }
  .ph {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-style: italic;
  }
</style>
