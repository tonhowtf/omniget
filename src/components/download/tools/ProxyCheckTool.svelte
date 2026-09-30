<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Route = { ok: boolean; ip: string | null; country: string | null; ms: number | null; error: string | null };
  type Site = { ok: boolean; title: string | null; ms: number | null; error: string | null };
  type Check = { proxy: string; via_proxy: Route; direct: Route; site: Site | null };

  let proxy = $state("");
  let url = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  let result = $state<Check | null>(null);
  let testedProxy = $state("");

  const hides = $derived(
    result?.via_proxy.ok && result.direct.ok ? result.via_proxy.ip !== result.direct.ip : null,
  );

  async function check() {
    if (loading) return;
    loading = true;
    error = null;
    result = null;
    try {
      result = await invoke<Check>("check_proxy", { proxy: proxy.trim() || null, url: url.trim() || null });
      testedProxy = proxy.trim();
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("downloads.tools.proxy_check.error");
    } finally {
      loading = false;
    }
  }

  /** `socks5://user:pw@host:1080` or `host:port` → the app's proxy settings. */
  function toSettings(raw: string) {
    const withScheme = raw.includes("://") ? raw : `http://${raw}`;
    const u = new URL(withScheme);
    const scheme = u.protocol.replace(":", "");
    return {
      enabled: true,
      proxy_type: scheme.startsWith("socks") ? "socks5" : scheme === "https" ? "https" : "http",
      host: u.hostname,
      port: Number(u.port),
      username: decodeURIComponent(u.username),
      password: decodeURIComponent(u.password),
    };
  }

  async function useIt() {
    try {
      await updateSettings({ proxy: toSettings(testedProxy) });
      showToast("success", $t("downloads.tools.proxy_check.saved"));
    } catch {
      showToast("error", $t("common.error"));
    }
  }

  function flagOf(cc: string | null): string {
    if (!cc || !/^[A-Z]{2}$/.test(cc)) return "";
    return String.fromCodePoint(...[...cc].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65));
  }
</script>

<div class="pc" data-agent-readiness={loading ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.proxy_check.lead")}</p>
  <form
    class="fields"
    onsubmit={(e) => {
      e.preventDefault();
      check();
    }}
  >
    <input
      bind:value={proxy}
      placeholder={$t("downloads.tools.proxy_check.proxy_placeholder")}
      aria-label={$t("downloads.tools.proxy_check.proxy_label")}
      spellcheck="false"
      autocomplete="off"
    />
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.proxy_check.url_placeholder")}
      aria-label={$t("downloads.tools.proxy_check.url_label")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={loading}>
      {loading ? $t("downloads.tools.proxy_check.loading") : $t("downloads.tools.proxy_check.check")}
    </button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="grid">
      <div class="card" class:bad={!result.via_proxy.ok}>
        <span class="dim small">{$t("downloads.tools.proxy_check.through", { proxy: result.proxy })}</span>
        {#if result.via_proxy.ok}
          <strong>{flagOf(result.via_proxy.country)} {result.via_proxy.ip}</strong>
          <span class="dim small">{result.via_proxy.country ?? ""} · {result.via_proxy.ms} ms</span>
        {:else}
          <strong class="err">{$t("downloads.tools.proxy_check.failed")}</strong>
          <span class="dim small">{result.via_proxy.error}</span>
        {/if}
      </div>
      <div class="card">
        <span class="dim small">{$t("downloads.tools.proxy_check.without")}</span>
        {#if result.direct.ok}
          <strong>{flagOf(result.direct.country)} {result.direct.ip}</strong>
          <span class="dim small">{result.direct.country ?? ""} · {result.direct.ms} ms</span>
        {:else}
          <strong class="err">{$t("downloads.tools.proxy_check.failed")}</strong>
          <span class="dim small">{result.direct.error}</span>
        {/if}
      </div>
    </div>

    {#if hides === false}
      <p class="warn">{$t("downloads.tools.proxy_check.same_ip")}</p>
    {/if}

    {#if result.site}
      <div class="card" class:bad={!result.site.ok}>
        <span class="dim small">{$t("downloads.tools.proxy_check.site")}</span>
        {#if result.site.ok}
          <strong>{result.site.title ?? "✓"}</strong>
          <span class="dim small">{$t("downloads.tools.proxy_check.site_ok", { ms: result.site.ms ?? 0 })}</span>
        {:else}
          <strong class="err">{$t("downloads.tools.proxy_check.site_failed")}</strong>
          <span class="dim small">{translateBackendError(result.site.error ?? "", $t)}</span>
        {/if}
      </div>
    {/if}

    {#if testedProxy && result.via_proxy.ok}
      <button class="button primary use" onclick={useIt}>{$t("downloads.tools.proxy_check.use")}</button>
    {/if}
  {/if}
</div>

<style>
  .pc {
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
  .fields {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .fields input {
    flex: 1;
    min-width: 180px;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .fields input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error,
  .err {
    margin: 0;
    color: var(--danger, #d33);
  }
  .error {
    font-size: var(--text-sm);
  }
  .warn {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--warning, #b7791f);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-2);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-width: 0;
  }
  .card strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card.bad {
    box-shadow: inset 0 0 0 var(--hairline) var(--danger, #d33);
  }
  .use {
    align-self: flex-start;
  }
</style>
