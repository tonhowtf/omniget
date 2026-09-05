<script lang="ts" module>
  export type ToolSection = "metadata" | "thumbnails" | "subs" | "cc" | "lc" | "workshop";
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import SubtitleWorkshop from "$components/downloads/SubtitleWorkshop.svelte";

  /**
   * A seção Tools monta uma ferramenta por página; a aba Tools de Downloads
   * continua mostrando todas. Sem `only`, nada muda.
   */
  let { only }: { only?: ToolSection[] } = $props();
  const show = (s: ToolSection) => !only || only.includes(s);

  let wsOpen = $state(false);

  let metaUrl = $state("");
  let metaBusy = $state(false);

  async function resolveOutputDir(): Promise<string | null> {
    const settings = getSettings();
    let dir = settings?.download.default_output_dir ?? "";
    if (!dir) {
      const sel = await openDialog({
        directory: true,
        title: $t("settings.download.default_output_dir") as string,
      });
      if (!sel || typeof sel !== "string") return null;
      dir = sel;
    }
    return dir;
  }

  type Thumb = { url: string; width: number; height: number };
  let thumbUrl = $state("");
  let thumbBusy = $state(false);
  let thumbTitle = $state("");
  let thumbs = $state<Thumb[]>([]);
  let thumbSavingUrl = $state<string | null>(null);

  async function listThumbs() {
    const url = thumbUrl.trim();
    if (!url || thumbBusy) return;
    thumbBusy = true;
    thumbs = [];
    thumbTitle = "";
    try {
      const res = await invoke<{ title: string; thumbnails: Thumb[] }>("thumbnails_list", {
        url,
      });
      thumbTitle = res.title;
      thumbs = res.thumbnails;
      if (thumbs.length === 0) showToast("info", $t("tools.thumbnails_none") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      thumbBusy = false;
    }
  }

  async function saveThumb(thumb: Thumb) {
    if (thumbSavingUrl) return;
    thumbSavingUrl = thumb.url;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      const name =
        thumb.width && thumb.height
          ? `${thumbTitle}-${thumb.width}x${thumb.height}`
          : thumbTitle;
      const saved = await invoke<string>("thumbnail_save", {
        thumbUrl: thumb.url,
        outputDir: dir,
        fileName: name,
      });
      showToast("success", $t("tools.thumbnails_saved", { name: saved }) as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      thumbSavingUrl = null;
    }
  }

  type SubFormat = { ext: string; url: string };
  type SubTrack = { lang: string; name: string; auto: boolean; formats: SubFormat[] };
  let subUrl = $state("");
  let subBusy = $state(false);
  let subTitle = $state("");
  let subTracks = $state<SubTrack[]>([]);
  let subPrimaryIdx = $state(0);
  let subPrimaryExt = $state("");
  let subSecondaryIdx = $state(-1);
  let subSaving = $state(false);

  let subPrimary = $derived(subTracks[subPrimaryIdx]);
  let subPrimaryFmt = $derived(
    subPrimary?.formats.find((f) => f.ext === subPrimaryExt) ?? subPrimary?.formats[0],
  );
  let subSecondary = $derived(
    subSecondaryIdx >= 0 ? subTracks[subSecondaryIdx] : undefined,
  );
  let subMergeable = $derived(
    !!subSecondary &&
      !!subPrimaryFmt &&
      (subPrimaryFmt.ext === "srt" || subPrimaryFmt.ext === "vtt"),
  );

  function trackLabel(tk: SubTrack): string {
    const base = tk.name && tk.name !== tk.lang ? `${tk.name} (${tk.lang})` : tk.lang;
    return tk.auto ? `${base} · auto` : base;
  }

  async function listSubs() {
    const url = subUrl.trim();
    if (!url || subBusy) return;
    subBusy = true;
    subTracks = [];
    subTitle = "";
    subPrimaryIdx = 0;
    subSecondaryIdx = -1;
    try {
      const res = await invoke<{ title: string; tracks: SubTrack[] }>("subtitles_list", {
        url,
      });
      subTitle = res.title;
      subTracks = res.tracks;
      subPrimaryExt = res.tracks[0]?.formats[0]?.ext ?? "";
      if (subTracks.length === 0) showToast("info", $t("tools.subs_none") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      subBusy = false;
    }
  }

  async function saveSub() {
    if (subSaving || !subPrimary || !subPrimaryFmt) return;
    subSaving = true;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      const saved = await invoke<string>("subtitles_save", {
        subUrl: subPrimaryFmt.url,
        ext: subPrimaryFmt.ext,
        outputDir: dir,
        fileName: `${subTitle}.${subPrimary.lang}`,
      });
      showToast("success", $t("tools.subs_saved", { name: saved }) as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      subSaving = false;
    }
  }

  async function mergeSubs() {
    if (subSaving || !subPrimary || !subPrimaryFmt || !subSecondary) return;
    const secFmt =
      subSecondary.formats.find((f) => f.ext === subPrimaryFmt!.ext) ??
      subSecondary.formats[0];
    if (!secFmt) return;
    subSaving = true;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      const saved = await invoke<string>("subtitles_merge", {
        primaryUrl: subPrimaryFmt.url,
        secondaryUrl: secFmt.url,
        format: subPrimaryFmt.ext,
        outputDir: dir,
        fileName: `${subTitle}.${subPrimary.lang}-${subSecondary.lang}`,
      });
      showToast("success", $t("tools.subs_saved", { name: saved }) as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      subSaving = false;
    }
  }

  type Comment = {
    id: string;
    parent: string;
    author: string;
    text: string;
    timestamp: number;
    like_count: number;
    is_uploader: boolean;
  };
  type Chapter = { start: number; end: number; title: string };
  let ccUrl = $state("");
  let ccBusy = $state(false);
  let ccMax = $state(100);
  let ccSort = $state<"top" | "new">("top");
  let ccTitle = $state("");
  let ccMode = $state<"comments" | "chapters" | null>(null);
  let ccComments = $state<Comment[]>([]);
  let ccChapters = $state<Chapter[]>([]);
  let ccFilter = $state("");
  let ccSaving = $state(false);

  let ccFilteredComments = $derived(
    ccFilter.trim()
      ? ccComments.filter((c) =>
          (c.author + " " + c.text).toLowerCase().includes(ccFilter.trim().toLowerCase()),
        )
      : ccComments,
  );
  let ccFilteredChapters = $derived(
    ccFilter.trim()
      ? ccChapters.filter((c) =>
          c.title.toLowerCase().includes(ccFilter.trim().toLowerCase()),
        )
      : ccChapters,
  );

  function fmtTime(s: number): string {
    const t = Math.max(0, Math.floor(s));
    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const sec = t % 60;
    const pad = (n: number) => String(n).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
  }

  function csvCell(v: string | number): string {
    const s = String(v);
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  }

  async function fetchComments() {
    const url = ccUrl.trim();
    if (!url || ccBusy) return;
    ccBusy = true;
    ccMode = null;
    ccComments = [];
    ccChapters = [];
    try {
      const res = await invoke<{ title: string; count: number; comments: Comment[] }>(
        "comments_fetch",
        { url, maxComments: ccMax, sort: ccSort },
      );
      ccTitle = res.title;
      ccComments = res.comments;
      ccMode = "comments";
      if (ccComments.length === 0) showToast("info", $t("tools.cc_none") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      ccBusy = false;
    }
  }

  async function fetchChapters() {
    const url = ccUrl.trim();
    if (!url || ccBusy) return;
    ccBusy = true;
    ccMode = null;
    ccComments = [];
    ccChapters = [];
    try {
      const res = await invoke<{ title: string; chapters: Chapter[] }>("chapters_fetch", {
        url,
      });
      ccTitle = res.title;
      ccChapters = res.chapters;
      ccMode = "chapters";
      if (ccChapters.length === 0) showToast("info", $t("tools.cc_none") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      ccBusy = false;
    }
  }

  async function exportCc(kind: "json" | "csv") {
    if (ccSaving || !ccMode) return;
    ccSaving = true;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      let content: string;
      let ext: string;
      if (ccMode === "comments") {
        if (kind === "json") {
          content = JSON.stringify(ccFilteredComments, null, 2);
          ext = "comments.json";
        } else {
          const rows = ccFilteredComments.map(
            (c) =>
              `${csvCell(c.author)},${csvCell(c.like_count)},${csvCell(c.text)}`,
          );
          content = ["author,likes,text", ...rows].join("\r\n");
          ext = "comments.csv";
        }
      } else if (kind === "json") {
        content = JSON.stringify(ccFilteredChapters, null, 2);
        ext = "chapters.json";
      } else {
        const rows = ccFilteredChapters.map(
          (c) => `${csvCell(fmtTime(c.start))},${csvCell(fmtTime(c.end))},${csvCell(c.title)}`,
        );
        content = ["start,end,title", ...rows].join("\r\n");
        ext = "chapters.csv";
      }
      const saved = await invoke<string>("tools_save_text", {
        outputDir: dir,
        fileName: `${ccTitle}.${ext}`,
        content,
      });
      showToast("success", $t("tools.cc_exported", { name: saved }) as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      ccSaving = false;
    }
  }

  type LiveMsg = {
    idx: number;
    time: string;
    timestamp_usec: number;
    author: string;
    channel_id: string;
    message: string;
    msg_type: string;
    amount: string;
  };
  let lcUrl = $state("");
  let lcBusy = $state(false);
  let lcMsgs = $state<LiveMsg[]>([]);
  let lcFilter = $state("");
  let lcSaving = $state(false);
  let lcFetched = $state(false);

  let lcFiltered = $derived(
    lcFilter.trim()
      ? lcMsgs.filter((m) =>
          (m.author + " " + m.message).toLowerCase().includes(lcFilter.trim().toLowerCase()),
        )
      : lcMsgs,
  );

  async function fetchLiveChat() {
    const url = lcUrl.trim();
    if (!url || lcBusy) return;
    lcBusy = true;
    lcMsgs = [];
    lcFetched = false;
    try {
      const res = await invoke<{ count: number; messages: LiveMsg[] }>("livechat_fetch", {
        url,
      });
      lcMsgs = res.messages;
      lcFetched = true;
      if (lcMsgs.length === 0) showToast("info", $t("tools.lc_none") as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      lcBusy = false;
    }
  }

  async function exportLiveChat(kind: "json" | "csv") {
    if (lcSaving || lcFiltered.length === 0) return;
    lcSaving = true;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      let content: string;
      let ext: string;
      if (kind === "json") {
        content = JSON.stringify(lcFiltered, null, 2);
        ext = "livechat.json";
      } else {
        const rows = lcFiltered.map(
          (m) =>
            `${csvCell(m.time)},${csvCell(m.author)},${csvCell(m.msg_type)},${csvCell(m.amount)},${csvCell(m.message)}`,
        );
        content = ["time,author,type,amount,message", ...rows].join("\r\n");
        ext = "livechat.csv";
      }
      const saved = await invoke<string>("tools_save_text", {
        outputDir: dir,
        fileName: `livechat.${ext}`,
        content,
      });
      showToast("success", $t("tools.lc_exported", { name: saved }) as string);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      lcSaving = false;
    }
  }

  async function fetchMetadata() {
    const url = metaUrl.trim();
    if (!url || metaBusy) return;
    metaBusy = true;
    try {
      const dir = await resolveOutputDir();
      if (!dir) return;
      const res = await invoke<{ title: string; saved: string[] }>("metadata_fetch", {
        url,
        outputDir: dir,
      });
      showToast(
        "success",
        $t("tools.metadata_done", {
          title: res.title,
          count: String(res.saved.length),
        }) as string,
      );
      metaUrl = "";
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      metaBusy = false;
    }
  }
</script>

<div class="tools">
  {#if show("metadata")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("tools.metadata_title")}</h3>
      <p class="tool-desc">{$t("tools.metadata_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <input
        class="tool-input"
        type="text"
        placeholder={$t("tools.url_placeholder") as string}
        bind:value={metaUrl}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") fetchMetadata();
        }}
      />
      <button
        class="tool-btn"
        type="button"
        disabled={metaBusy || !metaUrl.trim()}
        onclick={fetchMetadata}
      >
        {metaBusy ? $t("tools.working") : $t("tools.metadata_action")}
      </button>
    </div>
  </section>
  {/if}

  {#if show("thumbnails")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("tools.thumbnails_title")}</h3>
      <p class="tool-desc">{$t("tools.thumbnails_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <input
        class="tool-input"
        type="text"
        placeholder={$t("tools.url_placeholder") as string}
        bind:value={thumbUrl}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") listThumbs();
        }}
      />
      <button
        class="tool-btn"
        type="button"
        disabled={thumbBusy || !thumbUrl.trim()}
        onclick={listThumbs}
      >
        {thumbBusy ? $t("tools.working") : $t("tools.thumbnails_action")}
      </button>
    </div>
    {#if thumbs.length > 0}
      <div class="thumb-grid">
        {#each thumbs as thumb (thumb.url)}
          <button
            class="thumb-chip"
            type="button"
            disabled={thumbSavingUrl !== null}
            onclick={() => saveThumb(thumb)}
            title={$t("tools.thumbnails_save") as string}
          >
            {thumb.width && thumb.height
              ? `${thumb.width}×${thumb.height}`
              : $t("tools.thumbnails_default")}
          </button>
        {/each}
      </div>
    {/if}
  </section>
  {/if}

  {#if show("subs")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("tools.subs_title")}</h3>
      <p class="tool-desc">{$t("tools.subs_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <input
        class="tool-input"
        type="text"
        placeholder={$t("tools.url_placeholder") as string}
        bind:value={subUrl}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") listSubs();
        }}
      />
      <button
        class="tool-btn"
        type="button"
        disabled={subBusy || !subUrl.trim()}
        onclick={listSubs}
      >
        {subBusy ? $t("tools.working") : $t("tools.subs_action")}
      </button>
    </div>
    {#if subTracks.length > 0}
      <div class="sub-form">
        <label class="sub-field">
          <span>{$t("tools.subs_primary")}</span>
          <select class="tool-select" bind:value={subPrimaryIdx}>
            {#each subTracks as tk, i (i)}
              <option value={i}>{trackLabel(tk)}</option>
            {/each}
          </select>
        </label>
        {#if subPrimary && subPrimary.formats.length > 0}
          <label class="sub-field">
            <span>{$t("tools.subs_format")}</span>
            <select class="tool-select" bind:value={subPrimaryExt}>
              {#each subPrimary.formats as f (f.ext + f.url)}
                <option value={f.ext}>{f.ext || "?"}</option>
              {/each}
            </select>
          </label>
        {/if}
        <label class="sub-field">
          <span>{$t("tools.subs_secondary")}</span>
          <select class="tool-select" bind:value={subSecondaryIdx}>
            <option value={-1}>{$t("tools.subs_none")}</option>
            {#each subTracks as tk, i (i)}
              {#if i !== subPrimaryIdx}
                <option value={i}>{trackLabel(tk)}</option>
              {/if}
            {/each}
          </select>
        </label>
        <div class="sub-actions">
          <button
            class="tool-btn"
            type="button"
            disabled={subSaving || !subPrimaryFmt}
            onclick={saveSub}
          >
            {subSaving ? $t("tools.working") : $t("tools.subs_save")}
          </button>
          <button
            class="tool-btn ghost"
            type="button"
            disabled={subSaving || !subMergeable}
            title={subMergeable ? "" : ($t("tools.subs_merge_hint") as string)}
            onclick={mergeSubs}
          >
            {$t("tools.subs_merge")}
          </button>
        </div>
      </div>
    {/if}
  </section>
  {/if}

  {#if show("cc")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("tools.cc_title")}</h3>
      <p class="tool-desc">{$t("tools.cc_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <input
        class="tool-input"
        type="text"
        placeholder={$t("tools.url_placeholder") as string}
        bind:value={ccUrl}
        spellcheck="false"
      />
      <input
        class="tool-num"
        type="number"
        min="1"
        max="2000"
        bind:value={ccMax}
        aria-label={$t("tools.cc_max") as string}
        title={$t("tools.cc_max") as string}
      />
      <select class="tool-select" bind:value={ccSort} aria-label={$t("tools.cc_sort") as string}>
        <option value="top">{$t("tools.cc_sort_top")}</option>
        <option value="new">{$t("tools.cc_sort_new")}</option>
      </select>
    </div>
    <div class="sub-actions">
      <button
        class="tool-btn"
        type="button"
        disabled={ccBusy || !ccUrl.trim()}
        onclick={fetchComments}
      >
        {ccBusy ? $t("tools.working") : $t("tools.cc_comments")}
      </button>
      <button
        class="tool-btn ghost"
        type="button"
        disabled={ccBusy || !ccUrl.trim()}
        onclick={fetchChapters}
      >
        {$t("tools.cc_chapters")}
      </button>
    </div>
    {#if ccMode}
      <div class="cc-results">
        <div class="cc-toolbar">
          <input
            class="tool-input"
            type="text"
            placeholder={$t("tools.cc_filter") as string}
            bind:value={ccFilter}
            spellcheck="false"
          />
          <button class="tool-btn ghost" type="button" disabled={ccSaving} onclick={() => exportCc("json")}>JSON</button>
          <button class="tool-btn ghost" type="button" disabled={ccSaving} onclick={() => exportCc("csv")}>CSV</button>
        </div>
        {#if ccMode === "comments"}
          <ul class="cc-list">
            {#each ccFilteredComments.slice(0, 200) as c (c.id)}
              <li class="cc-row">
                <span class="cc-author">{c.author}{#if c.is_uploader} ★{/if}</span>
                <span class="cc-likes">▲ {c.like_count}</span>
                <span class="cc-text">{c.text}</span>
              </li>
            {/each}
          </ul>
          <p class="cc-count">{$t("tools.cc_showing", { shown: String(Math.min(ccFilteredComments.length, 200)), total: String(ccFilteredComments.length) })}</p>
        {:else}
          <ul class="cc-list">
            {#each ccFilteredChapters as c (c.start)}
              <li class="cc-row">
                <span class="cc-time">{fmtTime(c.start)}</span>
                <span></span>
                <span class="cc-text">{c.title}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </section>
  {/if}

  {#if show("lc")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("tools.lc_title")}</h3>
      <p class="tool-desc">{$t("tools.lc_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <input
        class="tool-input"
        type="text"
        placeholder={$t("tools.url_placeholder") as string}
        bind:value={lcUrl}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter") fetchLiveChat();
        }}
      />
      <button
        class="tool-btn"
        type="button"
        disabled={lcBusy || !lcUrl.trim()}
        onclick={fetchLiveChat}
      >
        {lcBusy ? $t("tools.working") : $t("tools.lc_action")}
      </button>
    </div>
    {#if lcFetched && lcMsgs.length > 0}
      <div class="cc-results">
        <div class="cc-toolbar">
          <input
            class="tool-input"
            type="text"
            placeholder={$t("tools.cc_filter") as string}
            bind:value={lcFilter}
            spellcheck="false"
          />
          <button class="tool-btn ghost" type="button" disabled={lcSaving} onclick={() => exportLiveChat("json")}>JSON</button>
          <button class="tool-btn ghost" type="button" disabled={lcSaving} onclick={() => exportLiveChat("csv")}>CSV</button>
        </div>
        <ul class="cc-list">
          {#each lcFiltered.slice(0, 300) as m (m.idx)}
            <li class="cc-row">
              <span class="cc-time">{m.time}</span>
              <span class="cc-author">{m.author}{#if m.amount} · {m.amount}{/if}</span>
              <span class="cc-text">{m.message}</span>
            </li>
          {/each}
        </ul>
        <p class="cc-count">{$t("tools.cc_showing", { shown: String(Math.min(lcFiltered.length, 300)), total: String(lcFiltered.length) })}</p>
      </div>
    {/if}
  </section>
  {/if}

  {#if show("workshop")}
  <section class="tool-card">
    {#if !only}
    <div class="tool-head">
      <h3 class="tool-title">{$t("downloads.sw.title")}</h3>
      <p class="tool-desc">{$t("downloads.sw.tool_desc")}</p>
    </div>
    {/if}
    <div class="tool-row">
      <button class="tool-btn" type="button" onclick={() => (wsOpen = true)}>
        {$t("downloads.sw.open")}
      </button>
    </div>
  </section>
  {/if}
</div>

{#if wsOpen}
  <SubtitleWorkshop onClose={() => (wsOpen = false)} />
{/if}

<style>
  .tools {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 4px 0;
  }
  .tool-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 18px;
    background: var(--surface);
    border: none;
    border-radius: var(--radius-md, 12px);
  }
  .tool-head {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .tool-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .tool-desc {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.45;
  }
  .tool-row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tool-input {
    flex: 1;
    min-width: 220px;
    padding: 8px 12px;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius, 8px);
    color: var(--secondary);
    font: inherit;
    font-size: 13px;
    outline: none;
  }
  .tool-input:focus-visible {
    border-color: var(--accent);
  }
  .tool-btn {
    padding: 8px 16px;
    background: var(--accent);
    color: var(--on-accent);
    border: 0;
    border-radius: var(--border-radius, 8px);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .tool-btn:hover:not(:disabled) {
    background: var(--accent-lo, var(--accent));
  }
  .tool-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .thumb-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .thumb-chip {
    padding: 6px 12px;
    background: var(--button);
    border: none;
    border-radius: 999px;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .thumb-chip:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .thumb-chip:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .sub-form {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: flex-end;
  }
  .sub-field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .tool-select {
    padding: 7px 10px;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius, 8px);
    color: var(--secondary);
    font: inherit;
    font-size: 13px;
    min-width: 140px;
  }
  .sub-actions {
    display: flex;
    gap: 8px;
  }
  .tool-btn.ghost {
    background: transparent;
    color: var(--secondary);
    border: none;
  }
  .tool-btn.ghost:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tool-num {
    width: 88px;
    padding: 7px 10px;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius, 8px);
    color: var(--secondary);
    font: inherit;
    font-size: 13px;
  }
  .cc-results {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cc-toolbar {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .cc-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 320px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cc-row {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: 10px;
    align-items: baseline;
    padding: 5px 0;
    border-bottom: none;
    font-size: var(--text-sm);
  }
  .cc-author {
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }
  .cc-likes,
  .cc-time {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .cc-text {
    color: var(--secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .cc-count {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-muted);
  }
</style>
