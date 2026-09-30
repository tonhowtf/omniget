<script lang="ts">
  // Auto-download rule editor with a live preview of what the rule would
  // queue now (tube_rules_preview runs the same code as the poller).
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar, fmtDuration, thumbUrl } from "$lib/tube/format";
  import type { AutoRule, Item, LocalSub } from "$lib/tube/types";
  import TIcon from "../../TIcon.svelte";
  import Row from "./Row.svelte";
  import Seg from "./Seg.svelte";
  import Toggle from "./Toggle.svelte";
  import { cleanRule, minutesToSeconds, parseWords, RULE_QUALITIES, RULE_SB, ruleProblems, secondsToMinutes } from "./rules";

  let {
    initial,
    subs,
    onsave,
    oncancel,
  }: { initial: AutoRule; subs: LocalSub[]; onsave: (r: AutoRule) => Promise<void> | void; oncancel: () => void } = $props();

  // Edited copy; the list keeps the original until Save.
  const cloneInitial = () => $state.snapshot(initial) as AutoRule;
  let rule = $state<AutoRule>(cloneInitial());
  let includeText = $state(rule.include_words.join(", "));
  let excludeText = $state(rule.exclude_words.join(", "));
  let minText = $state(secondsToMinutes(rule.min_duration_s));
  let maxText = $state(secondsToMinutes(rule.max_duration_s));
  let preview = $state<Item[] | null>(null);
  let previewing = $state(false);
  let previewError = $state<string | null>(null);
  let saving = $state(false);
  let tried = $state(false);

  let draft = $derived<AutoRule>({
    ...rule,
    include_words: parseWords(includeText),
    exclude_words: parseWords(excludeText),
    min_duration_s: minutesToSeconds(minText),
    max_duration_s: minutesToSeconds(maxText),
  });
  let problems = $derived(ruleProblems(draft));
  let selected = $derived(new Set(rule.channel_ids));

  function toggleChannel(id: string) {
    rule.channel_ids = selected.has(id) ? rule.channel_ids.filter((c) => c !== id) : [...rule.channel_ids, id];
  }

  function num(v: string): number {
    const n = Number(v);
    return Number.isFinite(n) && n >= 0 ? n : 0;
  }

  async function chooseFolder() {
    const dir = await open({ directory: true, title: $t("tube.settings.rule_folder") as string }).catch(() => null);
    if (typeof dir === "string") rule.folder = dir;
  }

  async function runPreview() {
    previewing = true;
    previewError = null;
    try {
      preview = await api.rulesPreview(cleanRule(draft));
    } catch (e) {
      previewError = String(e);
    } finally {
      previewing = false;
    }
  }

  async function save() {
    tried = true;
    if (problems.length) return;
    saving = true;
    try {
      await onsave(cleanRule(draft));
    } finally {
      saving = false;
    }
  }

  function itemTitle(it: Item): string {
    return it.kind === "channel" || it.kind === "artist" ? it.name : it.title;
  }
  function itemChannel(it: Item): string {
    return it.kind === "video" || it.kind === "short" ? (it.channel?.name ?? "") : "";
  }
  function itemThumb(it: Item): string {
    return it.kind === "video" || it.kind === "short" ? thumbUrl(it.thumbs, 120) : "";
  }
</script>

