<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { shortTime } from "$lib/stores/llm-jobs-store.svelte";

  type TokenView = {
    id: string;
    kind: "user" | "cli" | "job";
    name: string;
    scopes: string[];
    expires_ms: number | null;
    created_ms: number;
    last_used_ms: number | null;
    hint: string;
  };
  type Listing = {
    tokens: TokenView[];
    scopes: string[];
    legacy: { allowed: boolean; last: { at_ms: number; route: string } | null; count: number };
  };

  const expiryChoices = [0, 30, 90, 365];

  let listing = $state<Listing | null>(null);
  let name = $state("");
  let chosen = $state<string[]>(["jobs:read", "jobs:operate"]);
  let expiresDays = $state(90);
  let busy = $state(false);
  let fresh = $state<{ token: string; hint: string } | null>(null);

  async function load() {
    try {
      listing = await invoke<Listing>("llm_bridge_tokens");
    } catch (e) {
      showToast("error", String(e));
    }
  }

  onMount(() => void load());

  function scopeLabel(scope: string): string {
    return $t(`llm.jobs.tokens.scope_${scope.replace(":", "_")}`) as string;
  }

  function toggle(scope: string) {
    chosen = chosen.includes(scope) ? chosen.filter(s => s !== scope) : [...chosen, scope];
  }

  async function create() {
    busy = true;
    try {
      const out = await invoke<{ token: string; view: TokenView }>("llm_bridge_token_mint", {
        name: name.trim(),
        scopes: chosen,
        expiresDays: expiresDays || null,
      });
      fresh = { token: out.token, hint: out.view.hint };
      name = "";
      await load();
    } catch (e) {
      showToast("error", String(e));
    }
    busy = false;
  }

  async function copyFresh() {
    if (!fresh) return;
    try {
      await navigator.clipboard.writeText(fresh.token);
      showToast("success", $t("llm.jobs.copied"));
      fresh = null;
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function revoke(id: string) {
    try {
      await invoke("llm_bridge_token_revoke", { id });
      await load();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function setLegacy(allowed: boolean) {
    try {
      await invoke("llm_bridge_legacy_set", { allowed });
      await load();
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<section class="stack">
  <h2 class="section-header-title">{$t("llm.jobs.tokens.title")}</h2>
  <p class="hint">{$t("llm.jobs.tokens.lede")}</p>

  {#if listing}
    {#if listing.legacy.last}
      <p class="notice" role="status">
        {$t("llm.jobs.tokens.legacy_seen", {
          route: listing.legacy.last.route,
          when: shortTime(listing.legacy.last.at_ms),
        })}
      </p>
    {/if}

    {#if listing.tokens.length === 0}
      <p class="hint">{$t("llm.jobs.tokens.empty")}</p>
    {:else}
      <ul class="tokens">
        {#each listing.tokens as tok (tok.id)}
          <li class="surface-card token">
            <div class="head">
              <strong>{tok.kind === "cli" ? $t("llm.jobs.tokens.kind_cli") : tok.name || tok.hint}</strong>
              <code class="path">{tok.hint}</code>
              <button type="button" class="button" onclick={() => void revoke(tok.id)}>
                {$t("llm.jobs.tokens.revoke")}
              </button>
            </div>
            <p class="hint">
              {tok.scopes.map(scopeLabel).join(" · ")}
              · {tok.last_used_ms
                ? $t("llm.jobs.tokens.last_used", { when: shortTime(tok.last_used_ms) })
                : $t("llm.jobs.tokens.never_used")}
              {#if tok.expires_ms}· {$t("llm.jobs.tokens.expires_at", { when: shortTime(tok.expires_ms) })}{/if}
            </p>
          </li>
        {/each}
      </ul>
    {/if}

    {#if fresh}
      <div class="surface-card fresh" role="status" data-agent-sensitive>
        <p>{$t("llm.jobs.tokens.created")}</p>
        <div class="head">
          <code class="path">{fresh.hint}</code>
          <button type="button" class="button active" onclick={() => void copyFresh()}>
            {$t("llm.jobs.tokens.copy")}
          </button>
          <button type="button" class="button" onclick={() => (fresh = null)}>{$t("llm.jobs.clear")}</button>
        </div>
      </div>
    {/if}

    <div class="surface-card form">
      <div class="form-row">
        <label class="field grow">
          <span class="field-label">{$t("llm.jobs.tokens.name")}</span>
          <input class="input" type="text" maxlength="80" bind:value={name} />
        </label>
        <label class="field">
          <span class="field-label">{$t("llm.jobs.tokens.expires")}</span>
          <select class="input" bind:value={expiresDays}>
            {#each expiryChoices as d (d)}
              <option value={d}>
                {d ? $t("llm.jobs.tokens.expires_days", { days: d }) : $t("llm.jobs.tokens.expires_never")}
              </option>
            {/each}
          </select>
        </label>
      </div>
      <fieldset class="scopes">
        <legend class="field-label">{$t("llm.jobs.tokens.scopes")}</legend>
        {#each listing.scopes as scope (scope)}
          <label class="check">
            <input type="checkbox" checked={chosen.includes(scope)} onchange={() => toggle(scope)} />
            {scopeLabel(scope)}
          </label>
        {/each}
      </fieldset>
      <div>
        <button type="button" class="button active" disabled={busy || !name.trim() || chosen.length === 0} onclick={create}>
          {$t("llm.jobs.tokens.create")}
        </button>
      </div>
    </div>

    <label class="check legacy">
      <input
        type="checkbox"
        checked={listing.legacy.allowed}
        onchange={(e) => void setLegacy((e.currentTarget as HTMLInputElement).checked)}
      />
      {$t("llm.jobs.tokens.legacy_toggle")}
    </label>
    <p class="hint">{$t("llm.jobs.tokens.legacy_hint")}</p>
  {/if}
</section>

<style>
  .stack,
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }
  .form,
  .fresh,
  .token {
    padding: var(--space-4);
  }
  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: flex-end;
  }
  .grow {
    flex: 1;
    min-width: 200px;
  }
  .tokens {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .head .path {
    margin-right: auto;
  }
  .scopes {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .notice {
    margin: 0;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--accent-soft);
    font-size: var(--text-sm);
  }
</style>
