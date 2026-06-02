<script lang="ts">
  import { t } from "$lib/i18n";
  import { open as openShell } from "@tauri-apps/plugin-shell";
  import PlatformLogo from "./PlatformLogo.svelte";

  type Account = {
    slug: string;
    alias: string;
    source_url: string | null;
    source_label: string | null;
    captured_at_ms: number;
    cookie_count: number;
    last_used_at_ms: number | null;
  };

  type Bucket = {
    domain: string;
    platform_kind: string;
    accounts: Account[];
  };

  type HealthItem = { status: string; age_days: number; expires_in_days: number };

  type Props = {
    bucket: Bucket;
    health?: Record<string, HealthItem>;
    testing?: Record<string, boolean>;
    onView: (domain: string, slug: string) => void;
    onExport: (domain: string, slug: string) => void;
    onRename: (domain: string, slug: string, currentAlias: string) => void;
    onClear: (domain: string, slug: string) => void;
    onAddAccount?: (domain: string) => void;
    onTest?: (domain: string, slug: string) => void;
    selected?: Record<string, boolean>;
    onToggleSelection?: (domain: string, slug: string, checked: boolean) => void;
  };

  let {
    bucket,
    health = {},
    testing = {},
    onView,
    onExport,
    onRename,
    onClear,
    onAddAccount,
    onTest,
    selected = {},
    onToggleSelection,
  }: Props = $props();

  let primaryAccount = $derived(bucket.accounts.find((a) => a.slug === "_default") ?? bucket.accounts[0]);
  let extraAccounts = $derived(bucket.accounts.filter((a) => a !== primaryAccount));
  let isEditing = $state(false);
  let editValue = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);

  let primaryHealth = $derived(primaryAccount ? health[primaryAccount.slug] : undefined);
  let sourceUrl = $derived.by(() => displaySourceUrl(primaryAccount?.source_url ?? null, bucket.domain));

  let status = $derived.by(() => {
    if (!primaryAccount) return "empty";
    if (primaryHealth) {
      if (primaryHealth.status === "fresh") return "fresh";
      if (primaryHealth.status === "stale") return "aging";
      return "stale";
    }
    const ageMs = Date.now() - primaryAccount.captured_at_ms;
    const dayMs = 24 * 60 * 60 * 1000;
    if (ageMs < dayMs) return "fresh";
    if (ageMs < 28 * dayMs) return "aging";
    return "stale";
  });

  let statusLabel = $derived.by(() => {
    if (status === "fresh") return $t("settings.cookies.status_fresh") as string;
    if (status === "aging") return $t("settings.cookies.status_aging") as string;
    if (status === "stale") return $t("settings.cookies.status_stale") as string;
    return $t("settings.cookies.status_empty") as string;
  });

  function fmtAgo(ms: number): string {
    const seconds = Math.floor((Date.now() - ms) / 1000);
    if (seconds < 60) return $t("settings.cookies.time_just_now") as string;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return ($t("settings.cookies.time_minutes", { count: String(minutes) }) as string) || `${minutes} min`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return ($t("settings.cookies.time_hours", { count: String(hours) }) as string) || `${hours}h`;
    const days = Math.floor(hours / 24);
    if (days < 30) return ($t("settings.cookies.time_days", { count: String(days) }) as string) || `${days}d`;
    const months = Math.floor(days / 30);
    if (months < 12) return ($t("settings.cookies.time_months", { count: String(months) }) as string) || `${months}m`;
    const years = Math.floor(months / 12);
    return ($t("settings.cookies.time_years", { count: String(years) }) as string) || `${years}y`;
  }

  function platformDisplayName(kind: string, domain: string): string {
    const map: Record<string, string> = {
      youtube: "YouTube",
      youtube_music: "YouTube Music",
      soundcloud: "SoundCloud",
      spotify: "Spotify",
      twitch: "Twitch",
      instagram: "Instagram",
      x_twitter: "X",
      vimeo: "Vimeo",
      tiktok: "TikTok",
      bilibili: "Bilibili",
      reddit: "Reddit",
      pinterest: "Pinterest",
      bluesky: "Bluesky",
    };
    return map[kind] ?? domain;
  }

  function startEdit() {
    if (!primaryAccount) return;
    editValue = primaryAccount.alias;
    isEditing = true;
    queueMicrotask(() => inputRef?.focus());
  }

  function commitEdit() {
    if (!primaryAccount) return;
    const trimmed = editValue.trim();
    if (trimmed && trimmed !== primaryAccount.alias) {
      onRename(bucket.domain, primaryAccount.slug, trimmed);
    }
    isEditing = false;
  }

  function cancelEdit() {
    isEditing = false;
    editValue = "";
  }

  function onAliasKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    }
  }

  function siteHostForDomain(domain: string): string {
    const host = domain.trim().replace(/^\./, "").toLowerCase();
    if (host.endsWith("bilibili.com")) return "www.bilibili.com";
    const parts = host.split(".").filter(Boolean);
    if (parts.length >= 2) return parts.slice(-2).join(".");
    return host;
  }

  function displaySourceUrl(rawUrl: string | null, domain: string): string | null {
    const host = siteHostForDomain(domain);
    if (!host) return rawUrl;
    const siteUrl = `https://${host}/`;
    if (!rawUrl) return null;
    try {
      const parsed = new URL(rawUrl);
      const sourceHost = siteHostForDomain(parsed.hostname);
      if (sourceHost === host) return siteUrl;
    } catch {
      return siteUrl;
    }
    return siteUrl;
  }

  async function openSourceUrl() {
    if (!sourceUrl) return;
    try {
      await openShell(sourceUrl);
    } catch (e) {
      console.warn("[cookies] failed to open source url", e);
    }
  }

  function keyFor(slug: string): string {
    return `${bucket.domain}__${slug}`;
  }
