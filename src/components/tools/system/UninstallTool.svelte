<script lang="ts">
  /** Desinstalador (estudo 10, Kudu): lista, desinstala e manda as sobras para a lixeira. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, reveal, type ToolProgress } from "$lib/tools/rt";

  type App = { id: string; name: string; version: string; publisher: string; kind: string; path: string; bytes: number; needs_admin: boolean; key: string };
  type Leftover = { path: string; bytes: number };
  type Result = { ok: boolean; message: string; trashed: string[]; failed: string[] };

  let apps = $state<App[]>([]);
  let loading = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let query = $state("");
  let sort = $state<"name" | "size">("name");
  let picked = $state<App | null>(null);
  let leftovers = $state<Leftover[]>([]);
  let chosen = $state<Set<string>>(new Set());
  let busy = $state(false);
  let result = $state<Result | null>(null);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "uninstall") progress = p; });
    await refresh();
  });
  onDestroy(() => unlisten?.());

  async function refresh() {
    loading = true; picked = null; result = null;
    try { apps = await invoke<App[]>("tool_uninstall_list"); } catch (e) { showToast("error", errText(e)); } finally { loading = false; progress = null; }
  }
  async function pick(app: App) {
    picked = app; result = null; leftovers = []; chosen = new Set();
    try { leftovers = await invoke<Leftover[]>("tool_uninstall_leftovers", { app }); chosen = new Set(leftovers.map((l) => l.path)); }
    catch (e) { showToast("error", errText(e)); }
  }
  function toggle(p: string) { const s = new Set(chosen); if (s.has(p)) s.delete(p); else s.add(p); chosen = s; }
  async function uninstall() {
    if (!picked || busy) return;
    busy = true;
    try {
      result = await invoke<Result>("tool_uninstall_run", { app: picked, leftovers: [...chosen] });
      showToast(result.ok ? "success" : "error", result.message);
      if (result.ok) { const gone = picked.id; apps = apps.filter((a) => a.id !== gone); picked = null; }
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  let filtered = $derived(
    apps
      .filter((a) => !query || `${a.name} ${a.publisher} ${a.key}`.toLowerCase().includes(query.toLowerCase()))
      .sort((a, b) => (sort === "size" ? b.bytes - a.bytes : a.name.localeCompare(b.name))),
  );
  let leftoverBytes = $derived(leftovers.filter((l) => chosen.has(l.path)).reduce((n, l) => n + l.bytes, 0));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{apps.length} {$t("tools.uninstall.apps")}</div>
          <div class="group-row-sub">{loading ? (progress?.message ?? "…") : $t("tools.uninstall.hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="search" placeholder={$t("tools.hub.search_placeholder")} bind:value={query} style:width="12em" />
          <select class="input" bind:value={sort}><option value="name">{$t("tools.uninstall.by_name")}</option><option value="size">{$t("tools.uninstall.by_size")}</option></select>
          <button class="btn btn-secondary btn-sm" type="button" disabled={loading} onclick={refresh}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if picked}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{picked.name} <span class="dim">{picked.version}</span></div>
            <div class="group-row-sub mono">{picked.path || picked.key}</div>
            {#if picked.needs_admin}<div class="group-row-sub"><span class="tag tag-warning">{$t("tools.uninstall.needs_admin")}</span></div>{/if}
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (picked = null)}>×</button>
            <button class="btn btn-danger" type="button" disabled={busy} onclick={uninstall}>{busy ? $t("tools.common.working") : $t("tools.uninstall.run")}</button>
          </div>
        </div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.uninstall.leftovers")} <span class="dim">· {fmtBytes(leftoverBytes)}</span></div><div class="group-row-sub">{leftovers.length ? $t("tools.uninstall.leftovers_hint") : $t("tools.uninstall.no_leftovers")}</div></div></div>
        {#each leftovers as l (l.path)}
          <label class="group-row leftover">
            <input type="checkbox" checked={chosen.has(l.path)} onchange={() => toggle(l.path)} />
            <div class="group-row-content"><div class="group-row-sub mono">{l.path}</div></div>
            <div class="group-row-trailing btn-row"><span class="dim">{fmtBytes(l.bytes)}</span><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(l.path)}>{$t("tools.common.reveal")}</button></div>
          </label>
        {/each}
        {#if result}
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{result.message}</div>{#if result.trashed.length}<div class="group-row-sub">{result.trashed.length} {$t("tools.uninstall.trashed")}</div>{/if}{#each result.failed as f (f)}<div class="group-row-sub mono">{$t("tools.common.failed")}: {f}</div>{/each}</div></div>
        {/if}
      </div>
    </section>
  {/if}

  <section>
    <div class="group">
      {#each filtered as a (a.id)}
        <div class="group-row app" class:active={picked?.id === a.id}>
          <div class="group-row-content">
            <div class="group-row-title">{a.name} <span class="tag">{a.kind}</span>{#if a.needs_admin}<span class="tag tag-warning">admin</span>{/if}</div>
            <div class="group-row-sub">{a.version}{#if a.publisher} · {a.publisher}{/if}{#if a.bytes} · {fmtBytes(a.bytes)}{/if}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if a.path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(a.path)}>{$t("tools.common.reveal")}</button>{/if}
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => pick(a)}>{$t("tools.uninstall.select")}</button>
          </div>
        </div>
      {/each}
      {#if !loading && !filtered.length}<div class="group-row"><div class="group-row-sub">{$t("tools.hub.empty_title")}</div></div>{/if}
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .leftover { cursor: pointer; gap: var(--space-3); }
  .leftover input { accent-color: var(--accent); }
  .app.active { background: color-mix(in srgb, var(--accent) 10%, transparent); }
</style>
