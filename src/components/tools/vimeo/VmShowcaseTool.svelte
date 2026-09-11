<script lang="ts">
  /**
   * Backup de Showcase / Álbum / Canal do Vimeo: enumera a coleção, deixa
   * escolher o que entra e baixa item a item, com resumo por item e retomada
   * (o que já está na pasta, íntegro, é pulado).
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Entry = { id: string; title: string; url: string; duration: number | null; uploader: string | null };
  type Listing = { kind: string; id: string; title: string; entries: Entry[]; used_session: boolean; command: string };
  type Item = { id: string; title: string; status: string; file: string | null; reason: string | null };
  type Backup = {
    kind: string;
    title: string;
    dest: string;
    total: number;
    ok: number;
    skipped: number;
    failed: number;
    items: Item[];
    used_session: boolean;
    command: string;
  };

  let url = $state("");
  let dest = $state("");
  let showcasePassword = $state("");
  let videoPassword = $state("");
  let audioOnly = $state(false);
  let writeInfo = $state(false);
  let skipExisting = $state(true);
  let busy = $state<string | null>(null);
  let listing = $state<Listing | null>(null);
  let chosen = $state<Set<string>>(new Set());
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Backup | null>(null);
  let unlisten: (() => void) | null = null;

  const CODES = [
    "bad_url",
    "not_a_collection",
    "empty",
    "password_required",
    "wrong_password",
    "private",
    "not_found",
    "geo",
    "unavailable",
  ];

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "vm-showcase") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function niceError(e: unknown): string {
    const raw = errText(e).trim();
    const code = raw.startsWith("vimeo:") ? raw.slice(6) : "";
    if (code && CODES.includes(code)) {
      const text = $t(`tools.vimeo.err_${code}`) as string;
      if (text) return text;
    }
    return raw;
  }

  function reason(item: Item): string {
    if (!item.reason) return "";
    if (CODES.includes(item.reason)) {
      const text = $t(`tools.vimeo.err_${item.reason}`) as string;
      if (text) return text;
    }
    return item.reason;
  }

  function toggle(id: string) {
    const next = new Set(chosen);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    chosen = next;
  }

  async function list() {
    if (!url.trim() || busy) return;
    busy = "list";
    listing = null;
    result = null;
    progress = null;
    try {
      listing = await invoke<Listing>("tool_vm_list", {
        opts: { url: url.trim(), showcase_password: showcasePassword.trim() || null },
      });
      chosen = new Set(listing.entries.map((e) => e.id));
      showToast("success", `${listing.entries.length} ${$t("tools.vimeo.videos")}`);
    } catch (e) {
      showToast("error", niceError(e));
    } finally {
      busy = null;
    }
  }

  async function run() {
    if (!url.trim() || busy) return;
    let out = dest;
    if (!out) {
      const d = await pickDir();
      if (!d) return;
      out = d;
      dest = d;
    }
    busy = "run";
    result = null;
    progress = null;
    const all = listing ? listing.entries.length : 0;
    try {
      result = await invoke<Backup>("tool_vm_showcase", {
        opts: {
          url: url.trim(),
          dest: out,
          showcase_password: showcasePassword.trim() || null,
          video_password: videoPassword.trim() || null,
          audio_only: audioOnly,
          skip_existing: skipExisting,
          write_info: writeInfo,
          only_ids: all && chosen.size < all ? [...chosen] : [],
        },
      });
      showToast(result.failed ? "info" : "success", `${result.ok} ${$t("tools.vimeo.ok")} · ${result.skipped} ${$t("tools.vimeo.skipped")} · ${result.failed} ${$t("tools.vimeo.failed")}`);
    } catch (e) {
      showToast("error", niceError(e));
    } finally {
      busy = null;
    }
  }

  let stageText = $derived.by(() => {
    if (!progress) return "";
    const n = progress.total ? `${progress.done}/${progress.total}` : String(progress.done);
    return `${n}${progress.message ? ` · ${progress.message}` : ""}`;
  });
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <input class="input" type="text" bind:value={url} placeholder={$t("tools.vimeo.collection_placeholder")} onkeydown={(e) => e.key === "Enter" && list()} />
          <div class="group-row-sub">{$t("tools.vimeo.collection_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary" type="button" disabled={busy !== null || !url.trim()} onclick={list}>{busy === "list" ? $t("tools.common.working") : $t("tools.vimeo.list")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.vimeo.showcase_password")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub">{$t("tools.vimeo.showcase_password_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="password" bind:value={showcasePassword} style:width="12em" autocomplete="off" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.vimeo.video_password")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub">{$t("tools.vimeo.video_password_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="password" bind:value={videoPassword} style:width="12em" autocomplete="off" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if dest}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dest = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if listing}
    <section>
      <span class="group-label">{listing.title || listing.id}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{listing.entries.length} {$t("tools.vimeo.videos")} · {chosen.size} {$t("tools.vimeo.selected")}</div>
            <div class="group-row-sub"><span class="tag">{$t(`tools.vimeo.kind_${listing.kind}`)}</span> <span class="tag">{listing.used_session ? $t("tools.vimeo.session_on") : $t("tools.vimeo.session_off")}</span></div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (chosen = new Set())}>{$t("tools.vimeo.select_none")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => (chosen = new Set(listing!.entries.map((e) => e.id)))}>{$t("tools.vimeo.select_all")}</button>
          </div>
        </div>
        {#each listing.entries.slice(0, 300) as e (e.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{e.title}</div>
              <div class="group-row-sub mono">{e.id}{#if e.duration} · {fmtSecs(e.duration)}{/if}{#if e.uploader} · {e.uploader}{/if}</div>
            </div>
            <div class="group-row-trailing"><input class="checkbox" type="checkbox" checked={chosen.has(e.id)} onchange={() => toggle(e.id)} /></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  <section>
    <span class="group-label">{$t("tools.vimeo.options")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vimeo.audio_only")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={audioOnly} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vimeo.write_info")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={writeInfo} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.vimeo.skip_existing")}</div>
          <div class="group-row-sub">{$t("tools.vimeo.skip_existing_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={skipExisting} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy === "run"}
            <div class="group-row-sub mono">{stageText}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.vimeo.backup")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{result.title}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.ok} {$t("tools.vimeo.ok")} · {result.skipped} {$t("tools.vimeo.skipped")} · {result.failed} {$t("tools.vimeo.failed")}</div>
            <div class="group-row-sub mono">{result.dest}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.items as it (it.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{it.title}</div>
              <div class="group-row-sub">
                {#if it.status === "ok"}<span class="tag tag-success">{$t("tools.vimeo.status_ok")}</span>{:else if it.status === "skipped"}<span class="tag">{$t("tools.vimeo.status_skipped")}</span>{:else}<span class="tag tag-warning">{$t("tools.vimeo.status_failed")}</span>{/if}
                {#if it.reason}<span class="dim">{reason(it)}</span>{/if}
              </div>
            </div>
            <div class="group-row-trailing">{#if it.file}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(it.file!)}>↗</button>{/if}</div>
          </div>
        {/each}
        {#if result.command}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.vimeo.command")}</div>
              <div class="group-row-sub mono">{result.command}</div>
            </div>
          </div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .dim { color: var(--text-dim); font-weight: 400; }
</style>
