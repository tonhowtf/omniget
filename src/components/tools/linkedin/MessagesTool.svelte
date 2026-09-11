<script lang="ts">
  /** Le o messages.csv do export, agrupa por conversa e gera HTML/Markdown/JSON local. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openPath, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Message = { from: string; to: string; date: string; subject: string; content: string; mine: boolean; empty: boolean };
  type Conversation = { id: string; title: string; participants: string[]; count: number; sent: number; received: number; first: string; last: string; messages: Message[] };
  type Result = {
    me: string; me_guessed: boolean; total: number; sent: number; received: number; empty: number;
    conversations_total: number; matched: number; conversations: Conversation[];
    top_contacts: { name: string; count: number }[]; by_year: { period: string; count: number }[]; exports: string[];
  };

  let path = $state("");
  let me = $state("");
  let query = $state("");
  let exportHtml = $state(true);
  let exportMarkdown = $state(false);
  let exportJson = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let open = $state<string | null>(null);
  let unlisten: (() => void) | null = null;

  const ZIP = [{ name: "Zip", extensions: ["zip"] }];
  const current = $derived(result?.conversations.find((c) => c.id === open) ?? null);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "li-messages") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run(withExports: boolean) {
    if (!path || busy) return;
    busy = true;
    progress = null;
    open = null;
    try {
      result = await invoke<Result>("tool_li_messages", {
        opts: {
          path,
          me: me || null,
          query: query || null,
          limit: 400,
          export_html: withExports && exportHtml,
          export_markdown: withExports && exportMarkdown,
          export_json: withExports && exportJson,
        },
      });
      if (!me && result.me) me = result.me;
      if (withExports && result.exports.length) showToast("success", result.exports[0]);
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
          <div class="group-row-title">{$t("tools.limsg.source")}</div>
          <div class="group-row-sub mono">{path || $t("tools.limsg.source_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(ZIP); if (f) path = f; }}>{$t("tools.limsg.pick_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) path = d; }}>{$t("tools.limsg.pick_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.limsg.me")}</div><div class="group-row-sub">{$t("tools.limsg.me_sub")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={me} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.limsg.search")}</div></div>
        <div class="group-row-trailing"><input class="input" type="search" bind:value={query} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.limsg.exports")}</div><div class="group-row-sub">{$t("tools.limsg.exports_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="check"><input type="checkbox" bind:checked={exportHtml} />HTML</label>
          <label class="check"><input type="checkbox" bind:checked={exportMarkdown} />Markdown</label>
          <label class="check"><input type="checkbox" bind:checked={exportJson} />JSON</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing btn-row">
          {#if result}<button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => run(true)}>{$t("tools.limsg.export")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || !path} onclick={() => run(false)}>{busy ? $t("tools.common.working") : $t("tools.limsg.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="cards">
        <div class="card"><b>{result.total}</b><span>{$t("tools.limsg.messages")}</span></div>
        <div class="card"><b>{result.conversations_total}</b><span>{$t("tools.limsg.conversations")}</span></div>
        <div class="card"><b>{result.sent}</b><span>{$t("tools.limsg.sent")}</span></div>
        <div class="card"><b>{result.received}</b><span>{$t("tools.limsg.received")}</span></div>
        <div class="card"><b>{result.empty}</b><span>{$t("tools.limsg.attachments")}</span></div>
      </div>
    </section>

    {#if result.exports.length}
      <section>
        <div class="group">
          {#each result.exports as e (e)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub mono">{e}</div></div>
              <div class="group-row-trailing btn-row">
                <button class="btn btn-ghost btn-sm" type="button" onclick={() => openPath(e)}>{$t("tools.common.open")}</button>
                <button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(e)}>{$t("tools.common.reveal")}</button>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <section>
      <div class="chat">
        <div class="group list">
          {#each result.conversations as c (c.id)}
            <button class="group-row row-btn" class:on={c.id === open} type="button" onclick={() => (open = c.id)}>
              <div class="group-row-content">
                <div class="group-row-title">{c.title}</div>
                <div class="group-row-sub mono">{c.count} · {c.last}</div>
              </div>
            </button>
          {/each}
        </div>
        <div class="group pane">
          {#if current}
            <div class="group-row"><div class="group-row-content"><div class="group-row-title">{current.title}</div><div class="group-row-sub mono">{current.first} → {current.last}</div></div></div>
            {#each current.messages as m, i (i)}
              <div class="msg" class:mine={m.mine}>
                <div class="meta mono">{m.from} · {m.date}{#if m.subject} · {m.subject}{/if}</div>
                {#if m.empty}<span class="tag">{$t("tools.limsg.attachment")}</span>{:else}<div class="body">{m.content}</div>{/if}
              </div>
            {/each}
          {:else}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.limsg.pick_chat")}</div></div></div>
          {/if}
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .check { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: var(--space-3); }
  .card { background: var(--surface-2, rgba(127, 127, 127, 0.08)); border-radius: var(--radius-md, 10px); padding: var(--space-3); display: flex; flex-direction: column; gap: 2px; }
  .card b { font-size: var(--text-xl); }
  .card span { font-size: var(--text-xs); color: var(--text-dim); }
  .chat { display: grid; grid-template-columns: minmax(200px, 300px) 1fr; gap: var(--space-4); align-items: start; }
  .list { max-height: 60vh; overflow: auto; }
  .pane { max-height: 60vh; overflow: auto; padding: var(--space-3); }
  .row-btn { width: 100%; text-align: left; background: none; border: 0; cursor: pointer; color: inherit; font: inherit; }
  .row-btn.on { background: rgba(10, 102, 194, 0.12); }
  .msg { max-width: 80%; margin-bottom: var(--space-3); padding: var(--space-2) var(--space-3); border-radius: 14px; background: rgba(127, 127, 127, 0.12); }
  .msg.mine { margin-left: auto; background: rgba(10, 102, 194, 0.18); }
  .meta { color: var(--text-dim); margin-bottom: 2px; }
  .body { white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--text-sm); }
  @media (max-width: 700px) { .chat { grid-template-columns: 1fr; } }
</style>
