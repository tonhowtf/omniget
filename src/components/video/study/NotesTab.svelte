<script lang="ts">
  // Timestamped notes for the current video: N opens a draft 3 s back (or the
  // A-B range), optional frame capture, inline edit, Markdown export.
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { fmtTime } from "$lib/video/format";
  import { deleteNote, player, saveNote, seek, startNote, updateNote } from "$lib/video/player.svelte";
  import VIcon from "$components/video/player/VIcon.svelte";

  let editing = $state<number | null>(null);
  let editText = $state("");
  let saving = $state(false);
  let draftEl: HTMLTextAreaElement | undefined = $state();

  $effect(() => {
    if (player.noteDraft && draftEl) draftEl.focus();
  });

  async function save() {
    if (saving) return;
    saving = true;
    try {
      await saveNote();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }

  async function commitEdit(id: number) {
    const text = editText.trim();
    editing = null;
    if (!text) return;
    try {
      await updateNote(id, text);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function remove(id: number) {
    try {
      await deleteNote(id);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function exportMd() {
    const key = player.info?.key;
    if (!key) return;
    try {
      const path = await api.notesExport({ key });
      showToast("success", $t("video.notes.exported", { path }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function draftKeys(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape") player.noteDraft = null;
  }
</script>

<div class="notes">
  <div class="tools">
    {#if !player.noteDraft}
      <button class="btn btn-sm btn-primary" onclick={startNote}><VIcon name="plus" size={13} /> {$t("video.notes.add_at", { time: fmtTime(Math.max(0, player.position - 3) * 1000) })}</button>
    {/if}
    <span class="sp"></span>
    <button class="btn btn-sm" onclick={exportMd} disabled={!player.notes.length}><VIcon name="export" size={13} /> {$t("video.notes.export")}</button>
  </div>

  {#if player.noteDraft}
    {@const d = player.noteDraft}
    <div class="draft">
      <div class="dt">
        <span class="ts">{fmtTime(d.t_ms)}{d.t_end_ms != null ? `–${fmtTime(d.t_end_ms)}` : ""}</span>
        {#if d.t_end_ms != null}<span class="tag">{$t("video.notes.range")}</span>{/if}
      </div>
      <textarea
        bind:this={draftEl}
        value={d.text}
        oninput={(e) => player.noteDraft && (player.noteDraft.text = e.currentTarget.value)}
        rows="3" placeholder={$t("video.notes.placeholder") as string} onkeydown={draftKeys}></textarea>
      <div class="da">
        {#if player.info?.source.kind === "local"}
          <label class="cap"><input type="checkbox" checked={d.capture} onchange={(e) => player.noteDraft && (player.noteDraft.capture = e.currentTarget.checked)} /> <VIcon name="camera" size={13} /> {$t("video.notes.capture")}</label>
        {/if}
        <span class="sp"></span>
        <button class="btn btn-sm" onclick={() => (player.noteDraft = null)}>{$t("video.cancel")}</button>
        <button class="btn btn-sm btn-primary" onclick={save} disabled={!d.text.trim() || saving}>{$t("video.save")}</button>
      </div>
    </div>
  {/if}

  {#if player.notes.length}
    <ul>
      {#each player.notes as n (n.id)}
        <li>
          <button class="time" onclick={() => seek(n.t_ms / 1000)}>{fmtTime(n.t_ms)}{n.t_end_ms != null ? `–${fmtTime(n.t_end_ms)}` : ""}</button>
          <div class="body">
            {#if editing === n.id}
              <!-- svelte-ignore a11y_autofocus -->
              <textarea
                bind:value={editText}
                rows="3"
                autofocus
                onkeydown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void commitEdit(n.id);
                  else if (e.key === "Escape") editing = null;
                }}
                onblur={() => commitEdit(n.id)}
              ></textarea>
            {:else}
              <p>{n.text}</p>
            {/if}
            {#if n.image}
              {@const src = api.serverUrl(n.image)}
              {#if src}<button class="img" onclick={() => seek(n.t_ms / 1000)}><img {src} alt="" loading="lazy" /></button>{/if}
            {/if}
          </div>
          <div class="acts">
            <button
              onclick={() => {
                editing = n.id;
                editText = n.text;
              }}
              aria-label={$t("video.notes.edit") as string}
              title={$t("video.notes.edit") as string}><VIcon name="edit" size={13} /></button
            >
            <button onclick={() => remove(n.id)} aria-label={$t("video.notes.delete") as string} title={$t("video.notes.delete") as string}><VIcon name="trash" size={13} /></button>
          </div>
        </li>
      {/each}
    </ul>
  {:else if !player.noteDraft}
    <div class="empty">
      <img src="/icons/video/notes-64.webp" alt="" width="40" height="40" />
      <p>{$t("video.notes.empty")}</p>
    </div>
  {/if}
</div>

<style>
  .notes {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
  }
  .sp {
    flex: 1;
  }
  .draft {
    margin: 4px 10px 8px;
    padding: 10px;
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .dt {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 6px;
  }
  .ts {
    font-variant-numeric: tabular-nums;
    font-weight: 650;
    font-size: var(--text-sm);
    color: var(--accent-hi);
  }
  .tag {
    font-size: var(--text-caption);
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--purple) 20%, transparent);
    color: var(--purple);
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    padding: 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
  }
  textarea:focus {
    outline: none;
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .da {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
  }
  .cap {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    cursor: pointer;
  }
  .cap input {
    accent-color: var(--accent);
  }
  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0 6px 16px;
  }
  li {
    display: flex;
    gap: 8px;
    padding: 8px 6px;
    border-radius: var(--radius-md);
  }
  li:hover {
    background: var(--fill-1);
  }
  .time {
    all: unset;
    align-self: flex-start;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-xs);
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    white-space: nowrap;
  }
  .time:focus-visible,
  .acts button:focus-visible,
  .img:focus-visible {
    outline: var(--focus-ring);
  }
  .body {
    flex: 1;
    min-width: 0;
  }
  .body p {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .img {
    all: unset;
    display: block;
    margin-top: 6px;
    cursor: pointer;
  }
  .img img {
    display: block;
    width: 100%;
    max-width: 260px;
    border-radius: var(--radius-sm);
  }
  .acts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  li:hover .acts,
  li:focus-within .acts {
    opacity: 1;
  }
  .acts button {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .acts button:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 28px 16px;
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .empty p {
    margin: 0;
  }
</style>
