<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    botSend,
    isSoon,
    teamsCalendar,
    teamsChats,
    teamsExport,
    teamsStatus,
    teamsTranscriptImport,
    type CalendarMeeting,
    type TeamsChat,
    type TeamsExport,
    type TeamsStatus,
  } from "$lib/meetings/api";

  // Teams through the person's Microsoft 365 sign-in: the calendar's online
  // meetings (send the bot, import the official transcript) and chat export
  // as Markdown or JSON for an LLM. Sign-in lives in Meetings settings.

  let { onsettings, onimported }: { onsettings: () => void; onimported: (id: string) => void } = $props();

  let status = $state<TeamsStatus | null>(null);
  let meetings = $state<CalendarMeeting[]>([]);
  let chats = $state<TeamsChat[]>([]);
  let query = $state("");
  let selected = $state<Record<string, boolean>>({});
  let since = $state("30d");
  let format = $state<"md" | "json">("md");
  let loading = $state(false);
  let working = $state<string | null>(null);
  let result = $state<TeamsExport | null>(null);
  let error = $state<string | null>(null);

  let chosen = $derived(Object.keys(selected).filter((k) => selected[k]));

  async function load() {
    status = await teamsStatus();
    if (!status.signed_in) return;
    loading = true;
    error = null;
    try {
      const [cal, list] = await Promise.all([teamsCalendar(7, 1), teamsChats(undefined, 100)]);
      meetings = cal;
      chats = list;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function search() {
    try {
      chats = await teamsChats(query.trim() || undefined, 100);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function sendBot(m: CalendarMeeting) {
    if (!m.join_url) return;
    working = `bot:${m.id}`;
    try {
      const created = await botSend(m.join_url, m.subject);
      showToast("success", $t("meetings.bot_sent") as string);
      onimported(created.id);
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      working = null;
    }
  }

  async function importTranscript(m: CalendarMeeting) {
    if (!m.join_url) return;
    working = `tr:${m.id}`;
    try {
      const created = await teamsTranscriptImport(m.join_url, m.subject);
      onimported(created.id);
    } catch (e) {
      showToast("error", String(e), 10000);
    } finally {
      working = null;
    }
  }

  async function exportChats() {
    working = "export";
    result = null;
    try {
      result = await teamsExport(chosen, since === "all" ? undefined : since, format === "json");
      for (const e of result.errors) showToast("error", e, 9000);
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      working = null;
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("meetings.copied_generic") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  const when = (iso: string) => new Date(iso).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

  onMount(load);
</script>

{#if status && !status.signed_in}
  <div class="empty">
    <p>{status.configured ? $t("meetings.teams_signed_out") : $t("meetings.teams_not_configured")}</p>
    <button class="btn btn-primary btn-sm" onclick={onsettings}>{$t("meetings.teams_connect")}</button>
  </div>
{:else if status}
  <p class="who">{$t("meetings.teams_signed_in_as", { name: status.account?.name ?? "", upn: status.account?.upn ?? "" })}</p>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <div class="grid">
    <section>
      <h3>{$t("meetings.teams_calendar")}</h3>
      {#if loading}
        <p class="hint">…</p>
      {:else if meetings.length === 0}
        <p class="hint">{$t("meetings.teams_no_meetings")}</p>
      {:else}
        <ul class="list">
          {#each meetings as m (m.id)}
            <li class:soon={isSoon(m)}>
              <div class="line">
                <strong>{m.subject}</strong>
                <span class="hint">{when(m.start)}{m.organizer ? ` · ${m.organizer}` : ""}</span>
              </div>
              <div class="acts">
                <button class="btn btn-sm btn-secondary" onclick={() => sendBot(m)} disabled={working !== null}>{$t("meetings.send_bot")}</button>
                <button class="btn btn-sm btn-ghost" onclick={() => importTranscript(m)} disabled={working !== null} title={$t("meetings.teams_transcript_hint") as string}>
                  {working === `tr:${m.id}` ? "…" : $t("meetings.teams_import_transcript")}
                </button>
                {#if m.join_url}<button class="btn btn-sm btn-ghost" onclick={() => copy(m.join_url!)}>{$t("meetings.teams_copy_link")}</button>{/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section>
      <h3>{$t("meetings.teams_chats")}</h3>
      <div class="row">
        <input class="input input-search" bind:value={query} onkeydown={(e) => e.key === "Enter" && search()} placeholder={$t("meetings.teams_search_chats") as string} />
      </div>
      {#if chats.length}
        <ul class="list chats">
          {#each chats as c (c.id)}
            <li>
              <label class="check">
                <input type="checkbox" bind:checked={selected[c.id]} />
                <span class="name">{c.name}</span>
                <span class="hint">{c.kind}</span>
              </label>
            </li>
          {/each}
        </ul>
      {:else if !loading}
        <p class="hint">{$t("meetings.teams_no_chats")}</p>
      {/if}
      <div class="row">
        <select class="input" bind:value={since} aria-label={$t("meetings.teams_since") as string}>
          <option value="7d">{$t("meetings.teams_since_7d")}</option>
          <option value="30d">{$t("meetings.teams_since_30d")}</option>
          <option value="90d">{$t("meetings.teams_since_90d")}</option>
          <option value="all">{$t("meetings.teams_since_all")}</option>
        </select>
        <select class="input" bind:value={format} aria-label={$t("meetings.teams_format") as string}>
          <option value="md">Markdown</option>
          <option value="json">JSON</option>
        </select>
        <button class="btn btn-sm btn-primary" onclick={exportChats} disabled={chosen.length === 0 || working !== null}>
          {working === "export" ? "…" : $t("meetings.teams_export", { count: chosen.length })}
        </button>
      </div>
      {#if result}
        <div class="result">
          <p>{$t("meetings.teams_exported", { count: result.exported.length })}</p>
          <ul>
            {#each result.exported as x (x.path)}<li><code>{x.path}</code> · {x.messages}</li>{/each}
          </ul>
          <button class="btn btn-sm btn-ghost" onclick={() => invoke("open_path_default", { path: result!.folder })}>{$t("meetings.open_folder")}</button>
        </div>
      {/if}
      <p class="hint">{$t("meetings.teams_llm_hint")}</p>
    </section>
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-4);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  h3 {
    margin: 0;
    font-size: var(--text-sm);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 320px;
    overflow-y: auto;
  }
  .list li {
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .list li.soon {
    background: var(--accent-soft);
  }
  .line {
    display: flex;
    flex-direction: column;
    font-size: var(--text-sm);
  }
  .acts,
  .row {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
    align-items: center;
  }
  .acts {
    margin-top: 4px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    min-width: 0;
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }
  .result {
    font-size: var(--text-xs);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .result ul {
    margin: 4px 0;
    padding-left: 1.2em;
    word-break: break-all;
  }
  .who,
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
    white-space: pre-wrap;
  }
  .empty {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
    flex-wrap: wrap;
  }
</style>
