<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluralKey } from "$lib/home/plural";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Profile = {
    url: string;
    added_at: number;
    last_sync_at: number | null;
    last_new: number;
    total: number;
    folder: string | null;
  };
  type SyncResult = { profile: Profile; new_files: number; skipped: number; first_sync: boolean };

  let url = $state("");
  let profiles = $state<Profile[]>([]);
  let syncing = $state<Set<string>>(new Set());
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      profiles = await invoke<Profile[]>("profile_sync_list");
    } catch {
      profiles = [];
    }
  });

  function message(e: any): string {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  }

  async function add() {
    const target = url.trim();
    if (!target) return;
    error = null;
    try {
      profiles = await invoke<Profile[]>("profile_sync_add", { url: target });
      url = "";
      await sync(target);
    } catch (e) {
      error = message(e);
    }
  }

  async function remove(p: Profile) {
    try {
      profiles = await invoke<Profile[]>("profile_sync_remove", { url: p.url });
    } catch (e) {
      showToast("error", message(e));
    }
  }

  async function sync(target: string) {
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.profile_sync.no_folder"));
      return;
    }
    syncing = new Set([...syncing, target]);
    try {
      const r = await invoke<SyncResult>("profile_sync_run", { url: target, outputDir });
      profiles = profiles.map((p) => (p.url === r.profile.url ? r.profile : p));
      showToast(
        "success",
        r.new_files
          ? $t(pluralKey("downloads.tools.profile_sync.got_new", r.new_files), { count: r.new_files })
          : $t("downloads.tools.profile_sync.nothing_new"),
      );
    } catch (e) {
      showToast("error", message(e));
    } finally {
      const next = new Set(syncing);
      next.delete(target);
      syncing = next;
    }
  }

  async function syncAll() {
    for (const p of profiles) await sync(p.url);
  }

  function when(ts: number | null): string {
    if (!ts) return $t("downloads.tools.profile_sync.never");
    return new Date(ts * 1000).toLocaleString();
  }
</script>

<div class="ps" data-agent-readiness={syncing.size ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.profile_sync.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      add();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.profile_sync.placeholder")}
      aria-label={$t("downloads.tools.profile_sync.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={!url.trim()}>{$t("downloads.tools.profile_sync.add")}</button>
  </form>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if profiles.length}
    <div class="head">
      <span class="dim">{$t(pluralKey("downloads.tools.profile_sync.count", profiles.length), { count: profiles.length })}</span>
      <button class="button" onclick={syncAll} disabled={syncing.size > 0}>{$t("downloads.tools.profile_sync.sync_all")}</button>
    </div>
    <ul class="list">
      {#each profiles as p (p.url)}
        <li class="profile">
          <div class="info">
            <span class="url" title={p.url}>{p.url}</span>
            <span class="dim small">
              {$t("downloads.tools.profile_sync.last", { when: when(p.last_sync_at) })}
              {#if p.last_sync_at}
                · {$t("downloads.tools.profile_sync.stats", { last: p.last_new, total: p.total })}{/if}
            </span>
          </div>
          <div class="actions">
            <button class="button small-btn" onclick={() => sync(p.url)} disabled={syncing.has(p.url)}>
              {syncing.has(p.url) ? $t("downloads.tools.profile_sync.syncing") : $t("downloads.tools.profile_sync.sync")}
            </button>
            <button class="link" onclick={() => remove(p)} disabled={syncing.has(p.url)}>
              {$t("downloads.tools.profile_sync.remove")}
            </button>
          </div>
        </li>
      {/each}
    </ul>
    <p class="dim small">{$t("downloads.tools.profile_sync.note")}</p>
  {:else}
    <p class="dim">{$t("downloads.tools.profile_sync.empty")}</p>
  {/if}
</div>

<style>
  .ps {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row,
  .head {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  .head {
    justify-content: space-between;
  }
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .profile {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .url {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .link {
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
  }
  .small-btn {
    font-size: var(--text-sm);
  }
</style>
