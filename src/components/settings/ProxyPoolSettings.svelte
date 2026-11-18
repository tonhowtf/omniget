<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import {
    emptyPool, proxyState, MAX_CHECK_ROUNDS, POOL_PAGE,
    type PoolChange, type PoolDetected, type PoolProxy, type PoolRotation, type PoolView,
  } from "$lib/proxy-pool";

  let pool = $state<PoolView>(emptyPool());
  let draft = $state("");
  let busy = $state<"" | "add" | "detect" | "check" | "change">("");
  let notice = $state("");
  let failed = $state("");
  let shown = $state(POOL_PAGE);
  let visible = $derived(pool.proxies.slice(0, shown));
  let anyFailed = $derived(pool.proxies.some((p) => proxyState(p) === "failed"));

  async function run(kind: typeof busy, work: () => Promise<PoolView | void>) {
    if (busy) return;
    busy = kind;
    notice = "";
    failed = "";
    try {
      const next = await work();
      if (next) pool = next;
    } catch (e) {
      failed = String(e);
    } finally {
      busy = "";
    }
  }

  onMount(() => { void run("change", () => invoke<PoolView>("proxy_pool_status")); });

  function summary(out: PoolChange) {
    const parts = [$t("settings.proxy_pool.added", { count: out.added ?? 0 }) as string];
    if (out.rejected?.length) parts.push($t("settings.proxy_pool.rejected", { count: out.rejected.length }) as string);
    return parts.join(" ");
  }

  function add() {
    const text = draft.trim();
    if (!text) return;
    void run("add", async () => {
      const out = await invoke<PoolChange>("proxy_pool_add", { text });
      notice = summary(out);
      if (!out.rejected?.length) draft = "";
      return out;
    });
  }

  function detect() {
    void run("detect", async () => {
      const found = await invoke<PoolDetected>("proxy_pool_detect");
      const fresh = found.found.filter((f) => !f.in_pool).map((f) => f.proxy);
      if (found.total === 0) { notice = $t("settings.proxy_pool.detect_none") as string; return; }
      if (fresh.length === 0) { notice = $t("settings.proxy_pool.detect_known") as string; return; }
      const out = await invoke<PoolChange>("proxy_pool_add", { text: fresh.join("\n") });
      notice = $t("settings.proxy_pool.detect_added", { count: out.added ?? 0 }) as string;
      return out;
    });
  }

  function check(ids: string[]) {
    void run("check", async () => {
      let out = await invoke<PoolChange>("proxy_pool_check", { ids, url: null });
      pool = out;
      for (let round = 1; ids.length === 0 && (out.unchecked ?? 0) > 0 && round < MAX_CHECK_ROUNDS; round++) {
        out = await invoke<PoolChange>("proxy_pool_check", { ids, url: null });
        pool = out;
      }
      return out;
    });
  }

  const change = (args: { enabled?: boolean; rotation?: PoolRotation }) =>
    run("change", () => invoke<PoolView>("proxy_pool_set", { enabled: args.enabled ?? null, rotation: args.rotation ?? null }));
  const remove = (ids: string[], failing: boolean) =>
    run("change", () => invoke<PoolView>("proxy_pool_remove", { ids, failing }));

  function label(proxy: PoolProxy): string {
    switch (proxyState(proxy)) {
      case "failed": return $t("settings.proxy_pool.status_failed") as string;
      case "resting": return $t("settings.proxy_pool.status_resting", { secs: proxy.restingSecs }) as string;
      case "ok": return [$t("settings.proxy_pool.status_ok") as string, proxy.checked?.country, proxy.checked?.ms != null ? `${proxy.checked.ms} ms` : null].filter(Boolean).join(" · ");
      default: return $t("settings.proxy_pool.status_untested") as string;
    }
  }
</script>

