<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import OutputDir from "./OutputDir.svelte";
  import { tg, errText, isUnknownCommand, splitLinks, formatBytes, type ResolvedLink } from "./api";

  let { outputDir = $bindable("") }: { outputDir: string } = $props();

  let text = $state("");
  let resolved = $state<ResolvedLink[]>([]);
  let busy = $state<"resolve" | "download" | null>(null);
  let error = $state<string | null>(null);

  let links = $derived(splitLinks(text));
  let okCount = $derived(resolved.filter((r) => r.ok !== false && !r.error).length);

  async function resolve() {
    busy = "resolve";
    error = null;
    resolved = [];
    try {
      const r = await tg.resolveLinks(text);
      resolved = r;
    } catch (e) {
      error = isUnknownCommand(e) ? ($t("telegram.common.not_ready") as string) : errText(e);
    } finally {
      busy = null;
    }
  }

  async function download() {
    if (!outputDir) {
      showToast("info", $t("telegram.common.choose_folder_first") as string);
      return;
    }
    busy = "download";
    error = null;
    try {
      await tg.downloadLinks(text, outputDir);
      showToast("success", $t("telegram.links.started", { count: links.length }) as string);
    } catch (e) {
      error = isUnknownCommand(e) ? ($t("telegram.common.not_ready") as string) : errText(e);
    } finally {
      busy = null;
    }
  }
</script>

<section class="links">
  <header>
    <h3>{$t("telegram.links.title")}</h3>
    <p>{$t("telegram.links.hint")}</p>
  </header>
  <textarea class="input mono" rows="6" bind:value={text} spellcheck="false" placeholder="https://t.me/channel/123&#10;https://t.me/c/1234567890/45"></textarea>
  <div class="row">
    <span class="count">{$t("telegram.links.found", { count: links.length })}</span>
    <OutputDir bind:value={outputDir} />
    <button class="btn btn-secondary" onclick={resolve} disabled={!links.length || !!busy}>
      {#if busy === "resolve"}<span class="spinner"></span>{:else}<TgIcon name="search" />{/if}
      {$t("telegram.links.check")}
    </button>
    <button class="btn btn-primary" onclick={download} disabled={!links.length || !!busy}>
      {#if busy === "download"}<span class="spinner"></span>{:else}<TgIcon name="download" />{/if}
      {$t("telegram.links.download")}
    </button>
  </div>
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if resolved.length}
    <span class="group-label">{$t("telegram.links.resolved", { ok: okCount, total: resolved.length })}</span>
    <div class="group">
      {#each resolved as r, i (i)}
        <div class="group-row">
          <span class="mark" class:bad={r.ok === false || !!r.error}><TgIcon name={r.ok === false || r.error ? "x" : "check"} size={12} /></span>
          <div class="group-row-content">
            <span class="group-row-title">{r.file_name || r.chat_title || r.link}</span>
            <span class="group-row-sub">
              {#if r.error}{r.error}{:else}{[r.chat_title, r.media_type, r.file_size ? formatBytes(r.file_size) : ""].filter(Boolean).join(" · ")}{/if}
            </span>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .links {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 760px;
  }

  header h3 {
    margin: 0 0 2px;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  header p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .count {
    flex: 1;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .mark {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }

  .mark.bad {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--danger);
  }
</style>
