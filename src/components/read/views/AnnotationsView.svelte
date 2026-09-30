<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookById, coverUrl, library } from "$lib/read/library-store.svelte";
  import { fmtDate, go, openBook, tr } from "$lib/read/ui-store.svelte";
  import type { Annotation } from "$lib/read/types";
  import { agentActions } from "$lib/native-agent/actions";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  type Word = { id: number; word: string; context: string; definition: string; lang: string; book_id: number | null; book_title: string; created_at: number };

  let tab = $state<"notes" | "words">("notes");
  let list = $state<Annotation[]>([]);
  let words = $state<Word[]>([]);
  let q = $state("");
  let kind = $state<"all" | "highlight" | "note" | "bookmark">("all");
  let lib = $derived(library());

  onMount(async () => {
    [list, words] = await Promise.all([
      invoke<Annotation[]>("read_annotations", { bookId: null }).catch(() => []),
      invoke<Word[]>("read_vocab_list").catch(() => []),
    ]);
  });

  let shown = $derived.by(() => {
    void lib.version;
    const s = q.trim().toLowerCase();
    return list.filter(
      (a) =>
        (kind === "all" || a.kind === kind) &&
        (!s || a.text.toLowerCase().includes(s) || a.note.toLowerCase().includes(s) || (bookById(a.book_id)?.title ?? "").toLowerCase().includes(s)),
    );
  });

  let byBook = $derived.by(() => {
    const m = new Map<number, Annotation[]>();
    for (const a of shown) m.set(a.book_id, [...(m.get(a.book_id) ?? []), a]);
    return [...m.entries()];
  });

  async function exportWords() {
    const path = await save({ defaultPath: "omniget-words.txt" });
    if (!path) return;
    const n = await invoke<number>("read_vocab_export", { path });
    showToast("success", tr("read.words.exported", { count: n }));
  }
</script>

<div class="r-h">
  <h1>{$t("read.nav.annotations")}</h1>
  <span class="grow"></span>
  <div class="mac-segmented">
    <button class="mac-segmented-btn" data-agent-actions="click" use:agentActions={["click"]} class:active={tab === "notes"} onclick={() => (tab = "notes")}>{$t("read.notes.title")} ({list.length})</button>
    <button class="mac-segmented-btn" data-agent-actions="click" use:agentActions={["click"]} class:active={tab === "words"} onclick={() => (tab = "words")}>{$t("read.words.title")} ({words.length})</button>
  </div>
</div>

{#if tab === "notes"}
  <div class="r-toolbar">
    <input class="input" style="width:260px" bind:value={q} placeholder={$t("read.notes.search") as string} />
    {#each ["all", "highlight", "note", "bookmark"] as k (k)}
      <button class="r-chip" class:on={kind === k} onclick={() => (kind = k as typeof kind)}>{$t(`read.notes.kind_${k}`)}</button>
    {/each}
    <span style="flex:1"></span>
    <button class="r-chip" onclick={() => go({ kind: "settings", tab: "notes" })}><ReadIcon name="link" size={13} /> {$t("read.notes.live_export")}</button>
  </div>
  {#if !byBook.length}
    <div class="r-empty"><ReadIcon name="highlighter" size={36} /><p>{$t("read.notes.none_all")}</p></div>
  {/if}
  {#each byBook as [bid, items] (bid)}
    {@const b = bookById(bid)}
    <div class="grp">
      <button class="bk" onclick={() => b && go({ kind: "book", id: b.id })}>
        <div class="c"><Cover src={coverUrl(b?.cover)} title={b?.title ?? "?"} radius={3} /></div>
        <b>{b?.title ?? "?"}</b><span class="r-muted">{items.length}</span>
      </button>
      {#each items as a (a.id)}
        <button class="nt" style:--c={a.color} onclick={() => b && openBook(b, a.locator)}>
          {#if a.kind === "bookmark"}<span class="bm"><ReadIcon name="bookmark-fill" size={12} /> {a.chapter || $t("read.reader.bookmark")}</span>{:else}<q>{a.text}</q>{/if}
          {#if a.note}<p>{a.note}</p>{/if}
          <span class="r-muted">{a.chapter} · {fmtDate(a.created_at)}</span>
        </button>
      {/each}
    </div>
  {/each}
{:else}
  <div class="r-toolbar">
    <button class="btn" onclick={exportWords} disabled={!words.length}><ReadIcon name="upload" size={14} /> {$t("read.words.export_anki")}</button>
  </div>
  {#if !words.length}
    <div class="r-empty"><ReadIcon name="book" size={36} /><p>{$t("read.words.none")}</p></div>
  {/if}
  <ul class="r-list">
    {#each words as w (w.id)}
      <li class="r-li word">
        <div class="wi">
          <b>{w.word}</b> <span class="r-badge">{w.lang}</span>
          <p>{w.definition}</p>
          {#if w.context}<p class="r-muted ctx">{w.context.slice(0, 240)}</p>{/if}
          <span class="r-muted">{w.book_title} · {fmtDate(w.created_at)}</span>
        </div>
        <button
          class="r-icon-btn"
          onclick={async () => {
            await invoke("read_vocab_delete", { id: w.id });
            words = words.filter((x) => x.id !== w.id);
          }}
          aria-label={$t("read.delete") as string}><ReadIcon name="trash" size={14} /></button
        >
      </li>
    {/each}
  </ul>
{/if}

<style>
  .grp {
    margin-bottom: var(--space-5);
  }
  .bk {
    all: unset;
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
    cursor: pointer;
  }
  .bk .c {
    width: 28px;
  }
  .nt {
    all: unset;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    margin: 0 0 6px 38px;
    max-width: calc(100% - 38px);
    padding: 8px 10px;
    border-left: 3px solid var(--c, #e5c100);
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    background: var(--fill-1);
    cursor: pointer;
  }
  .nt q {
    font-family: Georgia, serif;
    line-height: 1.5;
  }
  .nt p {
    margin: 0;
    font-size: var(--text-sm);
  }
  .bm {
    display: flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
  }
  .word {
    align-items: flex-start;
  }
  .wi {
    flex: 1;
  }
  .wi p {
    margin: 4px 0;
  }
  .ctx {
    font-style: italic;
  }
</style>
