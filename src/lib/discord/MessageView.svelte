<script lang="ts">
  import { tick, untrack } from "svelte";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import MessageRow from "./MessageRow.svelte";
  import Composer from "./Composer.svelte";
  import * as dc from "./api";
  import type { DcChannel, DcMessage, DcUser } from "./api";
  import { applyDeleted, applyEdited, applySent, feedOf, findChannel, loadLatest, loadOlder, markRead, setViewing } from "./client.svelte";
  import { dayKey, groupedWith, type Lookup } from "./format";

  let {
    channel,
    me,
    place,
    outputDir,
    onDownload,
    onExport,
  }: {
    channel: DcChannel;
    me: DcUser;
    place: string;
    outputDir: string;
    onDownload: () => void;
    onExport: () => void;
  } = $props();

  let scroller: HTMLDivElement | undefined = $state();
  let atBottom = true;
  let replyTo = $state<DcMessage | null>(null);
  let editing = $state<DcMessage | null>(null);
  let sendError = $state<string | null>(null);

  let feed = $derived(feedOf(channel.id));
  const users = new Map<string, DcUser>();
  const look: Lookup = {
    meId: "",
    users,
    channelName: (id) => findChannel(id)?.name ?? null,
    everyone: "everyone",
    role: "role",
  };

  let rows = $derived.by(() => {
    look.meId = me.id;
    look.role = $t("discord.messages.role");
    const list = feed.messages;
    const out: { m: DcMessage; grouped: boolean; day: string | null }[] = [];
    let prevDay = "";
    for (let i = 0; i < list.length; i++) {
      const m = list[i];
      if (m.author.id && !users.has(m.author.id)) users.set(m.author.id, m.author);
      for (const u of m.mentions) if (!users.has(u.id)) users.set(u.id, u);
      const dk = dayKey(m.timestamp);
      const newDay = dk !== prevDay;
      prevDay = dk;
      out.push({ m, grouped: !newDay && groupedWith(list[i - 1], m), day: newDay ? m.timestamp : null });
    }
    return out;
  });

  $effect(() => {
    const id = channel.id;
    untrack(() => {
      replyTo = null;
      editing = null;
      sendError = null;
      atBottom = true;
      setViewing(id, true);
      if (!feedOf(id).loaded) void loadLatest(id);
    });
    return () => setViewing(null);
  });

  $effect(() => {
    const f = feed;
    if (!f.loaded && !f.loading) untrack(() => void loadLatest(channel.id));
  });

  $effect(() => {
    const list = feed.messages;
    const last = list[list.length - 1];
    untrack(() => {
      if (!atBottom || feed.detached) return;
      void tick().then(() => {
        if (!scroller) return;
        scroller.scrollTop = scroller.scrollHeight;
        if (scroller.scrollHeight <= scroller.clientHeight + 8 && feed.hasOlder && !feed.loading && list.length) void older();
      });
      if (last && document.visibilityState === "visible") markRead(channel.id, last.id);
    });
  });

  async function older() {
    if (!scroller) return;
    const anchor = scroller.querySelector<HTMLElement>("[data-id]");
    const id = anchor?.dataset.id;
    const y = anchor?.getBoundingClientRect().top ?? 0;
    const n = await loadOlder(channel.id);
    if (!n || !id) return;
    await tick();
    const again = scroller.querySelector<HTMLElement>(`[data-id="${id}"]`);
    if (again) scroller.scrollTop += again.getBoundingClientRect().top - y;
  }

  function onScroll() {
    if (!scroller) return;
    const near = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
    if (near !== atBottom) {
      atBottom = near;
      setViewing(channel.id, near);
      const list = feed.messages;
      if (near && !feed.detached && list.length) markRead(channel.id, list[list.length - 1].id);
    }
    if (scroller.scrollTop < 400 && feed.hasOlder && !feed.loading) void older();
  }

  async function jumpToPresent() {
    atBottom = true;
    await loadLatest(channel.id);
  }

  function clickLinks(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest?.("a.ext") as HTMLAnchorElement | null;
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href");
    if (href && /^https?:\/\//.test(href)) void shellOpen(href).catch(() => {});
  }

  function errMsg(e: unknown): string {
    const k = dc.errKey(e);
    return k === "rate_limited" ? $t("discord.errors.rate_limited_s", { s: dc.rateLimitSeconds(e) }) : $t(`discord.errors.${k}`);
  }

  async function submit(text: string): Promise<boolean> {
    sendError = null;
    try {
      if (editing) {
        const m = await dc.edit(channel.id, editing.id, text);
        applyEdited(m);
        editing = null;
      } else {
        const m = await dc.send(channel.id, text, replyTo?.id ?? null);
        replyTo = null;
        atBottom = true;
        applySent(m);
      }
      return true;
    } catch (e) {
      sendError = errMsg(e);
      return false;
    }
  }

  function editLast() {
    const list = feed.messages;
    for (let i = list.length - 1; i >= 0; i--) {
      const m = list[i];
      if (m.author.id === me.id && (m.type === 0 || m.type === 19)) {
        replyTo = null;
        editing = m;
        return;
      }
    }
  }

  function startReply(m: DcMessage) {
    editing = null;
    replyTo = m;
  }

  function startEdit(m: DcMessage) {
    replyTo = null;
    editing = m;
  }

  async function remove(m: DcMessage) {
    try {
      await dc.remove(channel.id, m.id);
      applyDeleted(channel.id, m.id);
      if (editing?.id === m.id) editing = null;
    } catch (e) {
      showToast("error", errMsg(e));
    }
  }

  async function downloadOne(m: DcMessage) {
    try {
      const r = await dc.downloadMessage(channel.id, m.id, outputDir || null);
      showToast(r.failed ? "info" : "success", $t("discord.download.saved_n", { n: r.paths.length, folder: r.folder }));
    } catch (e) {
      showToast("error", errMsg(e));
    }
  }

  const fmtDay = (ts: string) => new Date(ts).toLocaleDateString(undefined, { dateStyle: "full" });
  let placeholder = $derived(
    channel.can_send ? $t(channel.guild_id ? "discord.composer.placeholder_channel" : "discord.composer.placeholder_dm", { name: channel.name }) : $t("discord.composer.read_only"),
  );
</script>

<section class="view">
  <header class="v-head">
    <div class="v-title">
      <h3>{channel.guild_id ? `# ${channel.name}` : channel.name}</h3>
      {#if channel.topic}<p title={channel.topic}>{channel.topic}</p>{:else}<p>{place}</p>{/if}
    </div>
    <div class="v-tools">
      <button class="btn btn-ghost btn-sm" onclick={onDownload}><TgIcon name="download" /> {$t("discord.view.download")}</button>
      <button class="btn btn-ghost btn-sm" onclick={onExport}><TgIcon name="save" /> {$t("discord.view.export")}</button>
    </div>
  </header>

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <div class="scroller" bind:this={scroller} onscroll={onScroll} onclick={clickLinks} role="log" aria-live="polite" aria-label={channel.name} tabindex="-1">
    {#if feed.loading && feed.messages.length}
      <div class="older"><span class="spinner"></span></div>
    {:else if feed.loaded && !feed.hasOlder && !feed.error}
      <div class="start">
        <h4>{$t("discord.view.start_title", { name: channel.name })}</h4>
        <p>{$t("discord.view.start_hint")}</p>
      </div>
    {/if}
    {#if feed.error}
      <div class="notice notice-warning err">
        <div class="notice-body"><div class="notice-text">{$t(`discord.errors.${feed.error}`)}</div></div>
        <button class="btn btn-ghost btn-sm" onclick={() => void (feed.messages.length ? older() : loadLatest(channel.id))}>{$t("discord.common.retry")}</button>
      </div>
    {/if}
    {#if !feed.loaded}
      <div class="skel" aria-hidden="true">
        {#each [0, 1, 2, 3, 4] as i (i)}
          <div class="skel-row"><span class="skeleton circle"></span><span class="skeleton line" style:width="{40 + ((i * 17) % 45)}%"></span></div>
        {/each}
      </div>
    {/if}
    {#each rows as r (r.m.id)}
      {#if r.day}<div class="day" role="separator"><span>{fmtDay(r.day)}</span></div>{/if}
      <MessageRow m={r.m} grouped={r.grouped} {look} canSend={channel.can_send} onReply={startReply} onEdit={startEdit} onDelete={remove} onDownload={downloadOne} />
    {/each}
    <div class="tail"></div>
  </div>

  {#if feed.detached}
    <button class="present btn btn-secondary btn-sm" onclick={() => void jumpToPresent()}>{$t("discord.view.jump_present")}</button>
  {/if}

  {#if sendError}<p class="field-error send-err">{sendError}</p>{/if}
  <Composer {placeholder} disabled={!channel.can_send} {replyTo} {editing} onSubmit={submit} onEditLast={editLast} onCancel={() => { replyTo = null; editing = null; }} />
</section>

<style>
  .view {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .v-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-4);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .v-title {
    flex: 1;
    min-width: 0;
  }

  .v-title h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .v-title p {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .v-tools {
    display: flex;
    gap: var(--space-1);
    flex-shrink: 0;
  }

  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-anchor: none;
    padding: var(--space-2) 0 var(--space-2);
    outline: none;
  }

  .older {
    display: flex;
    justify-content: center;
    padding: var(--space-3);
  }

  .start {
    padding: var(--space-6) var(--space-5) var(--space-3);
  }

  .start h4 {
    margin: 0 0 4px;
    font-size: var(--text-xl);
    font-weight: 700;
    color: var(--text);
  }

  .start p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .err {
    margin: var(--space-2) var(--space-4);
  }

  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }

  .skel-row {
    display: flex;
    gap: var(--space-3);
    align-items: center;
  }

  .skel .circle {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-full);
  }

  .skel .line {
    height: 12px;
  }

  .day {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-4) var(--space-4) 0;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
  }

  .day::before,
  .day::after {
    content: "";
    flex: 1;
    height: var(--hairline);
    background: var(--separator);
  }

  .tail {
    height: var(--space-2);
  }

  .present {
    position: absolute;
    left: 50%;
    bottom: 76px;
    transform: translateX(-50%);
    box-shadow: var(--elev-2);
    z-index: 3;
  }

  .send-err {
    margin: 0 var(--space-4);
  }
</style>