<div class="re">
  <div class="re-grid">
    <label class="re-field re-name">
      <span>{$t("tube.settings.rule_name")}</span>
      <input bind:value={rule.name} placeholder={$t("tube.settings.rule_name_ph") as string} class:bad={tried && problems.includes("name")} />
    </label>
    <div class="re-enabled">
      <span>{$t("tube.settings.rule_enabled")}</span>
      <Toggle checked={rule.enabled} label={$t("tube.settings.rule_enabled") as string} onchange={(v) => (rule.enabled = v)} />
    </div>
  </div>

  <div class="re-block">
    <span class="re-label">{$t("tube.settings.rule_channels")}</span>
    <span class="re-hint">{$t("tube.settings.rule_channels_hint")}</span>
    {#if subs.length}
      <div class="re-chans">
        {#each subs as s (s.channel_id)}
          <button class="re-chan" class:on={selected.has(s.channel_id)} aria-pressed={selected.has(s.channel_id)} onclick={() => toggleChannel(s.channel_id)}>
            {#if s.avatar}<img class="yt-avatar" src={bestAvatar(s.avatar, 24)} alt="" width="24" height="24" loading="lazy" referrerpolicy="no-referrer" />{/if}
            <span class="yt-clamp-1">{s.name}</span>
            {#if selected.has(s.channel_id)}<TIcon name="check" size={14} />{/if}
          </button>
        {/each}
      </div>
    {:else}
      <p class="re-hint">{$t("tube.settings.rule_no_subs")}</p>
    {/if}
  </div>

  <div class="re-grid">
    <label class="re-field">
      <span>{$t("tube.settings.rule_include")}</span>
      <input bind:value={includeText} placeholder={$t("tube.settings.rule_words_ph") as string} />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_exclude")}</span>
      <input bind:value={excludeText} placeholder={$t("tube.settings.rule_words_ph") as string} />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_min_minutes")}</span>
      <input inputmode="decimal" bind:value={minText} placeholder="0" class:bad={tried && problems.includes("duration")} />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_max_minutes")}</span>
      <input inputmode="decimal" bind:value={maxText} placeholder="∞" class:bad={tried && problems.includes("duration")} />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_max_age")}</span>
      <input type="number" min="0" value={rule.max_age_days ?? ""} oninput={(e) => (rule.max_age_days = num(e.currentTarget.value) || null)} placeholder="∞" />
    </label>
  </div>

  <Row title={$t("tube.settings.rule_shorts") as string}>
    <Toggle checked={rule.include_shorts} label={$t("tube.settings.rule_shorts") as string} onchange={(v) => (rule.include_shorts = v)} />
  </Row>
  <Row title={$t("tube.settings.rule_live") as string}>
    <Toggle checked={rule.include_live} label={$t("tube.settings.rule_live") as string} onchange={(v) => (rule.include_live = v)} />
  </Row>
  <Row title={$t("tube.settings.rule_quality") as string}>
    <Seg
      value={rule.quality}
      label={$t("tube.settings.rule_quality") as string}
      options={RULE_QUALITIES.map((q) => ({ value: q as string, label: q === "best" ? ($t("tube.settings.q_best") as string) : q === "audio" ? ($t("tube.settings.q_audio") as string) : `${q}p` }))}
      onchange={(v) => (rule.quality = v)}
    />
  </Row>
  <Row title={$t("tube.settings.rule_sb") as string} desc={$t("tube.settings.rule_sb_desc") as string}>
    <Seg
      value={rule.sponsorblock}
      label={$t("tube.settings.rule_sb") as string}
      options={RULE_SB.map((m) => ({ value: m as string, label: $t(`tube.settings.sbdl_${m}`) as string }))}
      onchange={(v) => (rule.sponsorblock = v)}
    />
  </Row>
  <Row title={$t("tube.settings.rule_folder") as string} desc={rule.folder || ($t("tube.settings.rule_folder_default") as string)}>
    <button class="yt-btn yt-btn--s" onclick={chooseFolder}><TIcon name="folder" size={16} />{$t("tube.settings.choose")}</button>
    {#if rule.folder}<button class="yt-btn yt-btn--s yt-btn--ghost" onclick={() => (rule.folder = null)}>{$t("tube.settings.reset")}</button>{/if}
  </Row>

  <div class="re-grid re-limits">
    <label class="re-field">
      <span>{$t("tube.settings.rule_max_gb")}</span>
      <input type="number" min="0" step="1" value={rule.max_gb || ""} oninput={(e) => (rule.max_gb = num(e.currentTarget.value))} placeholder="∞" />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_keep_days")}</span>
      <input type="number" min="0" step="1" value={rule.keep_days || ""} oninput={(e) => (rule.keep_days = num(e.currentTarget.value))} placeholder="∞" />
    </label>
    <label class="re-field">
      <span>{$t("tube.settings.rule_keep_per_channel")}</span>
      <input type="number" min="0" step="1" value={rule.keep_per_channel || ""} oninput={(e) => (rule.keep_per_channel = num(e.currentTarget.value))} placeholder="∞" />
    </label>
  </div>
  <p class="re-hint">{$t("tube.settings.rule_limits_hint")}</p>

  {#if tried && problems.length}
    <p class="re-err" role="alert">
      {#each problems as p (p)}<span>{$t(`tube.settings.rule_problem_${p}`)}</span>{/each}
    </p>
  {/if}

  <div class="re-preview">
    <div class="re-preview-head">
      <span class="re-label">{$t("tube.settings.rule_preview")}</span>
      <button class="yt-btn yt-btn--s" onclick={runPreview} disabled={previewing}>
        <TIcon name={previewing ? "loader" : "sparkles"} size={16} class={previewing ? "re-spin" : ""} />
        {preview ? $t("tube.settings.rule_preview_again") : $t("tube.settings.rule_preview_run")}
      </button>
    </div>
    {#if previewError}
      <p class="re-err">{previewError}</p>
    {:else if previewing && !preview}
      {#each [0, 1, 2] as i (i)}<div class="re-pv-skel"><span class="yt-skel"></span><span class="yt-skel"></span></div>{/each}
    {:else if preview}
      {#if preview.length}
        <p class="re-hint">{$t("tube.settings.rule_preview_count", { count: preview.length })}</p>
        <ul class="re-pv yt-scroll">
          {#each preview.slice(0, 30) as it, i (i)}
            <li>
              {#if itemThumb(it)}<img src={itemThumb(it)} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />{:else}<span class="re-pv-ph"></span>{/if}
              <span class="re-pv-text">
                <span class="yt-clamp-1">{itemTitle(it)}</span>
                <span class="re-pv-sub yt-clamp-1">{[itemChannel(it), it.kind === "video" && it.duration_s ? fmtDuration(it.duration_s) : "", it.kind === "video" ? it.published_text : ""].filter(Boolean).join(" · ")}</span>
              </span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="re-hint">{$t("tube.settings.rule_preview_none")}</p>
      {/if}
    {:else}
      <p class="re-hint">{$t("tube.settings.rule_preview_hint")}</p>
    {/if}
  </div>

  <div class="re-actions">
    <button class="yt-btn yt-btn--ghost" onclick={oncancel}>{$t("tube.settings.cancel")}</button>
    <button class="yt-btn yt-btn--inverted" onclick={save} disabled={saving}>{$t("tube.settings.save")}</button>
  </div>
</div>

<style>
  .re { display: flex; flex-direction: column; gap: 12px; padding: 16px 0; }
  .re-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 12px 16px; align-items: end; }
  .re-name { grid-column: span 2; }
  .re-field { display: flex; flex-direction: column; gap: 6px; font: 500 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .re-field input {
    height: 36px; padding: 0 12px; border-radius: 8px;
    border: 1px solid var(--yt-outline); background: var(--yt-bg); color: var(--yt-text);
    font: var(--yt-type-meta);
  }
  .re-field input:focus { outline: 2px solid var(--yt-link); outline-offset: -1px; border-color: transparent; }
  .re-field input.bad { border-color: var(--yt-error); }
  .re-enabled { display: flex; align-items: center; gap: 12px; height: 36px; font: var(--yt-type-body-strong); }
  .re-block { display: flex; flex-direction: column; gap: 6px; }
  .re-label { font: var(--yt-type-body-strong); color: var(--yt-text); }
  .re-hint { margin: 0; font: 400 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .re-chans { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 4px; max-height: 168px; overflow-y: auto; }
  .re-chan {
    display: inline-flex; align-items: center; gap: 6px; max-width: 220px;
    height: 32px; padding: 0 12px 0 4px; border: 0; border-radius: 999px;
    font: 500 13px/18px var(--yt-font); color: var(--yt-text); background: var(--yt-tonal); cursor: pointer;
  }
  .re-chan:not(:has(img)) { padding-left: 12px; }
  .re-chan img { width: 24px; height: 24px; }
  .re-chan:hover { background: var(--yt-tonal-hover); }
  .re-chan.on { background: var(--yt-bg-inverted); color: var(--yt-on-inverted); }
  .re-chan:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .re-err { margin: 0; display: flex; flex-direction: column; gap: 2px; font: 500 13px/18px var(--yt-font); color: var(--yt-error); }

  .re-preview { margin-top: 8px; padding: 12px; border-radius: 12px; background: var(--yt-tonal); display: flex; flex-direction: column; gap: 8px; }
  .re-preview-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .re-preview :global(.re-spin) { animation: re-spin 1s linear infinite; }
  @keyframes re-spin { to { transform: rotate(360deg); } }
  .re-pv { list-style: none; margin: 0; padding: 0; max-height: 280px; display: flex; flex-direction: column; gap: 6px; }
  .re-pv li { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .re-pv img, .re-pv-ph { width: 80px; height: 45px; border-radius: 6px; object-fit: cover; flex: none; background: var(--yt-skeleton); }
  .re-pv-text { display: flex; flex-direction: column; min-width: 0; font: 500 13px/18px var(--yt-font); }
  .re-pv-sub { font: 400 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .re-pv-skel { display: flex; gap: 12px; align-items: center; }
  .re-pv-skel .yt-skel:first-child { width: 80px; height: 45px; }
  .re-pv-skel .yt-skel:last-child { flex: 1; height: 14px; }
  .re-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  @media (prefers-reduced-motion: reduce) { .re-preview :global(.re-spin) { animation: none; } }
</style>
