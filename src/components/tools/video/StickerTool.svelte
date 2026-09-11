<script lang="ts">
  /** Foto ou vídeo vira figurinha de WhatsApp/Telegram, dentro do limite de bytes. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, fmtSecs, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    input: string;
    output: string | null;
    animated: boolean;
    bytes: number;
    max_bytes: number;
    size: number;
    duration_s: number;
    quality: number;
    fps: number;
    tries: number;
    hit_target: boolean;
    ok: boolean;
    error: string | null;
  };
  type Result = { items: Item[]; target: string; pack: string | null; tray: string | null };

  const SOURCE_FILTER = [{ name: "Media", extensions: [...FILTERS.images[0].extensions, ...FILTERS.video[0].extensions] }];

  let files = $state<string[]>([]);
  let target = $state("whatsapp");
  let kind = $state("auto");
  let fit = $state("contain");
  let padding = $state(8);
  let start = $state(0);
  let duration = $state(0);
  let maxKb = $state(0);
  let outDir = $state("");
  let pack = $state(false);
  let packName = $state("");
  let packAuthor = $state("");
  let emoji = $state("😀");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const limit = $derived(
    target === "telegram"
      ? { still: 512, moving: 256, secs: 3, ext: "webm" }
      : { still: 100, moving: 500, secs: 10, ext: "webp" },
  );

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "wa-sticker") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_sticker", {
        opts: {
          inputs: files,
          target,
          kind,
          fit,
          padding,
          start,
          duration,
          max_kb: maxKb,
          output_dir: outDir,
          pack,
          pack_name: packName,
          pack_author: packAuthor,
          emoji,
        },
      });
      const over = result.items.filter((i) => i.ok && !i.hit_target).length;
      const failed = result.items.filter((i) => !i.ok).length;
      if (failed) showToast("error", `${failed} ${$t("tools.common.failed")}`);
      else if (over) showToast("info", `${over} ${$t("tools.sticker.over")}`);
      else showToast("success", `${result.items.length} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{files.length} {$t("tools.common.files")}</div>
          <div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(SOURCE_FILTER))]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sticker.app")}</div>
          <div class="group-row-sub">{$t("tools.sticker.limits", { still: limit.still, moving: limit.moving, secs: limit.secs })}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={target}><option value="whatsapp">WhatsApp</option><option value="telegram">Telegram</option></select>
          <select class="input" bind:value={kind}>
            <option value="auto">{$t("tools.sticker.kind_auto")}</option>
            <option value="static">{$t("tools.sticker.kind_static")}</option>
            <option value="animated">{$t("tools.sticker.kind_animated")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sticker.fit")}</div>
          <div class="group-row-sub">{fit === "cover" ? $t("tools.sticker.fit_cover_hint") : $t("tools.sticker.fit_contain_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={fit}>
            <option value="contain">{$t("tools.sticker.fit_contain")}</option>
            <option value="cover">{$t("tools.sticker.fit_cover")}</option>
          </select>
          <input class="input" type="number" min="0" max="120" step="2" bind:value={padding} style:width="5em" />
          <span class="dim">{$t("tools.sticker.padding")}</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sticker.clip")}</div>
          <div class="group-row-sub">{$t("tools.sticker.clip_hint", { secs: limit.secs })}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="0.5" bind:value={start} style:width="5em" />
          <span class="dim">s</span>
          <input class="input" type="number" min="0" step="0.5" bind:value={duration} style:width="5em" />
          <span class="dim">s</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sticker.max")}</div>
          <div class="group-row-sub">{$t("tools.sticker.max_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="10" bind:value={maxKb} style:width="6em" />
          <span class="dim">KB</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sticker.pack")}</div>
          <div class="group-row-sub">{$t("tools.sticker.pack_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={pack} /></div>
      </div>
      {#if pack}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.sticker.pack_name")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" bind:value={packName} placeholder="OmniGet" />
            <input class="input" bind:value={packAuthor} placeholder={$t("tools.sticker.pack_author")} />
            <input class="input" bind:value={emoji} style:width="4em" />
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.sticker.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        {#each result.items as item (item.input)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{baseName(item.input)}</div>
              {#if item.ok}
                <div class="group-row-sub">
                  <strong>{fmtBytes(item.bytes)}</strong> / {fmtBytes(item.max_bytes)} · {item.size}×{item.size}
                  {#if item.animated}<span class="dim"> · {fmtSecs(item.duration_s)} · {item.fps} fps</span>{/if}
                  <span class="dim"> · q{item.quality} · {$t("tools.sticker.tries", { n: item.tries })}</span>
                  {#if !item.hit_target}<span class="tag tag-danger">{$t("tools.sticker.over")}</span>{/if}
                </div>
              {:else}
                <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>
              {/if}
            </div>
            {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
          </div>
        {/each}
        {#if result.pack}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">pack.json</div>
              <div class="group-row-sub mono">{result.pack}</div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.pack!)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