</script>

<article class="bucket-card" data-status={status}>
  <PlatformLogo kind={bucket.platform_kind as never} domain={bucket.domain} size={56} />

  <div class="meta">
    <div class="top-row">
      {#if isEditing}
        <input
          bind:this={inputRef}
          class="alias-input"
          bind:value={editValue}
          onkeydown={onAliasKey}
          onblur={commitEdit}
          aria-label={$t("settings.cookies.alias_aria") as string}
        />
      {:else}
        <button
          type="button"
          class="alias-display"
          onclick={startEdit}
          aria-label={$t("settings.cookies.alias_edit_aria") as string}
        >
          <span class="alias-text">{primaryAccount?.alias ?? bucket.domain}</span>
          <svg class="pencil" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
            <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
          </svg>
        </button>
      {/if}
    </div>

    <p class="platform-line">
      <span class="platform-name">{platformDisplayName(bucket.platform_kind, bucket.domain)}</span>
      <span class="sep">·</span>
      <span class="domain">{bucket.domain}</span>
      {#if primaryAccount}
        <span class="sep">·</span>
        <span class="count">{$t("settings.cookies.count_cookies", { count: String(primaryAccount.cookie_count) })}</span>
      {/if}
    </p>

    {#if sourceUrl}
      <p class="source-line">
        <span class="source-label">{$t("settings.cookies.captured_from")}</span>
        <button type="button" class="source-link" onclick={openSourceUrl} title={sourceUrl}>
          {sourceUrl}
        </button>
      </p>
    {/if}

    {#if primaryAccount}
      <p class="time-line">
        <span>{fmtAgo(primaryAccount.captured_at_ms)}</span>
        {#if primaryAccount.source_label}
          <span class="sep">·</span>
          <span class="via">{$t("settings.cookies.via_label", { label: primaryAccount.source_label })}</span>
        {/if}
        {#if primaryHealth && primaryHealth.status !== "fresh"}
          <span class="sep">·</span>
          {#if primaryHealth.status === "expired"}
            <span class="expiry expired">{$t("settings.cookies.expired")}</span>
          {:else}
            <span class="expiry stale">{$t("settings.cookies.expires_in", { days: String(primaryHealth.expires_in_days) })}</span>
          {/if}
        {/if}
      </p>
    {/if}

    {#if primaryAccount}
      <div class="actions">
        {#if onToggleSelection}
          <label class="select-account">
            <input
              type="checkbox"
              checked={!!selected[keyFor(primaryAccount.slug)]}
              onchange={(e) => onToggleSelection?.(bucket.domain, primaryAccount.slug, e.currentTarget.checked)}
            />
          </label>
        {/if}
        <button type="button" class="ghost-btn" onclick={() => onView(bucket.domain, primaryAccount.slug)}>
          {$t("settings.cookies.action_view")}
        </button>
        <button type="button" class="ghost-btn" onclick={() => onExport(bucket.domain, primaryAccount.slug)}>
          {$t("settings.cookies.action_export")}
        </button>
        <button type="button" class="ghost-btn" onclick={() => startEdit()}>
          {$t("settings.cookies.action_rename")}
        </button>
        {#if onTest}
          <button
            type="button"
            class="ghost-btn"
            disabled={testing[`${bucket.domain}__${primaryAccount.slug}`]}
            onclick={() => onTest?.(bucket.domain, primaryAccount.slug)}
          >
            {testing[`${bucket.domain}__${primaryAccount.slug}`]
              ? $t("settings.cookies.test_running")
              : $t("settings.cookies.action_test")}
          </button>
        {/if}
        <button type="button" class="ghost-btn danger" onclick={() => onClear(bucket.domain, primaryAccount.slug)}>
          {$t("settings.cookies.action_clear")}
        </button>
      </div>
    {/if}

    {#if extraAccounts.length > 0}
      <ul class="extra-accounts">
        {#each extraAccounts as account (account.slug)}
          <li class="extra-account" class:selectable={!!onToggleSelection}>
            {#if onToggleSelection}
              <label class="select-extra">
                <input
                  type="checkbox"
                  checked={!!selected[keyFor(account.slug)]}
                  onchange={(e) => onToggleSelection?.(bucket.domain, account.slug, e.currentTarget.checked)}
                />
              </label>
            {/if}
            <span class="extra-alias">{account.alias}</span>
            <span class="extra-count">{$t("settings.cookies.count_cookies", { count: String(account.cookie_count) })}</span>
            <div class="extra-actions">
              <button type="button" class="mini-btn" onclick={() => onView(bucket.domain, account.slug)} aria-label={$t("settings.cookies.action_view") as string}>
                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>
              </button>
              <button type="button" class="mini-btn" onclick={() => onExport(bucket.domain, account.slug)} aria-label={$t("settings.cookies.action_export") as string}>
                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"/></svg>
              </button>
              <button type="button" class="mini-btn danger" onclick={() => onClear(bucket.domain, account.slug)} aria-label={$t("settings.cookies.action_clear") as string}>
                <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}

    {#if onAddAccount && primaryAccount}
      <button type="button" class="add-account-btn" onclick={() => onAddAccount(bucket.domain)}>
        + {$t("settings.cookies.add_account_label")}
      </button>
    {/if}
  </div>

  <div class="status-dot" data-state={status} title={statusLabel} aria-label={statusLabel}></div>
</article>

<style>
  .bucket-card {
    position: relative;
    display: grid;
    grid-template-columns: 56px 1fr 12px;
    gap: 18px;
    padding: 18px;
    background: color-mix(in oklab, var(--button) 25%, transparent);
    border: 1px solid color-mix(in oklab, var(--content-border) 40%, transparent);
    border-radius: 14px;
    transition: border-color 120ms;
  }
  .bucket-card:hover {
    border-color: color-mix(in oklab, var(--content-border) 70%, transparent);
  }
  .meta {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .top-row {
    display: flex;
    align-items: center;
    min-height: 24px;
  }
  .alias-display {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--secondary);
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
    text-align: left;
    max-width: 100%;
  }
  .alias-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pencil {
    opacity: 0;
    transition: opacity 120ms;
    flex-shrink: 0;
  }
  .alias-display:hover .pencil,
  .alias-display:focus-visible .pencil {
    opacity: 0.6;
  }
  .alias-input {
    width: 100%;
    padding: 2px 8px;
    background: color-mix(in oklab, var(--button) 60%, transparent);
    border: 1px solid var(--accent);
    border-radius: 6px;
    color: var(--secondary);
    font-size: 16px;
    font-weight: 600;
    outline: none;
  }
  .platform-line,
  .source-line,
  .time-line {
    margin: 0;
    font-size: 12px;
    color: var(--secondary);
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .platform-name {
    font-weight: 500;
  }
  .domain {
    font-family: ui-monospace, "Cascadia Code", monospace;
    color: var(--tertiary);
  }
  .count {
    color: var(--tertiary);
  }
  .sep {
    margin: 0 6px;
    color: var(--tertiary);
    opacity: 0.5;
  }
  .source-line {
    color: var(--tertiary);
  }
  .source-label {
    color: var(--tertiary);
  }
  .source-link {
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: bottom;
  }
  .source-link:hover {
    filter: brightness(1.2);
  }
  .time-line {
    color: var(--tertiary);
    font-size: 11px;
  }
  .expiry {
    font-weight: 600;
  }
  .expiry.stale {
    color: #f4a72b;
  }
  .expiry.expired {
    color: #d33;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .select-account,
  .select-extra {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .select-account input,
  .select-extra input {
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .ghost-btn {
    padding: 5px 12px;
    background: transparent;
    border: 1px solid color-mix(in oklab, var(--content-border) 40%, transparent);
    border-radius: 999px;
    color: var(--secondary);
    font-size: 12px;
    cursor: pointer;
    transition: border-color 120ms, color 120ms, background 120ms;
  }
  .ghost-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .ghost-btn.danger:hover {
    border-color: #d33;
    color: #d33;
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    align-self: start;
    margin-top: 6px;
  }
  .status-dot[data-state="fresh"] { background: #1DB954; }
  .status-dot[data-state="aging"] { background: #f4a72b; }
  .status-dot[data-state="stale"] { background: #d33; }
  .status-dot[data-state="empty"] { background: var(--tertiary); opacity: 0.4; }

  .extra-accounts {
    list-style: none;
    margin: 8px 0 0;
    padding: 8px 0 0;
    border-top: 1px dashed color-mix(in oklab, var(--content-border) 30%, transparent);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .extra-account {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 10px;
    align-items: center;
    padding: 4px 0;
    font-size: 12px;
  }
  .extra-account.selectable {
    grid-template-columns: auto 1fr auto auto;
  }
  .extra-alias {
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .extra-count {
    color: var(--tertiary);
    font-size: 11px;
  }
  .extra-actions {
    display: flex;
    gap: 4px;
  }
  .mini-btn {
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    border: 1px solid color-mix(in oklab, var(--content-border) 30%, transparent);
    border-radius: 6px;
    color: var(--tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: border-color 120ms, color 120ms;
  }
  .mini-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .mini-btn.danger:hover {
    border-color: #d33;
    color: #d33;
  }
  .add-account-btn {
    margin-top: 8px;
    padding: 6px 0;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    align-self: flex-start;
    transition: color 120ms;
  }
  .add-account-btn:hover {
    color: var(--accent);
  }
</style>
