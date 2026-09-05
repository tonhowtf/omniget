<script lang="ts">
  /**
   * Lista de contas (seguidores, fãs, fantasmas…) com filtro, seleção,
   * export CSV e ações em massa que o pai executa.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { openUrl } from "$lib/tools/rt";
  import { exportCsv, profileUrl, usersCsv, type MiniUser } from "$lib/tools/ig.svelte";

  type Action = { id: string; label: string; primary?: boolean; danger?: boolean };
  let {
    users,
    title = "",
    csvName = "instagram",
    actions = [] as Action[],
    onaction,
    whitelist = null as Set<string> | null,
    ontogglewhitelist,
    counts = {} as Record<string, number>,
  }: {
    users: MiniUser[];
    title?: string;
    csvName?: string;
    actions?: Action[];
    onaction?: (id: string, users: MiniUser[]) => void;
    whitelist?: Set<string> | null;
    ontogglewhitelist?: (u: MiniUser) => void;
    counts?: Record<string, number>;
  } = $props();

  let filter = $state("");
  let selected = $state<Set<string>>(new Set());
  let limit = $state(100);

  $effect(() => {
    users;
    selected = new Set();
    limit = 100;
  });

  let shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? users.filter((u) => u.username.toLowerCase().includes(q) || u.full_name.toLowerCase().includes(q)) : users;
  });

  function toggle(pk: string) {
    const s = new Set(selected);
    if (s.has(pk)) s.delete(pk);
    else s.add(pk);
    selected = s;
  }

  async function csv() {
    const p = await exportCsv(csvName, ["username", "full_name", "id", "privacy", "verified", "url"], usersCsv(shown));
    if (p) showToast("success", $t("tools.common.done") as string);
  }

  let chosen = $derived(users.filter((u) => selected.has(u.pk)));
</script>

<section>
  <div class="head">
    <span class="group-label">{title} · {users.length}</span>
    <div class="btn-row">
      <input class="input input-sm" type="search" bind:value={filter} placeholder={$t("tools.ig.list.filter")} />
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set(shown.map((u) => u.pk)))}>{$t("tools.ig.grid.select_all")}</button>
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set())}>{$t("tools.ig.grid.select_none")}</button>
      <button class="btn btn-secondary btn-sm" type="button" disabled={!shown.length} onclick={csv}>CSV</button>
    </div>
  </div>
  <div class="group">
    {#if !shown.length}
      <div class="group-row"><div class="group-row-sub">{$t("tools.ig.list.empty")}</div></div>
    {/if}
    {#each shown.slice(0, limit) as u (u.pk)}
      <div class="group-row row">
        <label class="pick"><input type="checkbox" checked={selected.has(u.pk)} onchange={() => toggle(u.pk)} /></label>
        <img class="avatar" src={u.profile_pic_url} alt="" loading="lazy" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.visibility = "hidden")} />
        <div class="group-row-content">
          <div class="group-row-title">@{u.username} {#if u.is_verified}✓{/if} {#if u.is_private}<span class="tag">{$t("tools.ig.list.private")}</span>{/if} {#if whitelist?.has(u.pk)}<span class="tag tag-success">{$t("tools.ig.follow.whitelisted")}</span>{/if} {#if counts[u.pk] !== undefined}<span class="tag tag-accent">{counts[u.pk]}</span>{/if}</div>
          <div class="group-row-sub">{u.full_name}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if ontogglewhitelist}<button class="btn btn-ghost btn-sm" type="button" onclick={() => ontogglewhitelist?.(u)} title={$t("tools.ig.follow.whitelist_toggle")}>{whitelist?.has(u.pk) ? "★" : "☆"}</button>{/if}
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(profileUrl(u.username))}>{$t("tools.common.open")}</button>
        </div>
      </div>
    {/each}
    {#if shown.length > limit}
      <div class="group-row"><div class="group-row-content"><button class="btn btn-ghost btn-sm" type="button" onclick={() => (limit += 200)}>{$t("tools.ig.list.more")} ({shown.length - limit})</button></div></div>
    {/if}
  </div>
  {#if actions.length}
    <div class="actions btn-row end">
      <span class="group-row-sub">{chosen.length} {$t("tools.ig.list.selected")}</span>
      {#each actions as a (a.id)}
        <button class="btn btn-sm" class:btn-primary={a.primary} class:btn-danger={a.danger} class:btn-secondary={!a.primary && !a.danger} type="button" disabled={!chosen.length} onclick={() => onaction?.(a.id, chosen)}>{a.label}</button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .head { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
  .head .input { max-width: 200px; }
  .row { display: flex; align-items: center; gap: var(--space-2); }
  .pick { display: flex; }
  .avatar { width: 32px; height: 32px; border-radius: 50%; object-fit: cover; flex: none; background: var(--content-border); }
  .actions { margin-top: var(--space-2); align-items: center; }
  .btn-danger { color: var(--error, #e0303a); background: color-mix(in srgb, var(--error, #e0303a) 12%, transparent); }
</style>