<section class="pool" data-setting-key="settings.proxy_pool" aria-labelledby="settings-proxy-pool-title">
  <h3 class="pool-title" id="settings-proxy-pool-title">{$t("settings.proxy_pool.title")}</h3>
  <p class="pool-desc"><bdi>{$t("settings.proxy_pool.desc")}</bdi></p>

  <SettingRow id="settings-proxy-pool-enabled" label={$t("settings.proxy_pool.enable") as string} help={$t("settings.proxy_pool.enable_desc") as string}>
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="toggle" class:on={pool.enabled} role="switch" aria-checked={pool.enabled} aria-labelledby={`${id}-label`}
        aria-describedby={describedBy} disabled={busy !== ""} onclick={() => change({ enabled: !pool.enabled })}><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>
  <SettingRow id="settings-proxy-pool-rotation" label={$t("settings.proxy_pool.rotation") as string} help={$t("settings.proxy_pool.rotation_desc") as string}>
    {#snippet control({ id, describedBy })}
      <select {id} class="select" aria-describedby={describedBy} disabled={busy !== ""} value={pool.rotation}
        onchange={(e) => change({ rotation: (e.target as HTMLSelectElement).value as PoolRotation })}>
        <option value="sticky">{$t("settings.proxy_pool.rotation_sticky")}</option>
        <option value="round_robin">{$t("settings.proxy_pool.rotation_round_robin")}</option>
      </select>
    {/snippet}
  </SettingRow>

  <div class="pool-add">
    <label class="pool-label" for="settings-proxy-pool-add">{$t("settings.proxy_pool.add_label")}</label>
    <p class="pool-desc" id="settings-proxy-pool-add-help"><bdi>{$t("settings.proxy_pool.add_desc")}</bdi></p>
    <textarea id="settings-proxy-pool-add" class="input pool-text" dir="ltr" rows="3" spellcheck="false" autocomplete="off"
      aria-describedby="settings-proxy-pool-add-help" placeholder="socks5://127.0.0.1:7890" bind:value={draft}></textarea>
    <div class="pool-actions">
      <button type="button" class="btn btn-primary btn-sm" disabled={busy !== "" || !draft.trim()} onclick={add}>{$t("settings.proxy_pool.add_action")}</button>
      <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== ""} onclick={detect}>{$t("settings.proxy_pool.detect_action")}</button>
    </div>
  </div>

  <p class="pool-notice" role="status" aria-live="polite">{notice}</p>
  {#if failed}<p class="pool-error" role="alert"><bdi>{failed}</bdi></p>{/if}

  {#if pool.total === 0}
    <p class="pool-empty">{$t("settings.proxy_pool.empty")}</p>
  {:else}
    <div class="pool-actions">
      <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== ""} onclick={() => check([])}>
        {busy === "check" ? $t("settings.proxy_pool.testing") : $t("settings.proxy_pool.test_all")}
      </button>
      {#if anyFailed}
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== ""} onclick={() => remove([], true)}>{$t("settings.proxy_pool.remove_failing")}</button>
      {/if}
    </div>
    {#if pool.enabled && pool.available === 0}<p class="pool-error" role="status">{$t("settings.proxy_pool.all_resting")}</p>{/if}
    <ul class="pool-list">
      {#each visible as proxy (proxy.id)}
        {@const state = proxyState(proxy)}
        <li class="pool-row">
          <span class="pool-address" dir="ltr"><bdi>{proxy.proxy}</bdi></span>
          {#if proxy.current}<span class="tag pool-current">{$t("settings.proxy_pool.status_current")}</span>{/if}
          <span class="tag" class:is-ok={state === "ok"} class:is-bad={state === "failed"} title={proxy.checked?.error ?? undefined}>{label(proxy)}</span>
          <button type="button" class="btn btn-secondary btn-sm" disabled={busy !== ""} aria-label={`${$t("settings.proxy_pool.remove") as string} ${proxy.proxy}`}
            onclick={() => remove([proxy.id], false)}>{$t("settings.proxy_pool.remove")}</button>
        </li>
      {/each}
    </ul>
    {#if pool.proxies.length > shown}
      <button type="button" class="btn btn-secondary btn-sm" onclick={() => { shown += POOL_PAGE; }}>{$t("settings.proxy_pool.show_more")}</button>
    {/if}
  {/if}
</section>

<style>
  .pool { display: flex; flex-direction: column; gap: var(--space-2); margin-block-start: var(--space-5); min-inline-size: 0; }
  .pool-title { margin: 0; font-size: var(--text-md); font-weight: 600; color: var(--text); }
  .pool-label { font-size: 13px; line-height: 20px; color: var(--text); }
  .pool-desc, .pool-empty, .pool-notice { margin: 0; font-size: 12px; line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .pool-notice:empty { display: none; }
  .pool-error { margin: 0; font-size: 12px; line-height: 18px; color: var(--error); overflow-wrap: anywhere; }
  .pool-add { display: flex; flex-direction: column; gap: var(--space-1); padding-block-start: var(--space-2); border-block-start: 1px solid var(--separator); }
  .pool-text { inline-size: 100%; font-family: var(--font-mono); font-size: var(--text-sm); }
  .pool-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .pool-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .pool-row { display: flex; align-items: center; gap: var(--space-2); min-block-size: var(--row-base); padding-block: var(--space-1); min-inline-size: 0; }
  .pool-row + .pool-row { border-block-start: 1px solid var(--separator); }
  .pool-address { flex: 1 1 auto; min-inline-size: 0; font-family: var(--font-mono); font-size: var(--text-sm); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pool-current { color: var(--accent-text); }
  .is-ok { color: var(--success); }
  .is-bad { color: var(--error); }
</style>
