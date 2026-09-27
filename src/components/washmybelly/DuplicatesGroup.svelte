<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes, shortenPath } from "$lib/washmybelly/format";
  import { baseName, trashId, type DupGroup } from "$lib/washmybelly/clean";

  /// One set of identical files, the way Photos shows duplicates: the file
  /// kept wears "Keep", every other copy has a box. Choosing "Keep" on
  /// another file moves it there (the parent recomputes the selection).
  let {
    group,
    keep,
    selected,
    onKeep,
    onToggle,
    disabled = false,
  }: {
    group: DupGroup;
    keep: string;
    selected: Set<string>;
    onKeep: (path: string) => void;
    onToggle: (path: string) => void;
    disabled?: boolean;
  } = $props();

  let name = $derived(baseName(keep));
  let copies = $derived(group.files.length);
  let chosen = $derived(group.files.filter((f) => selected.has(trashId(f.path))).length);
</script>

<section class="wmb-dg group" aria-label={name}>
  <header class="head">
    <span class="name" title={name}>{name}</span>
    <span class="meta">
      {$t("washmybelly.clean.dupes.copies", { count: copies, size: formatBytes(group.size) })}
      {#if chosen > 0}· <strong>{$t("washmybelly.clean.dupes.frees", { size: formatBytes(chosen * group.size) })}</strong>{/if}
    </span>
  </header>
  {#each group.files as f (f.path)}
    {@const isKeep = f.path === keep}
    <div class="group-row file" class:keep={isKeep}>
      {#if isKeep}
        <span class="keep-tag tag tag-success">{$t("washmybelly.clean.dupes.keep")}</span>
      {:else}
        <input
          type="checkbox"
          checked={selected.has(trashId(f.path))}
          {disabled}
          aria-label={$t("washmybelly.clean.dupes.trash_copy", { name: shortenPath(f.path, 60) }) as string}
          onchange={() => onToggle(f.path)}
        />
      {/if}
      <div class="group-row-content">
        <span class="path" title={f.path}>{shortenPath(f.path, 72)}</span>
        {#if f.mtime > 0}
          <span class="group-row-sub">{new Date(f.mtime * 1000).toLocaleDateString()}</span>
        {/if}
      </div>
      {#if !isKeep}
        <button type="button" class="btn btn-ghost btn-sm" {disabled} onclick={() => onKeep(f.path)}>{$t("washmybelly.clean.dupes.keep_this")}</button>
      {/if}
    </div>
  {/each}
</section>

<style>
  .wmb-dg {
    display: flex;
    flex-direction: column;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--fill-1, transparent);
    border-bottom: var(--hairline) solid var(--separator, var(--content-border));
  }
  .name {
    font-weight: 600;
    font-size: var(--text-base);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .meta {
    margin-left: auto;
    flex-shrink: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .meta strong {
    color: var(--text);
    font-weight: 600;
  }
  .file {
    min-height: 40px;
  }
  .file + .file {
    border-top: var(--hairline) solid var(--separator, var(--content-border));
  }
  .file input {
    width: 16px;
    height: 16px;
    margin: 0;
    flex-shrink: 0;
    accent-color: var(--accent);
  }
  .keep-tag {
    flex-shrink: 0;
  }
  .path {
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keep .path {
    font-weight: 500;
  }
</style>
