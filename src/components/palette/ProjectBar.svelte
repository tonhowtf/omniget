<script lang="ts">
  import { t } from "$lib/i18n";
  import { workspace } from "$lib/palette/workspace.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let { onnew }: { onnew: () => void } = $props();

  let project = $derived(workspace.project!);

  function rename(value: string) {
    const name = value.trim().slice(0, 80);
    if (!name) return;
    project.name = name;
    workspace.touch();
  }

  function duplicate() {
    workspace.duplicate($t("palette.project.copy_name", { name: project.name }) as string);
  }

  function remove() {
    workspace.deleteProject((name) => $t("palette.project.deleted", { name }) as string);
  }
</script>

<div class="pl-bar">
  <label class="pl-bar-field">
    <span class="pl-sr">{$t("palette.project.switch")}</span>
    <select class="input" value={project.id} onchange={(e) => workspace.select(e.currentTarget.value)}>
      {#each workspace.file.projects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
  </label>
  <label class="pl-bar-field pl-bar-name">
    <span class="pl-sr">{$t("palette.project.rename")}</span>
    <input class="input" type="text" value={project.name} maxlength="80" onchange={(e) => rename(e.currentTarget.value)} />
  </label>
  <div class="pl-bar-actions">
    <button type="button" class="btn btn-ghost btn-sm" onclick={onnew}>{$t("palette.project.new")}</button>
    <button type="button" class="btn btn-ghost btn-sm" onclick={duplicate}>{$t("palette.project.duplicate")}</button>
    <button type="button" class="btn btn-ghost btn-sm" onclick={remove}>{$t("palette.project.delete")}</button>
  </div>
  <div class="pl-save" aria-live="polite">
    {#if workspace.saveState === "saving"}
      <StatusIcon name="running" size={12} /><span>{$t("palette.save.saving")}</span>
    {:else if workspace.saveState === "saved"}
      <StatusIcon name="done" size={13} /><span>{$t("palette.save.saved")}</span>
    {:else if workspace.saveState === "error"}
      <StatusIcon name="failed" size={13} />
      <span title={workspace.saveError ?? ""}>{$t("palette.save.error")}</span>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => workspace.flush()}>{$t("palette.save.retry")}</button>
    {/if}
  </div>
</div>
{#if workspace.saveState === "error" && workspace.saveError}
  <p class="pl-save-detail" role="alert">{$t("palette.save.error_detail", { error: workspace.saveError })}</p>
{/if}

<style>
  .pl-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .pl-bar-field select {
    max-width: 220px;
  }

  .pl-bar-name .input {
    width: 220px;
  }

  .pl-bar-actions {
    display: flex;
    gap: 2px;
  }

  .pl-save {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .pl-save-detail {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .pl-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
