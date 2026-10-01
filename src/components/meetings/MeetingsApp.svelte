<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import MeetingDetailView from "./MeetingDetail.svelte";
  import MeetingsSettings from "./MeetingsSettings.svelte";
  import TeamsPanel from "./TeamsPanel.svelte";
  import {
    botSend,
    captureSupport,
    classifyLink,
    clock,
    getMeeting,
    importFile,
    importUrl,
    isBusy,
    listMeetings,
    recordStart,
    recordStatus,
    recordStop,
    type CaptureStatus,
    type CaptureSupport,
    type Meeting,
    type MeetingDetail,
    type Progress,
  } from "$lib/meetings/api";

  type Panel = "none" | "record" | "link" | "teams" | "settings";

  let meetings = $state<Meeting[]>([]);
  let selectedId = $state<string | null>(null);
  let detail = $state<MeetingDetail | null>(null);
  let progress = $state<Record<string, Progress>>({});
  let panel = $state<Panel>("none");
  let loading = $state(true);

  let support = $state<CaptureSupport | null>(null);
  let recTitle = $state("");
  let recMic = $state(true);
  let recSystem = $state(true);
  let recDevice = $state("");
  let recording = $state<CaptureStatus | null>(null);
  let busyAction = $state(false);

  let link = $state("");
  let linkTitle = $state("");
  let linkKind = $derived(classifyLink(link));

  let unlisten: UnlistenFn[] = [];
  let recTimer: ReturnType<typeof setInterval> | null = null;

  async function reload() {
    try {
      meetings = await listMeetings();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      loading = false;
    }
    if (selectedId && !meetings.some((m) => m.id === selectedId)) {
      selectedId = null;
      detail = null;
    }
    if (!selectedId && meetings.length) select(meetings[0].id);
  }

  async function select(id: string) {
    selectedId = id;
    try {
      detail = await getMeeting(id);
    } catch (e) {
      detail = null;
      showToast("error", String(e));
    }
  }

  async function refreshSelected(id: string) {
    if (id === selectedId) {
      try {
        detail = await getMeeting(id);
      } catch {
      }
    }
  }

  function startRecTimer() {
    stopRecTimer();
    recTimer = setInterval(async () => {
      recording = await recordStatus().catch(() => null);
      if (!recording) stopRecTimer();
    }, 400);
  }

  function stopRecTimer() {
    if (recTimer) clearInterval(recTimer);
    recTimer = null;
  }

  async function openRecord() {
    panel = panel === "record" ? "none" : "record";
    if (panel === "record" && !support) {
      support = await captureSupport();
      recSystem = support.system_audio;
      recDevice = support.inputs.find((d) => d.default)?.id ?? "";
    }
  }

  async function start() {
    busyAction = true;
    try {
      const res = await recordStart({
        title: recTitle.trim() || undefined,
        mic: recMic,
        system: recSystem && !!support?.system_audio,
        micDevice: recDevice || undefined,
      });
      for (const w of res.warnings) showToast("info", w, 8000);
      panel = "none";
      recTitle = "";
      recording = await recordStatus();
      startRecTimer();
      await reload();
      await select(res.meeting.id);
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      busyAction = false;
    }
  }

  async function stop() {
    busyAction = true;
    try {
      const m = await recordStop();
      stopRecTimer();
      recording = null;
      await reload();
      await select(m.id);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busyAction = false;
    }
  }

  async function submitLink() {
    if (!linkKind) return;
    busyAction = true;
    try {
      const title = linkTitle.trim() || undefined;
      const m = linkKind === "bot" ? await botSend(link.trim(), title) : await importUrl(link.trim(), title);
      showToast("success", $t(linkKind === "bot" ? "meetings.bot_sent" : "meetings.import_started") as string);
      link = "";
      linkTitle = "";
      panel = "none";
      await reload();
      await select(m.id);
    } catch (e) {
      showToast("error", String(e), 9000);
    } finally {
      busyAction = false;
    }
  }

  async function pickFile() {
    const chosen = await open({
      multiple: false,
      filters: [
        {
          name: $t("meetings.file_filter") as string,
          extensions: ["mp4", "m4a", "mp3", "wav", "webm", "mkv", "mov", "ogg", "opus", "flac", "aac", "vtt", "srt", "json"],
        },
      ],
    });
    if (typeof chosen !== "string") return;
    try {
      const m = await importFile(chosen);
      await reload();
      await select(m.id);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function statusLabel(m: Meeting): string {
    return $t(`meetings.status.${m.status}`) as string;
  }

  function sourceIcon(m: Meeting): string {
    switch (m.source.kind) {
      case "capture":
        return "●";
      case "teams":
        return "T";
      case "bot":
        return "B";
      default:
        return "F";
    }
  }

  function dateLabel(ms: number): string {
    return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  }

  onMount(async () => {
    await reload();
    recording = await recordStatus().catch(() => null);
    if (recording) startRecTimer();
    unlisten.push(
      await listen<{ id: string }>("meetings://changed", async (e) => {
        await reload();
        await refreshSelected(e.payload.id);
      }),
      await listen<Progress>("meetings://progress", (e) => {
        if (e.payload.stage === "status") return;
        progress = { ...progress, [e.payload.id]: e.payload };
      }),
    );
  });

  onDestroy(() => {
    stopRecTimer();
    for (const u of unlisten) u();
  });
</script>

<section class="meetings">
  <header class="meetings-head">
    <NavIcon icon="meetings" size={44} />
    <div class="head-copy">
      <h1>{$t("meetings.title")}</h1>
      <p>{$t("meetings.subtitle")}</p>
    </div>
    <div class="head-actions">
      {#if recording}
        <button class="btn btn-destructive" onclick={stop} disabled={busyAction}>{$t("meetings.stop")}</button>
      {:else}
        <button class="btn btn-primary" onclick={openRecord} aria-expanded={panel === "record"}>{$t("meetings.record")}</button>
      {/if}
      <button class="btn btn-secondary" onclick={() => (panel = panel === "link" ? "none" : "link")} aria-expanded={panel === "link"}>
        {$t("meetings.paste_link")}
      </button>
      <button class="btn btn-secondary" onclick={pickFile}>{$t("meetings.import_file")}</button>
      <button class="btn btn-secondary" onclick={() => (panel = panel === "teams" ? "none" : "teams")} aria-expanded={panel === "teams"}>Teams</button>
      <button class="btn btn-ghost" onclick={() => (panel = panel === "settings" ? "none" : "settings")} aria-expanded={panel === "settings"}>
        {$t("meetings.settings")}
      </button>
    </div>
  </header>

  {#if recording}
    <div class="rec-bar" role="status">
      <span class="rec-dot" aria-hidden="true"></span>
      <strong>{$t("meetings.recording")}</strong>
      <span class="rec-clock">{clock(recording.elapsed_ms)}</span>
      {#if recording.mic_level !== null}
        <span class="meter" title={$t("meetings.mic") as string}>
          <span class="meter-label">{$t("meetings.mic")}</span>
          <span class="meter-track"><span class="meter-fill" style:width="{Math.min(100, (recording.mic_level ?? 0) * 400)}%"></span></span>
        </span>
      {/if}
      {#if recording.system_level !== null}
        <span class="meter" title={$t("meetings.system_audio") as string}>
          <span class="meter-label">{$t("meetings.system_audio")}</span>
          <span class="meter-track"><span class="meter-fill" style:width="{Math.min(100, (recording.system_level ?? 0) * 400)}%"></span></span>
        </span>
      {/if}
      <span class="rec-hint">{$t("meetings.recording_hint")}</span>
    </div>
  {/if}

  {#if panel === "record"}
    <div class="panel">
      <label class="field">
        <span>{$t("meetings.title_optional")}</span>
        <input class="input" bind:value={recTitle} placeholder={$t("meetings.title_placeholder") as string} />
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={recMic} />
        <span>{$t("meetings.mic")}</span>
      </label>
      {#if recMic && support && support.inputs.length > 1}
        <select class="input" bind:value={recDevice} aria-label={$t("meetings.mic_device") as string}>
          {#each support.inputs as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      {/if}
      <label class="check" class:off={!support?.system_audio}>
        <input type="checkbox" bind:checked={recSystem} disabled={!support?.system_audio} />
        <span>{$t("meetings.system_audio")}</span>
      </label>
      <p class="hint">
        {support?.system_audio ? support.system_audio_note || $t("meetings.system_audio_hint") : $t("meetings.system_audio_unavailable")}
      </p>
      <p class="hint">{$t("meetings.consent_hint")}</p>
      <div class="panel-actions">
        <button class="btn btn-primary" onclick={start} disabled={busyAction || (!recMic && !recSystem)}>{$t("meetings.start_recording")}</button>
      </div>
    </div>
  {:else if panel === "link"}
    <div class="panel">
      <label class="field">
        <span>{$t("meetings.link_label")}</span>
        <input class="input" bind:value={link} placeholder="https://teams.microsoft.com/meet/… · https://contoso.sharepoint.com/…" />
      </label>
      <p class="hint">
        {#if linkKind === "bot"}
          {$t("meetings.link_is_meeting")}
        {:else if linkKind === "recording"}
          {$t("meetings.link_is_recording")}
        {:else}
          {$t("meetings.link_help")}
        {/if}
      </p>
      <label class="field">
        <span>{$t("meetings.title_optional")}</span>
        <input class="input" bind:value={linkTitle} />
      </label>
      <div class="panel-actions">
        <button class="btn btn-primary" onclick={submitLink} disabled={!linkKind || busyAction}>
          {linkKind === "bot" ? $t("meetings.send_bot") : $t("meetings.import_recording")}
        </button>
      </div>
    </div>
  {:else if panel === "teams"}
    <div class="panel">
      <TeamsPanel
        onsettings={() => (panel = "settings")}
        onimported={async (id) => {
          await reload();
          await select(id);
        }}
      />
    </div>
  {:else if panel === "settings"}
    <div class="panel">
      <MeetingsSettings />
    </div>
  {/if}

  <div class="meetings-body">
    <aside class="list" aria-label={$t("meetings.title") as string}>
      {#if loading}
        <p class="empty">…</p>
      {:else if meetings.length === 0}
        <div class="empty">
          <p><strong>{$t("meetings.empty_title")}</strong></p>
          <p>{$t("meetings.empty_body")}</p>
        </div>
      {:else}
        <ul>
          {#each meetings as m (m.id)}
            <li>
              <button class="row" class:active={m.id === selectedId} onclick={() => select(m.id)} aria-current={m.id === selectedId}>
                <span class="row-kind" data-kind={m.source.kind} aria-hidden="true">{sourceIcon(m)}</span>
                <span class="row-main">
                  <span class="row-title">{m.title}</span>
                  <span class="row-meta">
                    {dateLabel(m.created_at_ms)}{#if m.duration_ms} · {clock(m.duration_ms)}{/if}
                  </span>
                </span>
                <span class="row-status" data-status={m.status} class:pulse={isBusy(m.status) || m.status === "recording" || m.status === "bot_live"}>
                  {statusLabel(m)}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </aside>
    <main class="detail">
      {#if detail}
        <MeetingDetailView {detail} progress={progress[detail.meeting.id] ?? null} onchange={() => refreshSelected(detail!.meeting.id)} ondeleted={reload} />
      {:else if !loading}
        <div class="detail-empty">
          <p>{$t("meetings.pick_one")}</p>
        </div>
      {/if}
    </main>
  </div>
</section>

<style>
  .meetings {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    height: 100%;
    min-height: 0;
    box-sizing: border-box;
  }
  .meetings-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .head-copy {
    flex: 1;
    min-width: 200px;
  }
  .head-copy h1 {
    margin: 0;
    font-size: var(--text-xl);
  }
  .head-copy p {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .head-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .rec-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--danger) 12%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    flex-wrap: wrap;
    font-size: var(--text-sm);
  }
  .rec-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--danger);
    animation: blink 1.2s ease-in-out infinite;
  }
  .rec-clock {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .rec-hint {
    color: var(--text-muted);
    margin-left: auto;
  }
  .meter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .meter-label {
    color: var(--text-muted);
  }
  .meter-track {
    width: 80px;
    height: 6px;
    border-radius: 3px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .meter-fill {
    display: block;
    height: 100%;
    background: var(--success);
    transition: width 0.15s linear;
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .check.off {
    opacity: 0.55;
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .panel-actions {
    display: flex;
    justify-content: flex-end;
  }
  .meetings-body {
    display: grid;
    grid-template-columns: minmax(220px, 300px) 1fr;
    gap: var(--space-3);
    flex: 1;
    min-height: 0;
  }
  .list {
    overflow-y: auto;
    border-right: 1px solid var(--separator);
    padding-right: var(--space-2);
  }
  .list ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .row:hover {
    background: var(--fill-1);
  }
  .row.active {
    background: var(--accent-soft);
  }
  .row-kind {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 700;
    color: white;
    background: #1f8a9e;
  }
  .row-kind[data-kind="capture"] {
    background: var(--danger);
  }
  .row-kind[data-kind="teams"] {
    background: #5b5fc7;
  }
  .row-kind[data-kind="bot"] {
    background: #7b3fe4;
  }
  .row-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .row-title {
    font-size: var(--text-sm);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-meta {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .row-status {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .row-status[data-status="failed"] {
    color: var(--danger);
  }
  .row-status[data-status="ready"] {
    color: var(--success);
  }
  .row-status.pulse {
    color: var(--accent-text);
    animation: blink 1.6s ease-in-out infinite;
  }
  .empty,
  .detail-empty {
    color: var(--text-muted);
    font-size: var(--text-sm);
    padding: var(--space-3);
  }
  .detail {
    overflow-y: auto;
    min-width: 0;
  }
  @keyframes blink {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rec-dot,
    .row-status.pulse {
      animation: none;
    }
  }
  @media (max-width: 720px) {
    .meetings-body {
      grid-template-columns: 1fr;
    }
    .list {
      border-right: 0;
      max-height: 40vh;
    }
  }
</style>
