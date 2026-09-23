<script lang="ts">
  // Files surface: the working-tree changes of the thread's folder (HEAD →
  // disk) and the files this thread touched, both as folder trees. A click
  // opens the file in the Diff tab.
  import { t } from "$lib/i18n";
  import ChangedFilesCard from "$components/central/vcs/ChangedFilesCard.svelte";
  import { vcsWorkingChanges, vcsError, type ChangeSummary } from "$lib/central/vcs";
  import type { ChangedFile } from "$lib/central/threads-ui/timeline";
  import { relativeTo } from "$lib/central/threads-ui/format";

  let {
    cwd,
    touched,
    onopenfile,
  }: { cwd: string | null; touched: ChangedFile[]; onopenfile: (path: string) => void } = $props();

  let working = $state<ChangeSummary[] | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    if (!cwd) return;
    try {
      working = await vcsWorkingChanges(cwd);
      error = null;
    } catch (e) {
      working = null;
      error = vcsError(e).message;
    }
  }

  $effect(() => {
    cwd;
    void load();
  });

  let touchedRel = $derived(touched.map((f) => ({ ...f, path: relativeTo(f.path, cwd) })));
</script>

<svelte:window onfocus={load} />

<div class="files">
  <section>
    <h4>{$t("llm.central.threads.files.working")}</h4>
    {#if error}
      <p class="note">{error}</p>
    {:else if working && working.length}
      <ChangedFilesCard files={working} {onopenfile} />
    {:else if working}
      <p class="note">{$t("llm.central.threads.files.clean")}</p>
    {/if}
  </section>
  <section>
    <h4>{$t("llm.central.threads.files.touched")}</h4>
    {#if touchedRel.length}
      <ChangedFilesCard files={touchedRel} {onopenfile} />
    {:else}
      <p class="note">{$t("llm.central.threads.files.none")}</p>
    {/if}
  </section>
</div>

<style>
  .files { padding: 10px 12px; display: grid; gap: 16px; overflow: auto; height: 100%; box-sizing: border-box; align-content: start; }
  h4 { margin: 0 0 6px; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.03em; color: var(--text-muted); }
  .note { margin: 0; color: var(--text-muted); font-size: 12.5px; }
</style>
