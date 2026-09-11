<script lang="ts">
  /**
   * As tools de PDF que escrevem estrutura (lopdf): senha, marca d'água,
   * corte de margem e sumário. `mode` decide qual formulário aparece.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFile, pickFiles, reveal, type ToolProgress } from "$lib/tools/rt";

  let { mode = "password" }: { mode?: "password" | "watermark" | "crop" | "outline" } = $props();

  type Item = { input: string; output: string | null; pages: number; note: string; ok: boolean; error: string | null };
  type Result = { items: Item[] };
  type Entry = { title: string; page: number; level: number };

  const PDF = [{ name: "PDF", extensions: ["pdf"] }];

  let files = $state<string[]>([]);
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  // senha
  let pwMode = $state("add");
  let userPw = $state("");
  let ownerPw = $state("");
  let currentPw = $state("");
  let allowPrint = $state(true);
  let allowCopy = $state(true);
  let allowModify = $state(false);
  let allowAnnotate = $state(true);
  // marca
  let wmMode = $state("diagonal");
  let wmText = $state("CONFIDENCIAL");
  let fontSize = $state(48);
  let opacity = $state(15);
  let gray = $state(50);
  let prefix = $state("");
  let startNumber = $state(1);
  // corte
  let cropMode = $state("auto");
  let padding = $state(6);
  let left = $state(40);
  let right = $state(40);
  let top = $state(40);
  let bottom = $state(40);
  let uniform = $state(true);
  // sumário
  let entries = $state<Entry[]>([]);

  const PROGRESS_ID: Record<string, string> = { password: "pdf-password", watermark: "pdf-watermark", crop: "pdf-crop" };

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === PROGRESS_ID[mode]) progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      if (mode === "password") {
        result = await invoke<Result>("tool_pdf_password", {
          opts: { inputs: files, mode: pwMode, user_password: userPw, owner_password: ownerPw, current_password: currentPw, allow_print: allowPrint, allow_copy: allowCopy, allow_modify: allowModify, allow_annotate: allowAnnotate, output_dir: outDir, suffix: "" },
        });
      } else if (mode === "watermark") {
        result = await invoke<Result>("tool_pdf_watermark", {
          opts: { inputs: files, mode: wmMode, text: wmText, font_size: fontSize, opacity, gray, start_number: startNumber, prefix, output_dir: outDir, suffix: "" },
        });
      } else if (mode === "crop") {
        result = await invoke<Result>("tool_pdf_crop", {
          opts: { inputs: files, mode: cropMode, padding, left, right, top, bottom, uniform, output_dir: outDir, suffix: "" },
        });
      } else {
        const item = await invoke<Item>("tool_pdf_outline_write", {
          opts: { input: files[0], entries, output_dir: outDir, suffix: "" },
        });
        result = { items: [item] };
      }
      const failed = result.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${result.items.length - failed} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function readOutline() {
    if (!files.length) return;
    try {
      entries = await invoke<Entry[]>("tool_pdf_outline_read", { input: files[0] });
      if (!entries.length) showToast("info", $t("tools.pdfwrite.outline_empty") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">
          {#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => { files = []; entries = []; }}>×</button>{/if}
          {#if mode === "outline"}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(PDF); if (f) { files = [f]; entries = []; } }}>{$t("tools.common.choose")}</button>
          {:else}
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(PDF))]; }}>{$t("tools.common.add")}</button>
          {/if}
        </div>
      </div>

      {#if mode === "password"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.result")}</div><div class="group-row-sub">{$t("tools.pdfwrite.accessibility")}</div></div>
          <div class="group-row-trailing"><select class="input" bind:value={pwMode}><option value="add">{$t("tools.pdfwrite.mode_add")}</option><option value="remove">{$t("tools.pdfwrite.mode_remove")}</option></select></div>
        </div>
        {#if pwMode === "add"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.user_pw")}</div><div class="group-row-sub">{$t("tools.pdfwrite.no_open_pw")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="password" bind:value={userPw} style:width="9em" />
              <input class="input" type="password" bind:value={ownerPw} placeholder={$t("tools.pdfwrite.owner_pw")} style:width="9em" />
            </div>
          </div>
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.permissions")}</div></div>
            <div class="group-row-trailing btn-row wrap">
              {#each [["print", allowPrint], ["copy", allowCopy], ["modify", allowModify], ["annotate", allowAnnotate]] as [k] (k)}
                <button class="btn btn-sm" class:btn-primary={k === "print" ? allowPrint : k === "copy" ? allowCopy : k === "modify" ? allowModify : allowAnnotate} class:btn-secondary={!(k === "print" ? allowPrint : k === "copy" ? allowCopy : k === "modify" ? allowModify : allowAnnotate)} type="button"
                  onclick={() => { if (k === "print") allowPrint = !allowPrint; else if (k === "copy") allowCopy = !allowCopy; else if (k === "modify") allowModify = !allowModify; else allowAnnotate = !allowAnnotate; }}>{$t(`tools.pdfwrite.${k}`)}</button>
              {/each}
            </div>
          </div>
        {:else}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.current_pw")}</div></div>
            <div class="group-row-trailing"><input class="input" type="password" bind:value={currentPw} style:width="10em" /></div>
          </div>
        {/if}
      {/if}

      {#if mode === "watermark"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.style")}</div></div>
          <div class="group-row-trailing"><select class="input" bind:value={wmMode}>
            <option value="diagonal">{$t("tools.pdfwrite.diagonal")}</option>
            <option value="header">{$t("tools.pdfwrite.header")}</option>
            <option value="footer">{$t("tools.pdfwrite.footer")}</option>
            <option value="bates">{$t("tools.pdfwrite.bates")}</option>
          </select></div>
        </div>
        {#if wmMode === "bates"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.prefix")} <span class="dim">· {$t("tools.pdfwrite.start")}</span></div></div>
            <div class="group-row-trailing btn-row">
              <input class="input mono" type="text" bind:value={prefix} placeholder="OG-" style:width="7em" />
              <input class="input" type="number" min="0" bind:value={startNumber} style:width="7em" />
            </div>
          </div>
        {:else}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.text")}</div></div>
            <div class="group-row-trailing"><input class="input" type="text" bind:value={wmText} style:width="14em" /></div>
          </div>
        {/if}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.size")} {fontSize} <span class="dim">· {$t("tools.pdfwrite.opacity")} {opacity}% · {$t("tools.pdfwrite.gray")} {gray}%</span></div></div>
          <div class="group-row-trailing btn-row">
            <input type="range" min="8" max="96" step="2" bind:value={fontSize} />
            <input type="range" min="5" max="100" step="5" bind:value={opacity} />
            <input type="range" min="0" max="100" step="5" bind:value={gray} />
          </div>
        </div>
      {/if}

      {#if mode === "crop"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.crop_mode")}</div><div class="group-row-sub">{cropMode === "auto" ? $t("tools.pdfwrite.auto_note") : ""}</div></div>
          <div class="group-row-trailing"><select class="input" bind:value={cropMode}><option value="auto">{$t("tools.pdfwrite.auto")}</option><option value="fixed">{$t("tools.pdfwrite.fixed")}</option></select></div>
        </div>
        {#if cropMode === "auto"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.padding")} {padding} pt</div></div>
            <div class="group-row-trailing"><input type="range" min="0" max="40" step="2" bind:value={padding} /></div>
          </div>
        {:else}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.left")} / {$t("tools.pdfwrite.right")} / {$t("tools.pdfwrite.top")} / {$t("tools.pdfwrite.bottom")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="number" min="0" bind:value={left} style:width="5em" />
              <input class="input" type="number" min="0" bind:value={right} style:width="5em" />
              <input class="input" type="number" min="0" bind:value={top} style:width="5em" />
              <input class="input" type="number" min="0" bind:value={bottom} style:width="5em" />
            </div>
          </div>
        {/if}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.pdfwrite.uniform")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={uniform} type="button" role="switch" aria-checked={uniform} aria-label={$t("tools.pdfwrite.uniform")} onclick={() => (uniform = !uniform)}><span class="toggle-knob"></span></button></div>
        </div>
      {/if}

      {#if mode === "outline"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{entries.length} {$t("tools.pdfwrite.entries")}</div></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" disabled={!files.length} onclick={readOutline}>{$t("tools.pdfwrite.read")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => (entries = [...entries, { title: "", page: 1, level: 0 }])}>{$t("tools.pdfwrite.add_entry")}</button>
          </div>
        </div>
        {#each entries as e, i (i)}
          <div class="group-row">
            <div class="group-row-content"><input class="input full" type="text" bind:value={e.title} placeholder={$t("tools.pdfwrite.title")} style:margin-left="{e.level * 1.5}em" /></div>
            <div class="group-row-trailing btn-row">
              <input class="input" type="number" min="1" bind:value={e.page} style:width="5em" />
              <input class="input" type="number" min="0" max="4" bind:value={e.level} style:width="4em" />
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => (entries = entries.filter((_, j) => j !== i))}>×</button>
            </div>
          </div>
        {/each}
      {/if}

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length || (mode === "outline" && !entries.length)} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.pdfwrite.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}<div class="group-row-sub">{item.note} · {item.pages} {$t("tools.pdfwrite.pages")}</div>
            {:else}<div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>{/if}
          </div>
          {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .full { width: 100%; }
  .wrap { flex-wrap: wrap; }
</style>
