<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Props = {
    open: boolean;
    domain: string;
    slug: string;
    content: string;
    path: string;
    onClose: () => void;
  };

  let { open, domain, slug, content, path, onClose }: Props = $props();

  let filter = $state("");
  let nowSec = $derived(Math.floor(Date.now() / 1000));

  type ParsedLine = {
    raw: string;
    domain: string;
    name: string;
    value: string;
    expires: number;
    http_only: boolean;
    expired: boolean;
  };

  let parsed = $derived.by<ParsedLine[]>(() => {
    const out: ParsedLine[] = [];
    for (const raw of content.split("\n")) {
      const trimmed = raw.trim();
      if (!trimmed) continue;
      let effective = trimmed;
      let http_only = false;
      if (trimmed.startsWith("#HttpOnly_")) {
        effective = trimmed.slice("#HttpOnly_".length);
        http_only = true;
      } else if (trimmed.startsWith("#")) {
        continue;
      }
      const parts = effective.split("\t");
      if (parts.length < 7) continue;
      const expires = parseInt(parts[4] ?? "0", 10) || 0;
      out.push({
        raw,
        domain: parts[0] ?? "",
        name: parts[5] ?? "",
        value: parts[6] ?? "",
        expires,
        http_only,
        expired: expires > 0 && expires < nowSec,
      });
    }
    return out;
  });

  let filtered = $derived(
    !filter.trim()
      ? parsed
      : parsed.filter((p) => p.name.toLowerCase().includes(filter.toLowerCase().trim())),
  );

  let expiredCount = $derived(parsed.filter((p) => p.expired).length);

  async function copyAll() {
    try {
      await navigator.clipboard.writeText(content);
      showToast("success", $t("settings.cookies.modal_copied") as string);
    } catch {
      showToast("error", "clipboard");
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    onclick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onkeydown={(e) => { if (e.key === "Escape") onClose(); }}
    role="presentation"
  >
    <div class="modal" role="dialog" aria-label={$t("settings.cookies.modal_title") as string}>
      <header class="modal-head">
        <h2>{$t("settings.cookies.modal_title")}</h2>
        <p class="subtitle">
          <span class="domain">{domain}</span>
          {#if slug !== "_default"}<span class="slug">/{slug}</span>{/if}
        </p>
        <button class="close" type="button" onclick={onClose} aria-label={$t("settings.cookies.modal_close") as string}>×</button>
      </header>

      <div class="toolbar">
        <input
          type="search"
          class="filter"
          placeholder={$t("settings.cookies.modal_filter_placeholder") as string}
          bind:value={filter}
        />
        <button type="button" class="ghost-btn" onclick={copyAll}>
          {$t("settings.cookies.modal_copy_all")}
        </button>
      </div>

      <div class="list">
        {#if filtered.length === 0}
          <p class="empty">{$t("settings.cookies.modal_no_results")}</p>
        {:else}
          <ul>
            {#each filtered as row, i (row.name + i)}
              <li class:expired={row.expired}>
                <span class="row-name">{row.name}</span>
                <span class="row-domain">{row.domain}</span>
                <span class="row-value">{row.value}</span>
                {#if row.expired}
                  <span class="expired-tag">{$t("settings.cookies.modal_expired_tag")}</span>
                {/if}
                {#if row.http_only}
                  <span class="httponly-tag">httpOnly</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      <footer class="modal-foot">
        <span class="stat">{$t("settings.cookies.modal_total", { count: String(parsed.length) })}</span>
        {#if expiredCount > 0}
          <span class="stat warn">{$t("settings.cookies.modal_expired_count", { count: String(expiredCount) })}</span>
        {/if}
        <span class="path" title={path}>{path}</span>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 24px;
  }
  .modal {
    width: min(720px, 100%);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    background: var(--surface, var(--bg));
    border: none;
    border-radius: 16px;
    overflow: hidden;
  }
  .modal-head {
    position: relative;
    padding: 20px 56px 16px 20px;
    border-bottom: none;
  }
  .modal-head h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--secondary);
  }
  .subtitle {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--tertiary);
  }
  .domain {
    font-family: ui-monospace, monospace;
  }
  .slug {
    color: var(--secondary);
  }
  .close {
    position: absolute;
    top: 14px;
    right: 14px;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 50%;
    color: var(--tertiary);
    font-size: 18px;
    cursor: pointer;
  }
  .close:hover {
    color: var(--secondary);
    background: color-mix(in oklab, var(--button) 40%, transparent);
  }
  .toolbar {
    display: flex;
    gap: 10px;
    padding: 12px 20px;
    border-bottom: none;
  }
  .filter {
    flex: 1;
    padding: 6px 12px;
    background: color-mix(in oklab, var(--button) 50%, transparent);
    border: none;
    border-radius: 999px;
    color: var(--secondary);
    font-size: 13px;
    outline: none;
  }
  .filter:focus { border-color: var(--accent); }
  .ghost-btn {
    padding: 6px 14px;
    background: transparent;
    border: none;
    border-radius: 999px;
    color: var(--secondary);
    font-size: 12px;
    cursor: pointer;
  }
  .ghost-btn:hover { border-color: var(--accent); color: var(--accent); }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 8px 20px;
  }
  .list ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .list li {
    display: grid;
    grid-template-columns: 1.2fr 1.5fr 2fr auto auto;
    gap: 10px;
    align-items: center;
    padding: 6px 8px;
    font-size: 12px;
    font-family: ui-monospace, "Cascadia Code", monospace;
    border-radius: 6px;
  }
  .list li:hover {
    background: color-mix(in oklab, var(--button) 30%, transparent);
  }
  .list li.expired {
    color: var(--tertiary);
    text-decoration: line-through;
  }
  .row-name { font-weight: 600; color: var(--secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-domain { color: var(--tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-value { color: var(--secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .expired-tag {
    font-size: 10px;
    padding: 2px 6px;
    background: color-mix(in oklab, var(--danger) 20%, transparent);
    color: var(--danger);
    border-radius: 4px;
    text-decoration: none;
  }
  .httponly-tag {
    font-size: 10px;
    padding: 2px 6px;
    background: color-mix(in oklab, var(--accent) 15%, transparent);
    color: var(--accent);
    border-radius: 4px;
  }
  .empty {
    text-align: center;
    padding: 30px;
    color: var(--tertiary);
  }
  .modal-foot {
    padding: 12px 20px;
    border-top: none;
    display: flex;
    align-items: center;
    gap: 16px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .stat.warn { color: var(--warning); }
  .path {
    margin-left: auto;
    font-family: ui-monospace, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 50%;
  }
</style>
