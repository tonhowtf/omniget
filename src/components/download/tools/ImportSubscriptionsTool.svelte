<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { parseSubscriptions, type SubscriptionImport } from "$lib/download-tools/subscription-import";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Channel = { id: string; url: string; title: string };

  let text = $state("");
  let fileName = $state<string | null>(null);
  let parsed = $state<SubscriptionImport | null>(null);
  let invalid = $state(false);
  let followed = $state<Set<string>>(new Set());
  let picked = $state<Set<string>>(new Set());
  let autoDownload = $state(false);
  let importing = $state(false);
  let done = $state<{ added: number; failed: number } | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  const fresh = $derived(parsed ? parsed.channels.filter((c) => !followed.has(c.url)) : []);
  const chosen = $derived(fresh.filter((c) => picked.has(c.url)));

  async function loadFollowed() {
    try {
      const list = await invoke<Channel[]>("channels_list");
      followed = new Set(list.map((c) => c.url));
    } catch {
      followed = new Set();
    }
  }

  onMount(loadFollowed);

  function read(content: string) {
    done = null;
    parsed = parseSubscriptions(content);
    invalid = !parsed && content.trim().length > 0;
    picked = new Set(parsed ? parsed.channels.filter((c) => !followed.has(c.url)).map((c) => c.url) : []);
  }

  async function onFile(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    fileName = file.name;
    text = "";
    try {
      read(await file.text());
    } catch {
      parsed = null;
      invalid = true;
    }
  }

  function toggle(url: string) {
    const next = new Set(picked);
    if (next.has(url)) next.delete(url);
    else next.add(url);
    picked = next;
  }

  function toggleAll() {
    picked = chosen.length === fresh.length ? new Set() : new Set(fresh.map((c) => c.url));
  }

  async function follow() {
    if (importing || chosen.length === 0) return;
    importing = true;
    let added = 0;
    let failed = 0;
    for (const c of chosen) {
      try {
        const ch = await invoke<Channel>("channel_add", { url: c.url, title: c.title });
        if (autoDownload) await invoke("channel_update", { id: ch.id, autoDownload: true });
        added++;
      } catch {
        failed++;
      }
    }
    importing = false;
    done = { added, failed };
    await loadFollowed();
    if (failed) showToast("error", $t("downloads.tools.import_subscriptions.some_failed", { count: failed }));
  }
</script>

<div class="imp" data-agent-readiness={importing ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.import_subscriptions.lead")}</p>

  <div class="source">
    <input
      bind:this={fileInput}
      class="hidden"
      type="file"
      accept=".db,.json,.csv,.opml,.xml,.txt"
      onchange={onFile}
    />
    <button class="button" onclick={() => fileInput?.click()}>{$t("downloads.tools.import_subscriptions.pick_file")}</button>
    {#if fileName}<span class="dim">{fileName}</span>{/if}
  </div>
  <textarea
    rows="4"
    bind:value={text}
    placeholder={$t("downloads.tools.import_subscriptions.paste_placeholder")}
    aria-label={$t("downloads.tools.import_subscriptions.paste_placeholder")}
    spellcheck="false"
    oninput={() => {
      fileName = null;
      read(text);
    }}
  ></textarea>
  <p class="dim small">{$t("downloads.tools.import_subscriptions.formats")}</p>

  {#if invalid}
    <p class="error" role="alert">{$t("downloads.tools.import_subscriptions.invalid")}</p>
  {/if}

  {#if parsed}
    <div class="head">
      <span>
        {$t("downloads.tools.import_subscriptions.found", { count: parsed.channels.length })}
        <span class="dim">· {$t(`downloads.tools.import_subscriptions.format_${parsed.format.replace("-", "_")}`)}</span>
      </span>
      {#if fresh.length > 0}
        <button class="link" onclick={toggleAll}>
          {chosen.length === fresh.length
            ? $t("downloads.tools.import_subscriptions.select_none")
            : $t("downloads.tools.import_subscriptions.select_all")}
        </button>
      {/if}
    </div>
    {#if parsed.skipped > 0}
      <p class="dim small">{$t("downloads.tools.import_subscriptions.skipped", { count: parsed.skipped })}</p>
    {/if}
    <ul class="list">
      {#each parsed.channels as c (c.url)}
        {@const already = followed.has(c.url)}
        <li>
          <label class:already>
            <input type="checkbox" checked={already || picked.has(c.url)} disabled={already || importing} onchange={() => toggle(c.url)} />
            <span class="name">{c.title}</span>
            {#if already}
              <span class="tag">{$t("downloads.tools.import_subscriptions.already")}</span>
            {:else}
              <span class="dim small url">{c.url}</span>
            {/if}
          </label>
        </li>
      {/each}
    </ul>
    <label class="auto">
      <input type="checkbox" bind:checked={autoDownload} disabled={importing} />
      <span>
        {$t("downloads.tools.import_subscriptions.auto_download")}
        <span class="dim small">{$t("downloads.tools.import_subscriptions.auto_download_hint")}</span>
      </span>
    </label>
    <div class="actions">
      <button class="button" onclick={follow} disabled={importing || chosen.length === 0}>
        {importing
          ? $t("downloads.tools.import_subscriptions.importing")
          : $t("downloads.tools.import_subscriptions.follow", { count: chosen.length })}
      </button>
      {#if done}
        <span class="ok" role="status">{$t("downloads.tools.import_subscriptions.done", { count: done.added })}</span>
        <a class="link" href="/settings?tab=channels">{$t("downloads.tools.import_subscriptions.open_channels")}</a>
      {/if}
    </div>
  {/if}
</div>

<style>
  .imp {
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
  .hidden {
    display: none;
  }
  .source,
  .head,
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .head {
    justify-content: space-between;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-family: var(--font-mono, monospace);
    font-size: var(--text-sm);
    resize: vertical;
  }
  textarea:focus-visible {
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
    max-height: 320px;
    overflow-y: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .list label {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    cursor: pointer;
    min-width: 0;
  }
  .list li + li {
    border-top: var(--hairline) solid var(--content-border);
  }
  .list label.already {
    cursor: default;
    opacity: 0.7;
  }
  .name {
    font-size: var(--text-sm);
    white-space: nowrap;
  }
  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .tag {
    font-size: var(--text-xs, 11px);
    color: var(--accent);
  }
  .auto {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .auto .dim {
    display: block;
  }
  .ok {
    color: var(--success, var(--accent));
    font-size: var(--text-sm);
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-sm);
    text-decoration: none;
  }
</style>
