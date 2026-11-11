<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import SocialIcon from "./SocialIcon.svelte";
  import { social, type Platform, type Profile } from "./api";
  import {
    WEEKDAYS,
    fileName,
    groupByDay,
    isoToLocalInput,
    localInputToIso,
    rejection,
    sortSlots,
    textLimit,
    weightedLength,
    type ImportReport,
    type ScheduleItem,
    type ScheduleList,
    type ScheduleStatus,
    type Slot,
  } from "./schedule";

  let { platform, me, openProfile }: { platform: Platform; me: Profile; openProfile: (handle: string) => void } = $props();

  type Section = "queue" | "drafts" | "history" | "slots" | "import" | "feeds";
  const SECTIONS: Section[] = ["queue", "drafts", "history", "slots", "import", "feeds"];
  const FILTER: Record<Section, { status: ScheduleStatus[]; order: "asc" | "desc"; limit: number }> = {
    queue: { status: ["scheduled", "sending"], order: "asc", limit: 100 },
    drafts: { status: ["draft"], order: "desc", limit: 100 },
    history: { status: ["sent", "failed", "missed"], order: "desc", limit: 100 },
    slots: { status: ["scheduled"], order: "asc", limit: 1 },
    import: { status: ["scheduled"], order: "asc", limit: 1 },
    feeds: { status: ["scheduled"], order: "asc", limit: 1 },
  };
  const KNOWN_ERRORS = new Set([
    "PAST_TIME",
    "TOO_FAR",
    "TOO_LONG",
    "DUPLICATE",
    "NO_SLOTS",
    "TIME_REQUIRED",
    "EMPTY_TEXT",
    "INVALID_TARGET",
    "INVALID_TIME",
    "TOO_MANY_PARTS",
    "CONFLICTING_TARGETS",
    "MEDIA_NOT_FOUND",
    "MEDIA_TOO_LARGE",
    "UNSUPPORTED",
    "INVALID_CSV",
    "FILE_NOT_FOUND",
    "NOTHING_TO_EXPORT",
    "INVALID_URL",
    "INVALID_TEMPLATE",
    "NOT_EDITABLE",
    "PARTIALLY_SENT",
    "MISSED",
    "INTERRUPTED",
    "NOT_LOGGED_IN",
    "RATE_LIMITED",
  ]);

  let section = $state<Section>("queue");
  let data = $state<ScheduleList | null>(null);
  let loading = $state(true);
  let loadError = $state("");
  let busy = $state(false);
  let actionError = $state("");
  let notice = $state("");
  let exportedPath = $state("");
  let confirmId = $state<number | null>(null);

  let editing = $state<number | null>(null);
  let mode = $state<"post" | "repost">("post");
  let parts = $state<string[]>([""]);
  let replyTo = $state("");
  let quote = $state("");
  let repost = $state("");
  let media = $state<string[]>([]);
  let when = $state<"slot" | "time">("slot");
  let at = $state("");
  let previewSize = $state<"desktop" | "mobile">("desktop");

  let slotDay = $state<string>("mon");
  let slotTime = $state("09:00");
  let grace = $state(10);
  let longPosts = $state<"auto" | "on" | "off">("auto");

  let csvPath = $state("");
  let report = $state<ImportReport | null>(null);

  let feedUrl = $state("");
  let feedTemplate = $state("{title} {link}");
  let feedInterval = $state(60);
  let feedCap = $state(30);

  let supported = $derived(platform === "x");
  let limit = $derived(textLimit(data?.settings.long_posts ?? "auto", me.verified));
  let weights = $derived(parts.map((p) => weightedLength(p)));
  let filled = $derived(mode === "repost" ? repost.trim().length > 0 : parts.every((p) => p.trim().length > 0) && weights.every((w) => w <= limit));
  let timeReady = $derived(when === "slot" ? Boolean(data?.next_slot) : Boolean(localInputToIso(at)));
  let days = $derived(groupByDay(data?.items ?? []));
  let slots = $derived(sortSlots(data?.slots ?? []));

  function fmt(iso: string | null, options: Intl.DateTimeFormatOptions): string {
    if (!iso) return "";
    try {
      return new Intl.DateTimeFormat($locale ?? undefined, options).format(new Date(iso));
    } catch {
      return new Date(iso).toLocaleString();
    }
  }

  const time = (iso: string | null) => fmt(iso, { hour: "2-digit", minute: "2-digit" });
  const dayTime = (iso: string | null) => fmt(iso, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  const dayLabel = (day: string) => fmt(`${day}T12:00:00`, { weekday: "long", day: "numeric", month: "long" });
  const weekdayLabel = (d: string) => fmt(new Date(2024, 0, 1 + WEEKDAYS.indexOf(d as (typeof WEEKDAYS)[number])).toISOString(), { weekday: "long" });

  function message(e: unknown): string {
    const r = rejection(e);
    return KNOWN_ERRORS.has(r.code) ? $t(`social_schedule.errors.${r.code}`) : $t("social_schedule.errors.generic", { detail: r.message });
  }

  function itemError(item: ScheduleItem): string {
    return item.error ? message(item.error) : "";
  }

  async function load(quiet = false) {
    if (!quiet) loading = true;
    loadError = "";
    try {
      const f = FILTER[section];
      const next = await social.call<ScheduleList>("social_schedule_list", { platform, status: f.status, order: f.order, limit: f.limit });
      data = next;
      grace = next.settings.grace_minutes;
      longPosts = next.settings.long_posts;
    } catch (e) {
      loadError = message(e);
    } finally {
      loading = false;
    }
  }

  async function act<T>(args: Record<string, unknown>, tool = "social_schedule"): Promise<T | null> {
    busy = true;
    actionError = "";
    notice = "";
    try {
      const out = await social.call<T>(tool, { platform, ...args });
      confirmId = null;
      await load(true);
      return out;
    } catch (e) {
      actionError = message(e);
      return null;
    } finally {
      busy = false;
    }
  }

  function go(next: Section) {
    section = next;
    actionError = "";
    notice = "";
    confirmId = null;
    data = data ? { ...data, items: [] } : data;
    void load();
  }

  function resetComposer() {
    editing = null;
    mode = "post";
    parts = [""];
    replyTo = "";
    quote = "";
    repost = "";
    media = [];
    when = "slot";
    at = "";
  }

  function content(): Record<string, unknown> {
    if (mode === "repost") return { repost: repost.trim() };
    return {
      text: parts[0],
      thread: parts.slice(1),
      reply_to: replyTo.trim() || undefined,
      quote: quote.trim() || undefined,
      media: media.length ? media : undefined,
    };
  }

  function timing(): Record<string, unknown> {
    return when === "slot" ? { slot: "next" } : { at: localInputToIso(at) };
  }

  async function submit(asDraft: boolean) {
    const args =
      editing !== null
        ? { op: "update", id: editing, ...content(), ...(asDraft ? {} : timing()) }
        : asDraft
          ? { op: "draft", ...content() }
          : { op: "add", ...content(), ...timing() };
    if (await act(args)) resetComposer();
  }

  function edit(item: ScheduleItem) {
    editing = item.id;
    mode = item.repost ? "repost" : "post";
    parts = item.repost ? [""] : [item.text ?? "", ...item.thread];
    replyTo = item.reply_to ?? "";
    quote = item.quote ?? "";
    repost = item.repost ?? "";
    media = [...item.media];
    when = item.status === "scheduled" && item.at ? "time" : "slot";
    at = item.status === "scheduled" ? isoToLocalInput(item.at) : "";
    actionError = "";
    document.getElementById("schedule-composer")?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }

  async function remove(item: ScheduleItem) {
    if (confirmId !== item.id) {
      confirmId = item.id;
      return;
    }
    if ((await act({ op: "cancel", id: item.id })) && editing === item.id) resetComposer();
  }

  async function addImages() {
    try {
      const picked = await pickFile({ multiple: true, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
      const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
      media = [...media, ...list.filter((p) => !media.includes(p))].slice(0, data?.limits.images ?? 4);
    } catch {}
  }

  async function exportCalendar() {
    exportedPath = "";
    const out = await act<{ path: string }>({ op: "export_ics" });
    if (out) exportedPath = out.path;
  }

  async function saveSlots(next: Slot[]) {
    await act({ op: "set_slots", slots: next });
  }

  async function addSlot() {
    if (!/^\d{2}:\d{2}$/.test(slotTime) || slots.some((s) => s.weekday === slotDay && s.time === slotTime)) return;
    await saveSlots([...slots, { weekday: slotDay, time: slotTime }]);
  }

  async function saveSettings() {
    if (await act({ op: "set_slots", grace_minutes: Math.round(grace), long_posts: longPosts })) notice = $t("social_schedule.slots.saved");
  }

  async function chooseCsv() {
    try {
      const picked = await pickFile({ multiple: false, filters: [{ name: "CSV", extensions: ["csv"] }] });
      if (typeof picked === "string") {
        csvPath = picked;
        report = null;
        await runImport(true);
      }
    } catch {}
  }

  async function runImport(dry: boolean) {
    const out = await act<ImportReport>({ op: "import_csv", path: csvPath, dry_run: dry });
    if (!out) return;
    if (dry) {
      report = out;
    } else {
      notice = $t("social_schedule.import.done", { n: out.scheduled });
      report = null;
      csvPath = "";
    }
  }

  async function addFeed() {
    const out = await act({ op: "add", url: feedUrl.trim(), template: feedTemplate, interval_minutes: Math.round(feedInterval), monthly_cap: Math.round(feedCap) }, "social_feeds");
    if (out) feedUrl = "";
  }

  function external(url: string | null) {
    if (url) void shellOpen(url).catch(() => {});
  }

  onMount(() => {
    void load();
    let stop: (() => void) | null = null;
    let disposed = false;
    listen<{ platform?: string }>("social-schedule-changed", (ev) => {
      if (!ev.payload?.platform || ev.payload.platform === platform) void load(true);
    })
      .then((un) => {
        if (disposed) un();
        else stop = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  });
</script>

{#snippet row(item: ScheduleItem)}
  <li class="item" data-status={item.status}>
    <div class="item-when">
      <span class="item-time">{item.status === "draft" ? "" : time(item.sent_at ?? item.at)}</span>
      <span class="tag {item.status === 'sent' ? 'tag-success' : item.status === 'failed' ? 'tag-danger' : item.status === 'missed' ? 'tag-warning' : item.status === 'sending' ? 'tag-info' : ''}">{$t(`social_schedule.status.${item.status}`)}</span>
    </div>
    <div class="item-body">
      <p class="item-text">{item.repost ? $t("social_schedule.preview.reposting", { target: item.repost }) : item.text}</p>
      <div class="item-meta">
        <span class="tag">{$t(`social_schedule.kind.${item.kind}`)}</span>
        {#if item.parts_total > 1}
          <span>{item.parts_sent > 0 && item.status !== "sent" ? $t("social_schedule.list.parts_sent", { sent: item.parts_sent, total: item.parts_total }) : $t("social_schedule.list.parts", { n: item.parts_total })}</span>
        {/if}
        {#if item.media.length}<span>{$t("social_schedule.preview.images", { n: item.media.length })}</span>{/if}
        {#if item.created_by.startsWith("feed:")}<span>{$t("social_schedule.list.by_feed")}</span>{/if}
        {#if item.created_by.startsWith("agent:")}<span>{$t("social_schedule.list.by_agent")}</span>{/if}
        {#if section === "history"}<span>{dayTime(item.sent_at ?? item.at)}</span>{/if}
      </div>
      {#if item.error}<p class="item-error" role="status">{itemError(item)}</p>{/if}
    </div>
    <div class="item-actions">
      {#if item.post_url}
        <button class="btn btn-ghost btn-sm" onclick={() => external(item.post_url)}><SocialIcon name="external" /> {$t("social_schedule.list.open_post")}</button>
      {/if}
      {#if item.status !== "sent" && item.status !== "sending"}
        {#if item.parts_sent === 0}
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => edit(item)}>{$t("social_schedule.list.edit")}</button>
        {/if}
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => void act({ op: "send_now", id: item.id })}>
          <SocialIcon name="send" /> {item.parts_sent > 0 ? $t("social_schedule.list.continue") : $t("social_schedule.list.send_now")}
        </button>
        <button class="btn btn-sm {confirmId === item.id ? 'btn-destructive' : 'btn-ghost'}" disabled={busy} onclick={() => void remove(item)}>
          <SocialIcon name="trash" /> {confirmId === item.id ? $t("social_schedule.list.confirm") : item.status === "draft" ? $t("social_schedule.list.delete") : $t("social_schedule.list.cancel")}
        </button>
      {/if}
    </div>
  </li>
{/snippet}

{#snippet states(emptyKey: string)}
  {#if loading}
    <div class="state"><span class="spinner"></span><span>{$t("social_schedule.list.loading")}</span></div>
  {:else if loadError}
    <div class="notice notice-warning" role="alert">
      <div class="notice-body"><div class="notice-text">{loadError}</div></div>
      <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("social.common.retry")}</button>
    </div>
  {:else}
    <div class="empty-state"><div class="empty-state-title">{$t(emptyKey)}</div></div>
  {/if}
{/snippet}

<div class="sched">
  <div class="notice notice-info" role="note">
    <SocialIcon name="clock" />
    <div class="notice-body">
      <div class="notice-title">{$t("social_schedule.open_only")}</div>
      <div class="notice-text">{$t("social_schedule.late_rule", { minutes: data?.settings.grace_minutes ?? 10 })}</div>
    </div>
  </div>

  {#if !supported}
    <div class="notice notice-warning" role="status">
      <div class="notice-body"><div class="notice-text">{$t("social_schedule.unsupported")}</div></div>
    </div>
  {:else}
    <div class="segmented" role="tablist" aria-label={$t("social.tabs.schedule")}>
      {#each SECTIONS as s (s)}
        <button class="segmented-btn" role="tab" aria-selected={section === s} onclick={() => go(s)}>
          {$t(`social_schedule.sections.${s}`)}
          {#if s === "queue" && data?.counts.scheduled}<span class="pill">{data.counts.scheduled}</span>{/if}
          {#if s === "drafts" && data?.counts.draft}<span class="pill">{data.counts.draft}</span>{/if}
          {#if s === "history" && (data?.counts.failed || data?.counts.missed)}<span class="pill pill-alert">{(data.counts.failed ?? 0) + (data.counts.missed ?? 0)}</span>{/if}
        </button>
      {/each}
    </div>

    {#if actionError}
      <div class="notice notice-danger" role="alert"><div class="notice-body"><div class="notice-text">{actionError}</div></div></div>
    {/if}
    {#if notice}
      <div class="notice notice-success" role="status"><div class="notice-body"><div class="notice-text">{notice}</div></div></div>
    {/if}

    {#if section === "queue" || section === "drafts"}
      <section class="composer" id="schedule-composer" aria-label={$t(editing !== null ? "social_schedule.composer.title_edit" : "social_schedule.composer.title_new")}>
        <div class="composer-form">
          <div class="composer-head">
            <h2>{$t(editing !== null ? "social_schedule.composer.title_edit" : "social_schedule.composer.title_new")}</h2>
            <div class="segmented" role="tablist">
              <button class="segmented-btn" role="tab" aria-selected={mode === "post"} onclick={() => (mode = "post")}>{$t("social_schedule.kind.post")}</button>
              <button class="segmented-btn" role="tab" aria-selected={mode === "repost"} onclick={() => (mode = "repost")}>{$t("social_schedule.kind.repost")}</button>
            </div>
          </div>

          {#if mode === "repost"}
            <label class="field">
              <span class="field-label">{$t("social_schedule.composer.repost_target")}</span>
              <input class="input" bind:value={repost} maxlength="500" placeholder="https://x.com/…/status/…" />
            </label>
          {:else}
            {#each parts as _, i (i)}
              <div class="part">
                <textarea
                  bind:value={parts[i]}
                  rows={i === 0 ? 3 : 2}
                  maxlength="4000"
                  aria-label={$t("social_schedule.composer.part_placeholder", { n: i + 1 })}
                  placeholder={i === 0 ? $t("social_schedule.composer.placeholder") : $t("social_schedule.composer.part_placeholder", { n: i + 1 })}
                ></textarea>
                <div class="part-foot">
                  <span class="count" class:over={weights[i] > limit} aria-live="polite">
                    {weights[i]}/{limit}{#if weights[i] > limit} · {$t("social_schedule.composer.over", { n: weights[i] - limit })}{/if}
                  </span>
                  {#if i > 0}
                    <button class="btn btn-ghost btn-sm" onclick={() => (parts = parts.filter((_, k) => k !== i))} aria-label={$t("social_schedule.composer.remove_part")}><SocialIcon name="trash" /></button>
                  {/if}
                </div>
              </div>
            {/each}
            <div class="composer-row">
              <button class="btn btn-ghost btn-sm" disabled={parts.length >= (data?.limits.thread_parts ?? 25)} onclick={() => (parts = [...parts, ""])}>
                <SocialIcon name="plus" /> {$t("social_schedule.composer.add_part")}
              </button>
              <button class="btn btn-ghost btn-sm" disabled={media.length >= (data?.limits.images ?? 4)} onclick={() => void addImages()} title={$t("social_schedule.composer.images_hint")}>
                <SocialIcon name="plus" /> {$t("social_schedule.composer.add_image")}
              </button>
            </div>
            {#if media.length}
              <ul class="chips">
                {#each media as m (m)}
                  <li class="tag">
                    {fileName(m)}
                    <button class="chip-x" onclick={() => (media = media.filter((x) => x !== m))} aria-label={$t("social_schedule.composer.remove_image")}>×</button>
                  </li>
                {/each}
              </ul>
            {/if}
            <div class="composer-grid">
              <label class="field">
                <span class="field-label">{$t("social_schedule.composer.reply_to")}</span>
                <input class="input" bind:value={replyTo} maxlength="500" placeholder="https://x.com/…/status/…" />
              </label>
              <label class="field">
                <span class="field-label">{$t("social_schedule.composer.quote")}</span>
                <input class="input" bind:value={quote} maxlength="500" placeholder="https://x.com/…/status/…" />
              </label>
            </div>
          {/if}

          <fieldset class="when">
            <legend class="field-label">{$t("social_schedule.composer.when")}</legend>
            <label class="when-option">
              <input class="radio" type="radio" name="schedule-when" value="slot" bind:group={when} />
              <span>
                {$t("social_schedule.composer.next_slot")}
                <small>{data?.next_slot ? dayTime(data.next_slot) : $t("social_schedule.composer.next_slot_none")}</small>
              </span>
            </label>
            <label class="when-option">
              <input class="radio" type="radio" name="schedule-when" value="time" bind:group={when} />
              <span>{$t("social_schedule.composer.pick_time")}</span>
              <input class="input" type="datetime-local" bind:value={at} onfocus={() => (when = "time")} aria-label={$t("social_schedule.composer.pick_time")} />
            </label>
          </fieldset>

          <div class="composer-row end">
            {#if editing !== null}
              <button class="btn btn-ghost btn-sm" onclick={resetComposer}>{$t("social_schedule.composer.cancel_edit")}</button>
            {/if}
            {#if editing === null || data?.items.find((x) => x.id === editing)?.status === "draft"}
              <button class="btn btn-secondary btn-sm" disabled={busy || !filled} onclick={() => void submit(true)}>{$t("social_schedule.composer.save_draft")}</button>
            {/if}
            <button class="btn btn-primary btn-sm" disabled={busy || !filled || !timeReady} onclick={() => void submit(false)}>
              <SocialIcon name="clock" /> {$t(editing !== null ? "social_schedule.composer.save" : "social_schedule.composer.schedule")}
            </button>
          </div>
        </div>

        <div class="preview" aria-label={$t("social_schedule.preview.title")}>
          <div class="preview-head">
            <span class="field-label">{$t("social_schedule.preview.title")}</span>
            <div class="segmented" role="tablist">
              <button class="segmented-btn" role="tab" aria-selected={previewSize === "desktop"} onclick={() => (previewSize = "desktop")}>{$t("social_schedule.preview.desktop")}</button>
              <button class="segmented-btn" role="tab" aria-selected={previewSize === "mobile"} onclick={() => (previewSize = "mobile")}>{$t("social_schedule.preview.mobile")}</button>
            </div>
          </div>
          <div class="preview-frame" class:mobile={previewSize === "mobile"}>
            {#if mode === "repost"}
              <p class="preview-note"><SocialIcon name="repost" /> {repost.trim() ? $t("social_schedule.preview.reposting", { target: repost.trim() }) : $t("social_schedule.preview.empty")}</p>
            {:else if !parts.some((p) => p.trim())}
              <p class="preview-note">{$t("social_schedule.preview.empty")}</p>
            {:else}
              {#each parts as p, i (i)}
                <article class="preview-post" class:chained={i < parts.length - 1}>
                  {#if me.avatar}<img class="avatar avatar-sm" src={me.avatar} alt="" />{:else}<span class="avatar avatar-sm"></span>{/if}
                  <div class="preview-body">
                    <button class="preview-author" onclick={() => openProfile(me.handle)}><strong>{me.name}</strong> <span>@{me.handle}</span></button>
                    {#if i === 0 && replyTo.trim()}<p class="preview-note">{$t("social_schedule.preview.replying", { target: replyTo.trim() })}</p>{/if}
                    <p class="preview-text">{p}</p>
                    {#if i === 0 && media.length}<p class="preview-note">{$t("social_schedule.preview.images", { n: media.length })}</p>{/if}
                    {#if i === 0 && quote.trim()}<p class="preview-quote">{$t("social_schedule.preview.quoting", { target: quote.trim() })}</p>{/if}
                  </div>
                </article>
              {/each}
            {/if}
          </div>
        </div>
      </section>
    {/if}

    {#if section === "queue"}
      <div class="section-header">
        <h2 class="section-header-title">{$t("social_schedule.sections.queue")}</h2>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => void exportCalendar()}><SocialIcon name="calendar" /> {$t("social_schedule.list.export")}</button>
      </div>
      {#if exportedPath}
        <div class="notice notice-success" role="status">
          <div class="notice-body"><div class="notice-text">{$t("social_schedule.list.exported", { path: exportedPath })}</div></div>
          <button class="btn btn-ghost btn-sm" onclick={() => void invoke("reveal_file", { path: exportedPath }).catch(() => {})}>{$t("social_schedule.list.reveal")}</button>
        </div>
      {/if}
      {#if days.length}
        {#each days as d (d.day)}
          <h3 class="day">{dayLabel(d.day)}</h3>
          <ul class="items">
            {#each d.items as item (item.id)}{@render row(item)}{/each}
          </ul>
        {/each}
        {#if data?.truncated}<p class="more-note">{$t("social_schedule.list.more", { returned: data.returned, total: data.total })}</p>{/if}
      {:else}
        {@render states("social_schedule.list.empty_queue")}
      {/if}
    {:else if section === "drafts" || section === "history"}
      {#if data?.items.length && !loading}
        <ul class="items">
          {#each data.items as item (item.id)}{@render row(item)}{/each}
        </ul>
        {#if data.truncated}<p class="more-note">{$t("social_schedule.list.more", { returned: data.returned, total: data.total })}</p>{/if}
      {:else}
        {@render states(section === "drafts" ? "social_schedule.list.empty_drafts" : "social_schedule.list.empty_history")}
      {/if}
    {:else if section === "slots"}
      {#if !data}
        {@render states("social_schedule.slots.empty")}
      {:else}
        <section class="card">
          <h2>{$t("social_schedule.slots.title")}</h2>
          <p class="hint">{$t("social_schedule.slots.hint")}</p>
          {#if slots.length}
            <ul class="chips">
              {#each slots as s (`${s.weekday}${s.time}`)}
                <li class="tag">
                  {weekdayLabel(s.weekday)} · {s.time}
                  <button class="chip-x" disabled={busy} onclick={() => void saveSlots(slots.filter((x) => x !== s))} aria-label={$t("social_schedule.slots.remove")}>×</button>
                </li>
              {/each}
            </ul>
            <p class="hint">{$t("social_schedule.slots.next", { time: dayTime(data.next_slot) })}</p>
          {:else}
            <div class="empty-state"><div class="empty-state-title">{$t("social_schedule.slots.empty")}</div></div>
          {/if}
          <div class="composer-row">
            <label class="field">
              <span class="field-label">{$t("social_schedule.slots.weekday")}</span>
              <select class="input" bind:value={slotDay}>
                {#each WEEKDAYS as d (d)}<option value={d}>{weekdayLabel(d)}</option>{/each}
              </select>
            </label>
            <label class="field">
              <span class="field-label">{$t("social_schedule.slots.time")}</span>
              <input class="input" type="time" bind:value={slotTime} />
            </label>
            <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => void addSlot()}><SocialIcon name="plus" /> {$t("social_schedule.slots.add")}</button>
          </div>
        </section>
        <section class="card">
          <div class="composer-grid">
            <label class="field">
              <span class="field-label">{$t("social_schedule.slots.grace")}</span>
              <input class="input" type="number" min="0" max="1440" bind:value={grace} />
            </label>
            <label class="field">
              <span class="field-label">{$t("social_schedule.slots.long_posts")}</span>
              <select class="input" bind:value={longPosts}>
                <option value="auto">{$t("social_schedule.slots.long_auto")}</option>
                <option value="on">{$t("social_schedule.slots.long_on")}</option>
                <option value="off">{$t("social_schedule.slots.long_off")}</option>
              </select>
            </label>
          </div>
          <div class="composer-row end">
            <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => void saveSettings()}>{$t("social_schedule.slots.save")}</button>
          </div>
        </section>
      {/if}
    {:else if section === "import"}
      <section class="card">
        <h2>{$t("social_schedule.import.title")}</h2>
        <p class="hint">{$t("social_schedule.import.hint")}</p>
        <div class="composer-row">
          <button class="btn btn-secondary btn-sm" disabled={busy} onclick={() => void chooseCsv()}>{$t("social_schedule.import.choose")}</button>
          <span class="path">{csvPath ? fileName(csvPath) : $t("social_schedule.import.none")}</span>
          {#if busy}<span class="spinner"></span>{/if}
        </div>
        {#if report}
          <p class="report" role="status">{$t("social_schedule.import.report", { valid: report.valid, rows: report.rows })}</p>
          {#if report.rejected.length}
            <h3 class="day">{$t("social_schedule.import.rejected")}</h3>
            <ul class="items">
              {#each report.rejected as r (r.row)}
                <li class="item">
                  <div class="item-when"><span class="tag tag-danger">{$t("social_schedule.import.row", { n: r.row })}</span></div>
                  <div class="item-body"><p class="item-error">{message(`${r.code}: ${JSON.stringify({ message: r.reason })}`)}</p></div>
                </li>
              {/each}
            </ul>
          {/if}
          {#if report.preview.length}
            <ul class="items">
              {#each report.preview as r (r.row)}
                <li class="item">
                  <div class="item-when"><span class="tag">{$t("social_schedule.import.row", { n: r.row })}</span><span class="item-time">{dayTime(r.at)}</span></div>
                  <div class="item-body"><p class="item-text">{r.repost ? $t("social_schedule.preview.reposting", { target: r.repost }) : r.text}</p></div>
                </li>
              {/each}
            </ul>
          {/if}
          <div class="composer-row end">
            <button class="btn btn-primary btn-sm" disabled={busy || report.valid === 0} onclick={() => void runImport(false)}>{$t("social_schedule.import.confirm", { n: report.valid })}</button>
          </div>
        {/if}
      </section>
    {:else if section === "feeds"}
      <section class="card">
        <h2>{$t("social_schedule.feeds.title")}</h2>
        <p class="hint">{$t("social_schedule.feeds.hint")}</p>
        {#if data?.feeds.length}
          <ul class="items">
            {#each data.feeds as f (f.id)}
              <li class="item">
                <div class="item-body">
                  <p class="item-text">{f.url}</p>
                  <div class="item-meta">
                    <span class="tag">{f.template}</span>
                    <span>{$t("social_schedule.feeds.stats", { posted: f.posted_this_month, cap: f.monthly_cap })}</span>
                    {#if f.skipped_over_cap}<span>{$t("social_schedule.feeds.skipped", { n: f.skipped_over_cap })}</span>{/if}
                    <span>{f.last_checked_at ? $t("social_schedule.feeds.checked", { when: dayTime(f.last_checked_at) }) : $t("social_schedule.feeds.waiting")}</span>
                  </div>
                  {#if f.last_error}<p class="item-error">{message(f.last_error)}</p>{/if}
                </div>
                <div class="item-actions">
                  <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => void act({ op: "remove", id: f.id }, "social_feeds")} aria-label={$t("social_schedule.feeds.remove")}><SocialIcon name="trash" /></button>
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          {@render states("social_schedule.feeds.empty")}
        {/if}
        <div class="composer-grid">
          <label class="field">
            <span class="field-label">{$t("social_schedule.feeds.url")}</span>
            <input class="input" type="url" bind:value={feedUrl} maxlength="1000" placeholder="https://…/feed.xml" />
          </label>
          <label class="field">
            <span class="field-label">{$t("social_schedule.feeds.template")}</span>
            <input class="input" bind:value={feedTemplate} maxlength="500" />
          </label>
          <label class="field">
            <span class="field-label">{$t("social_schedule.feeds.interval")}</span>
            <input class="input" type="number" min="5" max="1440" bind:value={feedInterval} />
          </label>
          <label class="field">
            <span class="field-label">{$t("social_schedule.feeds.cap")}</span>
            <input class="input" type="number" min="1" max="3000" bind:value={feedCap} />
          </label>
        </div>
        <div class="composer-row end">
          <button class="btn btn-primary btn-sm" disabled={busy || !feedUrl.trim()} onclick={() => void addFeed()}><SocialIcon name="plus" /> {$t("social_schedule.feeds.add")}</button>
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .sched {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .sched > .segmented {
    align-self: flex-start;
    flex-wrap: wrap;
  }

  .pill {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--fill-1);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .pill-alert {
    background: var(--danger);
    color: #fff;
  }

  .composer {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
    gap: var(--space-4);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .composer-form,
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .card {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .composer-head,
  .preview-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .part {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md, 10px);
    background: var(--fill-1);
  }

  .part textarea {
    resize: vertical;
    min-height: 44px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--text-base);
    outline: none;
  }

  .part:focus-within {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .part-foot,
  .composer-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .part-foot {
    justify-content: space-between;
    min-height: 24px;
  }

  .composer-row.end {
    justify-content: flex-end;
  }

  .composer-row .field {
    flex: 0 1 180px;
  }

  .composer-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-3);
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .count.over,
  .item-error {
    color: var(--danger);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .chip-x {
    margin-left: 4px;
    padding: 0 4px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .chip-x:focus-visible {
    outline: 2px solid var(--accent);
  }

  .when {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .when-option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    font-size: var(--text-sm);
  }

  .when-option small {
    display: block;
    color: var(--text-dim);
  }

  .when-option .input {
    flex: 0 1 220px;
  }

  .preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .preview-frame {
    width: 100%;
    max-width: 520px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--bg, var(--fill-1));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .preview-frame.mobile {
    max-width: 320px;
  }

  .preview-post {
    position: relative;
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr);
    gap: var(--space-2);
    padding-bottom: var(--space-3);
  }

  .preview-post.chained::before {
    content: "";
    position: absolute;
    left: 15px;
    top: 36px;
    bottom: 0;
    width: 2px;
    background: var(--content-border);
  }

  .preview-author {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    text-align: left;
  }

  .preview-author span,
  .preview-note,
  .hint,
  .more-note,
  .path {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .preview-text,
  .item-text {
    margin: 2px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  .preview-note,
  .hint,
  .more-note,
  .report {
    margin: 0;
    line-height: 1.5;
  }

  .preview-note {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .preview-quote {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-md, 10px);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-dim);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .day {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
  }

  .items {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: var(--space-3);
    align-items: start;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .card .item {
    background: var(--fill-1);
    box-shadow: none;
  }

  .item-when {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    min-width: 72px;
  }

  .item-time {
    font-size: var(--text-sm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .item-text {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .item-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: 6px;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .item-error {
    margin: 6px 0 0;
    font-size: var(--text-xs);
  }

  .item-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 2px;
  }

  .state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  @media (max-width: 860px) {
    .composer {
      grid-template-columns: minmax(0, 1fr);
    }

    .item {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
