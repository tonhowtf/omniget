<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar, fmtBytes } from "$lib/tube/format";
  import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { AutoRule, LocalSub, SbCategory, SbMode, StorageStats, TubeSettings, TubeStatus } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import { activePreset, presetDiff, presetPatch, PRESET_IDS, type PresetId } from "./presets";
  import RuleEditor from "./RuleEditor.svelte";
  import Row from "./Row.svelte";
  import Section from "./Section.svelte";
  import Seg from "./Seg.svelte";
  import Toggle from "./Toggle.svelte";
  import { newRule, parseWords, usageRatio } from "./rules";

  let { route }: { route: Extract<Route, { name: "settings" }> } = $props();

  const SECTIONS = [
    { id: "account", icon: "tube" },
    { id: "playback", icon: "queue" },
    { id: "sponsorblock", icon: "sponsorblock" },
    { id: "dearrow", icon: "dearrow" },
    { id: "distractions", icon: "shorts-off" },
    { id: "filters", icon: "history" },
    { id: "appearance", icon: "home" },
    { id: "downloads", icon: "autodownload" },
    { id: "import", icon: "import" },
  ] as const;

  const SB_CATS: SbCategory[] = ["sponsor", "selfpromo", "interaction", "intro", "outro", "preview", "hook", "music_offtopic", "filler", "poi_highlight", "exclusive_access"];
  const SB_MODES: SbMode[] = ["skip", "ask", "show", "off"];
  const QUALITIES: TubeSettings["default_quality"][] = ["auto", "2160", "1440", "1080", "720", "480", "360"];
  const PRESET_ICON: Record<PresetId, string> = { clean: "sparkles", focus: "eye-off", seeker: "search" };

  let p = $derived(tubePrefs());
  let preset = $derived(activePreset(p));
  let status = $state<TubeStatus | null>(null);
  let statusError = $state(false);
  let accountBusy = $state(false);
  let stats = $state<StorageStats | null>(null);
  let rules = $state<AutoRule[] | null>(null);
  let rulesError = $state<string | null>(null);
  let editing = $state<AutoRule | null>(null);
  let editingIsNew = $state(false);
  let confirmDelete = $state<string | null>(null);
  let checking = $state(false);
  let subs = $state<LocalSub[] | null>(null);
  let subsFilter = $state("");
  let wordInput = $state("");
  let wlInput = $state("");
  let activeSection = $state<string>("account");
  let root = $state<HTMLDivElement>();

  let logged = $derived(!!status?.account.logged_in);
  let usage = $derived(stats ? usageRatio(stats.total_bytes, p.storage_cap_gb) : 0);
  let subsShown = $derived.by(() => {
    const q = subsFilter.trim().toLocaleLowerCase();
    const list = subs ?? [];
    return q ? list.filter((s) => s.name.toLocaleLowerCase().includes(q)) : list;
  });
  let subName = $derived(new Map((subs ?? []).map((s) => [s.channel_id, s.name])));
  let readiness = $derived(status || statusError ? "ready" : "loading");

  async function set(patch: Partial<TubeSettings>) {
    try {
      await patchTubePrefs(patch);
    } catch {
      showToast("error", $t("tube.settings.save_failed") as string);
    }
  }

  async function loadStatus() {
    try {
      status = await api.status();
      statusError = false;
    } catch {
      statusError = true;
    }
  }

  async function loadStats() {
    stats = await api.storageStats().catch(() => null);
  }

  async function loadRules() {
    try {
      rules = await api.rulesList();
      rulesError = null;
    } catch (e) {
      rulesError = String(e);
      rules = [];
    }
  }

  async function loadSubs() {
    subs = await api.subsList().catch(() => []);
  }

  async function signIn() {
    accountBusy = true;
    try {
      await api.login();
      await loadStatus();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      accountBusy = false;
    }
  }

  async function signOut() {
    accountBusy = true;
    try {
      await api.logout();
      await loadStatus();
      showToast("success", $t("tube.settings.signed_out") as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      accountBusy = false;
    }
  }

  function setCat(cat: SbCategory, mode: SbMode) {
    void set({ sb_categories: { ...p.sb_categories, [cat]: mode } });
  }

  function addWords() {
    const words = parseWords(wordInput);
    if (!words.length) return;
    const have = new Set(p.blocked_words.map((w) => w.toLocaleLowerCase()));
    void set({ blocked_words: [...p.blocked_words, ...words.filter((w) => !have.has(w.toLocaleLowerCase()))] });
    wordInput = "";
  }

  function addWhitelist() {
    const ids = parseWords(wlInput).map((x) => x.replace(/^https?:\/\/(www\.)?youtube\.com\/channel\//, ""));
    if (!ids.length) return;
    void set({ sb_whitelist: [...new Set([...p.sb_whitelist, ...ids])] });
    wlInput = "";
  }

  function minutesField(e: Event, key: "min_duration_s" | "max_duration_s") {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    void set({ [key]: Number.isFinite(v) && v > 0 ? Math.round(v * 60) : 0 } as Partial<TubeSettings>);
  }

  async function chooseDownloadFolder() {
    const dir = await open({ directory: true, title: $t("tube.settings.download_folder") as string }).catch(() => null);
    if (typeof dir === "string") void set({ download_folder: dir });
  }

  function startNewRule() {
    editing = newRule("", []);
    editingIsNew = true;
  }

  async function saveRule(r: AutoRule) {
    try {
      const saved = await api.rulesSave(r);
      const list = rules ?? [];
      const i = list.findIndex((x) => x.id === saved.id);
      rules = i >= 0 ? list.map((x) => (x.id === saved.id ? saved : x)) : [...list, saved];
      editing = null;
      showToast("success", $t("tube.settings.rule_saved") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function toggleRule(r: AutoRule, enabled: boolean) {
    try {
      const saved = await api.rulesSave({ ...$state.snapshot(r), enabled } as AutoRule);
      rules = (rules ?? []).map((x) => (x.id === saved.id ? saved : x));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function deleteRule(id: string) {
    if (confirmDelete !== id) {
      confirmDelete = id;
      return;
    }
    try {
      await api.rulesRemove(id);
      rules = (rules ?? []).filter((x) => x.id !== id);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      confirmDelete = null;
    }
  }

  async function checkNow() {
    checking = true;
    try {
      const n = await api.autoCheckNow();
      showToast("success", n ? ($t("tube.settings.check_queued", { count: n }) as string) : ($t("tube.settings.check_nothing") as string));
      void loadStats();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      checking = false;
    }
  }

  async function updateSub(s: LocalSub, notify: boolean | null, auto: boolean | null) {
    try {
      await api.subsUpdate(s.channel_id, notify, auto);
      if (notify !== null) s.notify = notify;
      if (auto !== null) s.auto_download = auto;
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function ruleSummary(r: AutoRule): string {
    const parts: string[] = [];
    parts.push(r.channel_ids.length ? ($t("tube.settings.rule_n_channels", { count: r.channel_ids.length }) as string) : ($t("tube.settings.rule_all_channels") as string));
    parts.push(r.quality === "audio" ? ($t("tube.settings.q_audio") as string) : r.quality === "best" ? ($t("tube.settings.q_best") as string) : `${r.quality}p`);
    if (r.include_words.length) parts.push(`+ ${r.include_words.join(", ")}`);
    if (r.exclude_words.length) parts.push(`− ${r.exclude_words.join(", ")}`);
    if (r.max_age_days) parts.push($t("tube.settings.rule_age_days", { count: r.max_age_days }) as string);
    return parts.join(" · ");
  }

  function scrollTo(id: string) {
    activeSection = id;
    root?.querySelector(`#tube-set-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  onMount(() => {
    void loadStatus();
    void loadStats();
    void loadRules();
    void loadSubs();
    if (route.section) activeSection = route.section;
    if (route.section) void tick().then(() => root?.querySelector(`#tube-set-${route.section}`)?.scrollIntoView({ block: "start" }));
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) activeSection = (e.target as HTMLElement).dataset.section ?? activeSection;
      },
      { rootMargin: "-10% 0px -75% 0px" },
    );
    root?.querySelectorAll("[data-section]").forEach((el) => io.observe(el));
    return () => io.disconnect();
  });
</script>

<div class="ts" bind:this={root} data-agent-readiness={readiness} data-agent-surface="omnitube-settings">
  <nav class="ts-nav" aria-label={$t("tube.settings.title") as string}>
    <h1 class="ts-title">{$t("tube.settings.title")}</h1>
    <ul>
      {#each SECTIONS as s (s.id)}
        <li>
          <button class="ts-nav-item" class:on={activeSection === s.id} aria-current={activeSection === s.id ? "true" : undefined} onclick={() => scrollTo(s.id)}>
            <img src="/icons/tube/{s.icon}-32.webp" alt="" width="24" height="24" />
            <span>{$t(`tube.settings.sec_${s.id}`)}</span>
          </button>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="ts-body">
    <Section id="account" icon="tube" title={$t("tube.settings.sec_account") as string} desc={$t("tube.settings.sec_account_desc") as string}>
      <div class="ts-account">
        {#if !status && !statusError}
          <span class="yt-skel ts-av-skel"></span>
          <div class="ts-acc-text"><span class="yt-skel ts-line"></span><span class="yt-skel ts-line short"></span></div>
        {:else}
          {#if logged && status?.account.avatar}
            <img class="yt-avatar" src={bestAvatar(status.account.avatar, 56)} alt="" width="56" height="56" referrerpolicy="no-referrer" />
          {:else}
            <span class="ts-av-ph"><TIcon name="user" size={28} /></span>
          {/if}
          <div class="ts-acc-text">
            {#if logged}
              <strong>{status?.account.name ?? $t("tube.settings.account_signed_in")}</strong>
              <span>{[status?.account.handle, status?.account.email].filter(Boolean).join(" · ")}</span>
            {:else}
              <strong>{$t("tube.settings.account_signed_out")}</strong>
              <span>{$t("tube.settings.account_signed_out_desc")}</span>
            {/if}
          </div>
          {#if logged}
            <button class="yt-btn" onclick={signOut} disabled={accountBusy}><TIcon name="logout" size={18} />{$t("tube.common.sign_out")}</button>
          {:else}
            <button class="yt-btn yt-btn--outline" onclick={signIn} disabled={accountBusy}><TIcon name="login" size={18} />{$t("tube.common.sign_in")}</button>
          {/if}
        {/if}
      </div>
      <Row title={$t("tube.settings.use_account") as string} desc={$t("tube.settings.use_account_desc") as string}>
        <Toggle checked={p.use_account} label={$t("tube.settings.use_account") as string} onchange={(v) => set({ use_account: v })} />
      </Row>
      <Row title={$t("tube.settings.send_history") as string} desc={$t("tube.settings.send_history_desc") as string} dim={!logged}>
        <Toggle checked={p.send_history} label={$t("tube.settings.send_history") as string} onchange={(v) => set({ send_history: v })} />
      </Row>
      {#if status}
        <Row title={$t("tube.settings.tools") as string} desc={$t("tube.settings.tools_desc") as string}>
          <span class="ts-pill" class:ok={status.ytdlp}><TIcon name={status.ytdlp ? "check" : "alert"} size={14} />yt-dlp</span>
          <span class="ts-pill" class:ok={status.ffmpeg}><TIcon name={status.ffmpeg ? "check" : "alert"} size={14} />FFmpeg</span>
        </Row>
      {/if}
    </Section>

    <Section id="playback" icon="queue" title={$t("tube.settings.sec_playback") as string} desc={$t("tube.settings.sec_playback_desc") as string}>
      <Row title={$t("tube.settings.default_quality") as string} desc={$t("tube.settings.default_quality_desc") as string}>
        <select class="ts-select" value={p.default_quality} onchange={(e) => set({ default_quality: e.currentTarget.value as TubeSettings["default_quality"] })} aria-label={$t("tube.settings.default_quality") as string}>
          {#each QUALITIES as q (q)}
            <option value={q}>{q === "auto" ? $t("tube.settings.q_auto") : q === "2160" ? "2160p (4K)" : q === "1440" ? "1440p (2K)" : `${q}p`}</option>
          {/each}
        </select>
      </Row>
      <Row title={$t("tube.settings.prefer_codec") as string} desc={$t("tube.settings.prefer_codec_desc") as string}>
        <Seg
          value={p.prefer_codec}
          label={$t("tube.settings.prefer_codec") as string}
          options={[
            { value: "auto", label: $t("tube.settings.q_auto") as string },
            { value: "avc1", label: "H.264" },
            { value: "vp9", label: "VP9" },
            { value: "av01", label: "AV1" },
          ]}
          onchange={(v) => set({ prefer_codec: v })}
        />
      </Row>
      <Row title={$t("tube.settings.audio_quality") as string} desc={$t("tube.settings.audio_quality_desc") as string}>
        <Seg
          value={p.audio_quality}
          label={$t("tube.settings.audio_quality") as string}
          options={[
            { value: "premium", label: $t("tube.settings.aq_premium") as string },
            { value: "high", label: $t("tube.settings.aq_high") as string },
            { value: "normal", label: $t("tube.settings.aq_normal") as string },
            { value: "low", label: $t("tube.settings.aq_low") as string },
          ]}
          onchange={(v) => set({ audio_quality: v })}
        />
      </Row>
      <Row title={$t("tube.settings.autoplay_next") as string} desc={$t("tube.settings.autoplay_next_desc") as string}>
        <Toggle checked={p.autoplay_next} label={$t("tube.settings.autoplay_next") as string} onchange={(v) => set({ autoplay_next: v })} />
      </Row>
      <Row title={$t("tube.settings.autoplay_on_open") as string} desc={$t("tube.settings.autoplay_on_open_desc") as string}>
        <Toggle checked={p.autoplay_on_open} label={$t("tube.settings.autoplay_on_open") as string} onchange={(v) => set({ autoplay_on_open: v })} />
      </Row>
      <Row title={$t("tube.settings.preview_on_hover") as string} desc={$t("tube.settings.preview_on_hover_desc") as string}>
        <Toggle checked={p.preview_on_hover} label={$t("tube.settings.preview_on_hover") as string} onchange={(v) => set({ preview_on_hover: v })} />
      </Row>
      <Row title={$t("tube.settings.normalize_volume") as string} desc={$t("tube.settings.normalize_volume_desc") as string}>
        <Toggle checked={p.normalize_volume} label={$t("tube.settings.normalize_volume") as string} onchange={(v) => set({ normalize_volume: v })} />
      </Row>
      <Row title={$t("tube.settings.endless_radio") as string} desc={$t("tube.settings.endless_radio_desc") as string}>
        <Toggle checked={p.endless_radio} label={$t("tube.settings.endless_radio") as string} onchange={(v) => set({ endless_radio: v })} />
      </Row>
      <Row title={$t("tube.settings.lyrics_provider") as string} desc={$t("tube.settings.lyrics_provider_desc") as string}>
        <Seg
          value={p.lyrics_provider}
          label={$t("tube.settings.lyrics_provider") as string}
          options={[
            { value: "auto", label: $t("tube.settings.q_auto") as string },
            { value: "youtube", label: $t("tube.settings.lyrics_youtube") as string },
            { value: "lrclib", label: "LRCLIB" },
            { value: "off", label: $t("tube.settings.off") as string },
          ]}
          onchange={(v) => set({ lyrics_provider: v })}
        />
      </Row>
    </Section>

    <Section id="sponsorblock" icon="sponsorblock" title="SponsorBlock" desc={$t("tube.settings.sec_sponsorblock_desc") as string}>
      <Row title={$t("tube.settings.sb_enabled") as string} desc={$t("tube.settings.sb_enabled_desc") as string}>
        <Toggle checked={p.sponsorblock} label={$t("tube.settings.sb_enabled") as string} onchange={(v) => set({ sponsorblock: v })} />
      </Row>
      <div class="ts-cats" class:off={!p.sponsorblock}>
        {#each SB_CATS as cat (cat)}
          <div class="ts-cat">
            <span class="ts-cat-dot" style:background="var(--sb-{cat})"></span>
            <div class="ts-cat-text">
              <span class="ts-cat-name">{$t(`tube.settings.sb_cat_${cat}`)}</span>
              <span class="ts-cat-desc">{$t(`tube.settings.sb_cat_${cat}_desc`)}</span>
            </div>
            <Seg
              value={p.sb_categories[cat] ?? "off"}
              label={$t(`tube.settings.sb_cat_${cat}`) as string}
              disabled={!p.sponsorblock}
              options={SB_MODES.map((m) => ({ value: m, label: $t(`tube.settings.sb_mode_${m}`) as string }))}
              onchange={(v) => setCat(cat, v)}
            />
          </div>
        {/each}
      </div>
      <Row title={$t("tube.settings.sb_download") as string} desc={$t("tube.settings.sb_download_desc") as string}>
        <Seg
          value={p.sb_download}
          label={$t("tube.settings.sb_download") as string}
          options={(["off", "mark", "remove"] as const).map((m) => ({ value: m, label: $t(`tube.settings.sbdl_${m}`) as string }))}
          onchange={(v) => set({ sb_download: v })}
        />
      </Row>
      <Row title={$t("tube.settings.sb_whitelist") as string} desc={$t("tube.settings.sb_whitelist_desc") as string} stack>
        {#if p.sb_whitelist.length}
          <div class="ts-chips">
            {#each p.sb_whitelist as id (id)}
              <span class="ts-chip">
                {subName.get(id) ?? id}
                <button onclick={() => set({ sb_whitelist: p.sb_whitelist.filter((x) => x !== id) })} aria-label={$t("tube.settings.remove_named", { name: subName.get(id) ?? id }) as string}><TIcon name="close" size={14} /></button>
              </span>
            {/each}
          </div>
        {/if}
        <form class="ts-add" onsubmit={(e) => (e.preventDefault(), addWhitelist())}>
          <input bind:value={wlInput} placeholder={$t("tube.settings.sb_whitelist_ph") as string} aria-label={$t("tube.settings.sb_whitelist") as string} />
          <button class="yt-btn yt-btn--s" type="submit" disabled={!wlInput.trim()}>{$t("tube.settings.add")}</button>
        </form>
      </Row>
      <Row title={$t("tube.settings.sb_server") as string} desc={$t("tube.settings.sb_server_desc") as string}>
        <input class="ts-input" value={p.sb_server} onchange={(e) => set({ sb_server: e.currentTarget.value.trim() || "https://sponsor.ajay.app" })} aria-label={$t("tube.settings.sb_server") as string} spellcheck="false" />
        {#if p.sb_server !== "https://sponsor.ajay.app"}
          <button class="yt-btn yt-btn--s yt-btn--ghost" onclick={() => set({ sb_server: "https://sponsor.ajay.app" })}>{$t("tube.settings.reset")}</button>
        {/if}
      </Row>
    </Section>

    <Section id="dearrow" icon="dearrow" title={$t("tube.settings.sec_dearrow") as string} desc={$t("tube.settings.sec_dearrow_desc") as string}>
      <Row title={$t("tube.settings.dearrow") as string} desc={$t("tube.settings.dearrow_desc") as string}>
        <Toggle checked={p.dearrow} label={$t("tube.settings.dearrow") as string} onchange={(v) => set({ dearrow: v })} />
      </Row>
      <Row title={$t("tube.settings.dearrow_thumbs") as string} desc={$t("tube.settings.dearrow_thumbs_desc") as string} dim={!p.dearrow}>
        <Toggle checked={p.dearrow_thumbs} disabled={!p.dearrow} label={$t("tube.settings.dearrow_thumbs") as string} onchange={(v) => set({ dearrow_thumbs: v })} />
      </Row>
      <Row title={$t("tube.settings.ryd") as string} desc={$t("tube.settings.ryd_desc") as string}>
        <Toggle checked={p.ryd} label={$t("tube.settings.ryd") as string} onchange={(v) => set({ ryd: v })} />
      </Row>
    </Section>

    <Section id="distractions" icon="shorts-off" title={$t("tube.settings.sec_distractions") as string} desc={$t("tube.settings.sec_distractions_desc") as string}>
      <div class="ts-presets" role="radiogroup" aria-label={$t("tube.settings.presets") as string}>
        {#each PRESET_IDS as id (id)}
          {@const diff = presetDiff(p, id)}
          <button class="ts-preset" class:on={preset === id} role="radio" aria-checked={preset === id} onclick={() => set(presetPatch(id))}>
            <span class="ts-preset-icon"><TIcon name={PRESET_ICON[id]} size={20} /></span>
            <span class="ts-preset-name">{$t(`tube.settings.preset_${id}`)}</span>
            <span class="ts-preset-desc">{$t(`tube.settings.preset_${id}_desc`)}</span>
            <span class="ts-preset-foot">
              {#if preset === id}<TIcon name="check" size={14} />{$t("tube.settings.preset_active")}{:else}{$t("tube.settings.preset_changes", { count: diff })}{/if}
            </span>
          </button>
        {/each}
      </div>
      {#if !preset}<p class="ts-note">{$t("tube.settings.preset_custom")}</p>{/if}

      <h3 class="ts-sub">{$t("tube.settings.group_content")}</h3>
      {#each ["hide_shorts", "hide_live", "hide_upcoming", "hide_members", "hide_mixes", "hide_playables", "hide_posts"] as const as k (k)}
        <Row title={$t(`tube.settings.${k}`) as string} desc={$t(`tube.settings.${k}_desc`) as string}>
          <Toggle checked={p[k]} label={$t(`tube.settings.${k}`) as string} onchange={(v) => set({ [k]: v })} />
        </Row>
      {/each}
      <Row title={$t("tube.settings.hide_watched") as string} desc={$t("tube.settings.hide_watched_desc") as string}>
        <Toggle checked={p.hide_watched} label={$t("tube.settings.hide_watched") as string} onchange={(v) => set({ hide_watched: v })} />
      </Row>
      <Row title={$t("tube.settings.watched_threshold") as string} desc={$t("tube.settings.watched_threshold_desc", { count: p.watched_threshold }) as string} dim={!p.hide_watched}>
        <input
          class="ts-range"
          type="range"
          min="10"
          max="100"
          step="5"
          value={p.watched_threshold}
          disabled={!p.hide_watched}
          onchange={(e) => set({ watched_threshold: Number(e.currentTarget.value) })}
          aria-label={$t("tube.settings.watched_threshold") as string}
        />
        <span class="ts-val">{p.watched_threshold}%</span>
      </Row>

      <h3 class="ts-sub">{$t("tube.settings.group_pages")}</h3>
      {#each ["hide_home_feed", "hide_related", "hide_comments", "hide_chips", "hide_metrics"] as const as k (k)}
        <Row title={$t(`tube.settings.${k}`) as string} desc={$t(`tube.settings.${k}_desc`) as string}>
          <Toggle checked={p[k]} label={$t(`tube.settings.${k}`) as string} onchange={(v) => set({ [k]: v })} />
        </Row>
      {/each}
      <Row title={$t("tube.settings.infinite_scroll") as string} desc={$t("tube.settings.infinite_scroll_desc") as string}>
        <Toggle checked={p.infinite_scroll} label={$t("tube.settings.infinite_scroll") as string} onchange={(v) => set({ infinite_scroll: v })} />
      </Row>
    </Section>

    <Section id="filters" icon="history" title={$t("tube.settings.sec_filters") as string} desc={$t("tube.settings.sec_filters_desc") as string}>
      <Row title={$t("tube.settings.blocked_channels") as string} desc={$t("tube.settings.blocked_channels_desc") as string} stack>
        {#if p.blocked_channels.length}
          <ul class="ts-blocked">
            {#each p.blocked_channels as c (c.id)}
              <li>
                <TIcon name="ban" size={16} />
                <span class="yt-clamp-1">{c.name || c.id}</span>
                <button class="yt-btn yt-btn--s yt-btn--ghost" onclick={() => set({ blocked_channels: p.blocked_channels.filter((x) => x.id !== c.id) })}>{$t("tube.settings.unblock")}</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="ts-note">{$t("tube.settings.blocked_channels_empty")}</p>
        {/if}
      </Row>
      <Row title={$t("tube.settings.blocked_words") as string} desc={$t("tube.settings.blocked_words_desc") as string} stack>
        {#if p.blocked_words.length}
          <div class="ts-chips">
            {#each p.blocked_words as w (w)}
              <span class="ts-chip">
                {w}
                <button onclick={() => set({ blocked_words: p.blocked_words.filter((x) => x !== w) })} aria-label={$t("tube.settings.remove_named", { name: w }) as string}><TIcon name="close" size={14} /></button>
              </span>
            {/each}
          </div>
        {/if}
        <form class="ts-add" onsubmit={(e) => (e.preventDefault(), addWords())}>
          <input bind:value={wordInput} placeholder={$t("tube.settings.blocked_words_ph") as string} aria-label={$t("tube.settings.blocked_words") as string} />
          <button class="yt-btn yt-btn--s" type="submit" disabled={!wordInput.trim()}>{$t("tube.settings.add")}</button>
        </form>
      </Row>
      <Row title={$t("tube.settings.whole_word") as string} desc={$t("tube.settings.whole_word_desc") as string}>
        <Toggle checked={p.whole_word} label={$t("tube.settings.whole_word") as string} onchange={(v) => set({ whole_word: v })} />
      </Row>
      <Row title={$t("tube.settings.min_duration") as string} desc={$t("tube.settings.min_duration_desc") as string}>
        <input class="ts-input ts-num" type="number" min="0" step="1" value={p.min_duration_s ? Math.round(p.min_duration_s / 60) : ""} placeholder="0" onchange={(e) => minutesField(e, "min_duration_s")} aria-label={$t("tube.settings.min_duration") as string} />
        <span class="ts-val">{$t("tube.settings.minutes")}</span>
      </Row>
      <Row title={$t("tube.settings.max_duration") as string} desc={$t("tube.settings.max_duration_desc") as string}>
        <input class="ts-input ts-num" type="number" min="0" step="1" value={p.max_duration_s ? Math.round(p.max_duration_s / 60) : ""} placeholder="∞" onchange={(e) => minutesField(e, "max_duration_s")} aria-label={$t("tube.settings.max_duration") as string} />
        <span class="ts-val">{$t("tube.settings.minutes")}</span>
      </Row>
    </Section>

    <Section id="appearance" icon="home" title={$t("tube.settings.sec_appearance") as string} desc={$t("tube.settings.sec_appearance_desc") as string}>
      <Row title={$t("tube.settings.look") as string} desc={$t("tube.settings.look_desc") as string}>
        <Seg
          value={p.look}
          label={$t("tube.settings.look") as string}
          options={[
            { value: "classic", label: $t("tube.settings.look_classic") as string },
            { value: "app", label: $t("tube.settings.look_app") as string },
          ]}
          onchange={(v) => set({ look: v })}
        />
      </Row>
      <Row title={$t("tube.settings.grid_columns") as string} desc={$t("tube.settings.grid_columns_desc") as string}>
        <select class="ts-select" value={String(p.grid_columns)} onchange={(e) => set({ grid_columns: Number(e.currentTarget.value) })} aria-label={$t("tube.settings.grid_columns") as string}>
          <option value="0">{$t("tube.settings.q_auto")}</option>
          {#each [1, 2, 3, 4, 5, 6] as n (n)}<option value={String(n)}>{n}</option>{/each}
        </select>
      </Row>
      <Row title={$t("tube.settings.start_face") as string} desc={$t("tube.settings.start_face_desc") as string}>
        <Seg
          value={p.start_face}
          label={$t("tube.settings.start_face") as string}
          options={[
            { value: "video", label: $t("tube.settings.face_video") as string },
            { value: "music", label: $t("tube.settings.face_music") as string },
          ]}
          onchange={(v) => set({ start_face: v })}
        />
      </Row>
      <Row title={$t("tube.settings.start_page") as string} desc={$t("tube.settings.start_page_desc") as string}>
        <Seg
          value={p.start_page}
          label={$t("tube.settings.start_page") as string}
          options={[
            { value: "home", label: $t("tube.settings.page_home") as string },
            { value: "subscriptions", label: $t("tube.settings.page_subscriptions") as string },
            { value: "library", label: $t("tube.settings.page_library") as string },
          ]}
          onchange={(v) => set({ start_page: v })}
        />
      </Row>
    </Section>

    <Section id="downloads" icon="autodownload" title={$t("tube.settings.sec_downloads") as string} desc={$t("tube.settings.sec_downloads_desc") as string}>
      <Row title={$t("tube.settings.download_folder") as string} desc={p.download_folder || ($t("tube.settings.download_folder_default") as string)}>
        <button class="yt-btn yt-btn--s" onclick={chooseDownloadFolder}><TIcon name="folder" size={16} />{$t("tube.settings.choose")}</button>
        {#if p.download_folder}<button class="yt-btn yt-btn--s yt-btn--ghost" onclick={() => set({ download_folder: null })}>{$t("tube.settings.reset")}</button>{/if}
      </Row>
      <Row title={$t("tube.settings.storage_cap") as string} desc={$t("tube.settings.storage_cap_desc") as string} stack>
        <div class="ts-storage">
          <div class="ts-bar" class:warn={usage > 0.9} role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(usage * 100)} aria-label={$t("tube.settings.storage_used") as string}>
            <i style:width="{p.storage_cap_gb > 0 ? usage * 100 : 0}%"></i>
          </div>
          <div class="ts-storage-line">
            {#if stats}
              <span>
                {p.storage_cap_gb > 0
                  ? $t("tube.settings.storage_of", { used: fmtBytes(stats.total_bytes), cap: `${p.storage_cap_gb} GB` })
                  : $t("tube.settings.storage_used_nocap", { used: fmtBytes(stats.total_bytes) })}
                · {$t("tube.settings.storage_files", { count: stats.files })}
                {#if stats.free_bytes != null}· {$t("tube.settings.storage_free", { free: fmtBytes(stats.free_bytes) })}{/if}
              </span>
            {:else}
              <span class="yt-skel ts-line"></span>
            {/if}
            <label class="ts-cap">
              <input class="ts-input ts-num" type="number" min="0" step="1" value={p.storage_cap_gb || ""} placeholder="∞" onchange={(e) => set({ storage_cap_gb: Math.max(0, Number(e.currentTarget.value) || 0) })} aria-label={$t("tube.settings.storage_cap") as string} />
              GB
            </label>
          </div>
        </div>
      </Row>

      <div class="ts-rules-head">
        <div>
          <h3 class="ts-sub flush">{$t("tube.settings.rules")}</h3>
          <p class="ts-note">{$t("tube.settings.rules_desc")}</p>
        </div>
        <div class="ts-rules-actions">
          <button class="yt-btn yt-btn--s" onclick={checkNow} disabled={checking}><TIcon name={checking ? "loader" : "rss"} size={16} />{$t("tube.settings.check_now")}</button>
          <button class="yt-btn yt-btn--s yt-btn--inverted" onclick={startNewRule} disabled={!!editing}><TIcon name="plus" size={16} />{$t("tube.settings.new_rule")}</button>
        </div>
      </div>
      {#if editing && editingIsNew}
        <div class="ts-rule-edit">
          <RuleEditor initial={editing} subs={subs ?? []} onsave={saveRule} oncancel={() => (editing = null)} />
        </div>
      {/if}
      {#if rules === null}
        <span class="yt-skel ts-rule-skel"></span>
      {:else if rulesError}
        <p class="ts-err">{rulesError}</p>
      {:else if !rules.length && !editing}
        <div class="ts-empty">
          <img src="/icons/tube/autodownload-64.webp" alt="" width="48" height="48" />
          <span>{$t("tube.settings.rules_empty")}</span>
        </div>
      {:else}
        <ul class="ts-rules">
          {#each rules as r (r.id)}
            <li class="ts-rule" class:off={!r.enabled}>
              {#if editing && !editingIsNew && editing.id === r.id}
                <RuleEditor initial={editing} subs={subs ?? []} onsave={saveRule} oncancel={() => (editing = null)} />
              {:else}
                <Toggle checked={r.enabled} label={$t("tube.settings.rule_enabled") as string} onchange={(v) => toggleRule(r, v)} />
                <div class="ts-rule-text">
                  <strong class="yt-clamp-1">{r.name}</strong>
                  <span class="yt-clamp-1">{ruleSummary(r)}</span>
                </div>
                <button class="yt-btn yt-btn--s yt-btn--ghost" onclick={() => ((editing = $state.snapshot(r) as AutoRule), (editingIsNew = false))} disabled={!!editing}><TIcon name="edit" size={16} />{$t("tube.settings.edit")}</button>
                <button class="yt-btn yt-btn--s yt-btn--ghost ts-danger" onclick={() => deleteRule(r.id)} onblur={() => confirmDelete === r.id && (confirmDelete = null)}>
                  <TIcon name="trash" size={16} />{confirmDelete === r.id ? $t("tube.settings.confirm_delete") : $t("tube.settings.delete")}
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <h3 class="ts-sub">{$t("tube.settings.local_subs")}</h3>
      <p class="ts-note">{$t("tube.settings.local_subs_desc")}</p>
      {#if subs === null}
        <span class="yt-skel ts-rule-skel"></span>
      {:else if !subs.length}
        <div class="ts-empty">
          <img src="/icons/tube/subscriptions-64.webp" alt="" width="48" height="48" />
          <span>{$t("tube.settings.local_subs_empty")}</span>
          <button class="yt-btn yt-btn--s" onclick={() => go({ name: "import" })}>{$t("tube.settings.import_open")}</button>
        </div>
      {:else}
        {#if subs.length > 8}
          <input class="ts-input ts-filter" bind:value={subsFilter} placeholder={$t("tube.settings.local_subs_filter") as string} aria-label={$t("tube.settings.local_subs_filter") as string} />
        {/if}
        <div class="ts-subs-head" aria-hidden="true">
          <span></span><span>{$t("tube.settings.notify")}</span><span>{$t("tube.settings.auto_download")}</span>
        </div>
        <ul class="ts-subs yt-scroll">
          {#each subsShown as s (s.channel_id)}
            <li>
              <button class="ts-sub-ch" onclick={() => go({ name: "channel", id: s.channel_id })}>
                {#if s.avatar}<img class="yt-avatar" src={bestAvatar(s.avatar, 32)} alt="" width="32" height="32" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="ts-av-ph small"><TIcon name="user" size={16} /></span>{/if}
                <span class="yt-clamp-1">{s.name}</span>
              </button>
              <Toggle checked={s.notify} label={$t("tube.settings.notify_named", { name: s.name }) as string} onchange={(v) => updateSub(s, v, null)} />
              <Toggle checked={s.auto_download} label={$t("tube.settings.auto_download_named", { name: s.name }) as string} onchange={(v) => updateSub(s, null, v)} />
            </li>
          {/each}
        </ul>
      {/if}
    </Section>

    <Section id="import" icon="import" title={$t("tube.settings.sec_import") as string} desc={$t("tube.settings.sec_import_desc") as string}>
      <Row title={$t("tube.settings.import_row") as string} desc={$t("tube.settings.import_row_desc") as string}>
        <button class="yt-btn yt-btn--inverted" onclick={() => go({ name: "import" })}><TIcon name="import" size={18} />{$t("tube.settings.import_open")}</button>
      </Row>
    </Section>
  </div>
</div>

<style>
  .ts {
    display: grid;
    grid-template-columns: 220px minmax(0, 820px);
    justify-content: center;
    gap: 40px;
    padding: 24px 24px 64px;
    background: var(--yt-bg);
    color: var(--yt-text);
    min-height: 100%;
  }
  .ts-nav { position: sticky; top: 16px; align-self: start; }
  .ts-title { margin: 0 0 16px 12px; font: var(--yt-type-headline-m); }
  .ts-nav ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .ts-nav-item {
    display: flex; align-items: center; gap: 12px; width: 100%;
    height: 40px; padding: 0 12px; border: 0; border-radius: 10px;
    font: 400 14px/20px var(--yt-font); color: var(--yt-text); text-align: left;
    background: transparent; cursor: pointer;
    transition: background var(--yt-dur-fast);
  }
  .ts-nav-item img { width: 24px; height: 24px; flex: none; }
  .ts-nav-item:hover { background: var(--yt-hover); }
  .ts-nav-item.on { background: var(--yt-tonal); font-weight: 500; }
  .ts-nav-item:focus-visible { outline: 2px solid var(--yt-link); outline-offset: -2px; }
  .ts-body { min-width: 0; }

  .ts-account { display: flex; align-items: center; gap: 16px; padding: 16px 0; border-bottom: 1px solid var(--yt-divider); }
  .ts-account img { width: 56px; height: 56px; }
  .ts-av-ph { display: grid; place-items: center; width: 56px; height: 56px; border-radius: 50%; background: var(--yt-tonal); color: var(--yt-text-2); flex: none; }
  .ts-av-ph.small { width: 32px; height: 32px; }
  .ts-av-skel { width: 56px; height: 56px; border-radius: 50%; flex: none; }
  .ts-acc-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .ts-acc-text strong { font: 500 16px/22px var(--yt-font); }
  .ts-acc-text span { font: var(--yt-type-meta); color: var(--yt-text-2); }
  .ts-line { display: block; height: 14px; width: 60%; border-radius: 4px; }
  .ts-line.short { width: 35%; }
  .ts-pill {
    display: inline-flex; align-items: center; gap: 4px; height: 24px; padding: 0 10px; border-radius: 999px;
    font: 500 12px/16px var(--yt-font); color: var(--yt-error); background: color-mix(in srgb, var(--yt-error) 14%, transparent);
  }
  .ts-pill.ok { color: var(--yt-green); background: color-mix(in srgb, var(--yt-green) 14%, transparent); }

  .ts-select, .ts-input {
    height: 36px; padding: 0 12px; border-radius: 8px;
    border: 1px solid var(--yt-outline); background: var(--yt-bg); color: var(--yt-text);
    font: var(--yt-type-meta);
  }
  .ts-select { padding-right: 28px; cursor: pointer; }
  .ts-input { width: 260px; max-width: 100%; }
  .ts-num { width: 88px; }
  .ts-select:focus, .ts-input:focus { outline: 2px solid var(--yt-link); outline-offset: -1px; border-color: transparent; }
  .ts-val { font: var(--yt-type-meta); color: var(--yt-text-2); min-width: 40px; }
  .ts-range { width: 180px; accent-color: var(--yt-accent); }

  .ts-cats { display: flex; flex-direction: column; border-top: 1px solid var(--yt-divider); padding: 4px 0; }
  .ts-cats.off { opacity: .55; }
  .ts-cat { display: flex; align-items: center; gap: 12px; padding: 10px 0; }
  .ts-cat-dot { width: 12px; height: 12px; border-radius: 50%; flex: none; box-shadow: 0 0 0 2px color-mix(in srgb, var(--yt-text) 12%, transparent); }
  .ts-cat-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .ts-cat-name { font: var(--yt-type-body-strong); }
  .ts-cat-desc { font: 400 12px/16px var(--yt-font); color: var(--yt-text-2); }

  .ts-chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .ts-chip {
    display: inline-flex; align-items: center; gap: 4px; height: 28px; padding: 0 4px 0 12px; border-radius: 8px;
    font: 500 13px/18px var(--yt-font); background: var(--yt-tonal);
  }
  .ts-chip button { display: grid; place-items: center; width: 22px; height: 22px; border: 0; border-radius: 50%; background: transparent; color: var(--yt-text-2); cursor: pointer; }
  .ts-chip button:hover { background: var(--yt-tonal-hover); color: var(--yt-text); }
  .ts-chip button:focus-visible { outline: 2px solid var(--yt-link); }
  .ts-add { display: flex; gap: 8px; width: 100%; }
  .ts-add input {
    flex: 1; min-width: 0; height: 32px; padding: 0 12px; border-radius: 8px;
    border: 1px solid var(--yt-outline); background: var(--yt-bg); color: var(--yt-text); font: var(--yt-type-meta);
  }
  .ts-add input:focus { outline: 2px solid var(--yt-link); outline-offset: -1px; border-color: transparent; }
  .ts-note { margin: 4px 0 8px; font: 400 13px/18px var(--yt-font); color: var(--yt-text-2); }
  .ts-err { margin: 8px 0; font: 500 13px/18px var(--yt-font); color: var(--yt-error); }
  .ts-sub { margin: 20px 0 4px; font: 500 12px/16px var(--yt-font); letter-spacing: .06em; text-transform: uppercase; color: var(--yt-text-2); }
  .ts-sub.flush { margin-top: 0; }

  .ts-presets { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; padding: 16px 0 8px; }
  .ts-preset {
    display: flex; flex-direction: column; align-items: flex-start; gap: 6px; text-align: left;
    padding: 14px; border-radius: 12px; border: 1px solid var(--yt-divider);
    background: var(--yt-bg); color: var(--yt-text); cursor: pointer;
    transition: border-color var(--yt-dur-base), background var(--yt-dur-base), transform var(--yt-dur-fast);
  }
  .ts-preset:hover { background: var(--yt-hover); border-color: var(--yt-outline); }
  .ts-preset:active { transform: scale(.98); }
  .ts-preset.on { border-color: var(--yt-accent); background: color-mix(in srgb, var(--yt-accent) 8%, var(--yt-bg)); }
  .ts-preset:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .ts-preset-icon { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; background: var(--yt-tonal); }
  .ts-preset.on .ts-preset-icon { background: var(--yt-accent); color: var(--yt-on-accent); }
  .ts-preset-name { font: 500 15px/20px var(--yt-font); }
  .ts-preset-desc { font: 400 12px/17px var(--yt-font); color: var(--yt-text-2); flex: 1; }
  .ts-preset-foot { display: inline-flex; align-items: center; gap: 4px; font: 500 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .ts-preset.on .ts-preset-foot { color: var(--yt-accent); }

  .ts-blocked { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .ts-blocked li { display: flex; align-items: center; gap: 10px; padding: 4px 0; color: var(--yt-text); }
  .ts-blocked li > span { flex: 1; min-width: 0; }

  .ts-storage { width: 100%; display: flex; flex-direction: column; gap: 8px; }
  .ts-bar { height: 8px; border-radius: 999px; background: var(--yt-tonal); overflow: hidden; }
  .ts-bar i { display: block; height: 100%; border-radius: inherit; background: var(--yt-accent); transition: width var(--yt-dur-preview) var(--yt-ease-standard); }
  .ts-bar.warn i { background: var(--yt-error); }
  .ts-storage-line { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; font: var(--yt-type-meta); color: var(--yt-text-2); }
  .ts-storage-line .ts-line { width: 220px; }
  .ts-cap { display: inline-flex; align-items: center; gap: 8px; }

  .ts-rules-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; padding-top: 16px; border-top: 1px solid var(--yt-divider); flex-wrap: wrap; }
  .ts-rules-actions { display: flex; gap: 8px; }
  .ts-rule-edit { border-top: 1px solid var(--yt-divider); }
  .ts-rules { list-style: none; margin: 8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .ts-rule { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: 10px; background: var(--yt-tonal); }
  .ts-rule:has(:global(.re)) { display: block; background: transparent; padding: 0; border-top: 1px solid var(--yt-divider); border-radius: 0; }
  .ts-rule.off .ts-rule-text { opacity: .6; }
  .ts-rule-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .ts-rule-text strong { font: var(--yt-type-body-strong); }
  .ts-rule-text span { font: 400 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .ts-danger:hover { color: var(--yt-error); }
  .ts-rule-skel { display: block; height: 56px; margin: 8px 0; border-radius: 10px; }
  .ts-empty { display: flex; align-items: center; gap: 12px; padding: 12px 0; font: var(--yt-type-meta); color: var(--yt-text-2); flex-wrap: wrap; }

  .ts-filter { width: 100%; margin-bottom: 8px; }
  .ts-subs-head, .ts-subs li { display: grid; grid-template-columns: minmax(0, 1fr) 96px 96px; align-items: center; justify-items: center; gap: 8px; }
  .ts-subs-head { font: 500 12px/16px var(--yt-font); color: var(--yt-text-2); padding: 0 4px 6px; }
  .ts-subs { list-style: none; margin: 0 0 12px; padding: 0; max-height: 420px; }
  .ts-subs li { padding: 6px 4px; border-top: 1px solid var(--yt-divider); }
  .ts-sub-ch {
    justify-self: stretch; display: flex; align-items: center; gap: 10px; min-width: 0;
    border: 0; padding: 0; background: none; color: var(--yt-text); font: var(--yt-type-body-strong); text-align: left; cursor: pointer;
  }
  .ts-sub-ch img { width: 32px; height: 32px; }
  .ts-sub-ch:hover span { text-decoration: underline; }
  .ts-sub-ch:focus-visible { outline: 2px solid var(--yt-link); border-radius: 4px; }

  @container omnitube (max-width: 980px) {
    .ts { grid-template-columns: minmax(0, 1fr); gap: 16px; padding: 16px 16px 48px; }
    .ts-nav { position: sticky; top: 0; z-index: 2; margin: -16px -16px 0; padding: 12px 16px 8px; background: var(--yt-frosted); backdrop-filter: blur(12px); }
    .ts-title { margin: 0 0 8px; font: var(--yt-type-headline-s); }
    .ts-nav ul { flex-direction: row; overflow-x: auto; scrollbar-width: none; gap: 6px; }
    .ts-nav-item { width: auto; height: 32px; white-space: nowrap; background: var(--yt-tonal); border-radius: 8px; }
    .ts-nav-item img { width: 20px; height: 20px; }
    .ts-nav-item.on { background: var(--yt-bg-inverted); color: var(--yt-on-inverted); }
    .ts-presets { grid-template-columns: 1fr; }
  }
  @container omnitube (max-width: 560px) {
    .ts-subs-head, .ts-subs li { grid-template-columns: minmax(0, 1fr) 56px 56px; }
    .ts-cat { flex-wrap: wrap; }
  }
</style>
