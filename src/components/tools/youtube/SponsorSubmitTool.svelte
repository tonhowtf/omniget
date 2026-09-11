<script lang="ts">
  /** Enviar trechos ao SponsorBlock. O envio é público: só sai com confirmação explícita. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, openUrl } from "$lib/tools/rt";

  type NewSegment = { start: number; end: number; category: string; action_type: string; description: string };
  type SubmitResult = { video_id: string; accepted: number; uuids: string[]; fingerprint: string; message: string };
  type Existing = { video_id: string; segments: { segment: [number, number]; category: string; action_type: string }[]; video_duration: number };

  const CATEGORIES = ["sponsor", "selfpromo", "interaction", "intro", "outro", "preview", "music_offtopic", "filler", "exclusive_access", "poi_highlight", "chapter"];
  const ACTIONS = ["skip", "mute", "full", "poi", "chapter"];

  let url = $state("");
  let duration = $state(0);
  let fingerprint = $state("");
  let confirmed = $state(false);
  let busy = $state(false);
  let sent = $state<SubmitResult | null>(null);
  let rows = $state<NewSegment[]>([{ start: 0, end: 0, category: "sponsor", action_type: "skip", description: "" }]);

  const valid = $derived(url.trim().length > 0 && rows.length > 0 && rows.every((r) => r.action_type === "poi" || r.action_type === "full" || r.end > r.start));

  onMount(async () => {
    try {
      const u = await invoke<{ fingerprint: string }>("tool_sb_user");
      fingerprint = u.fingerprint;
    } catch {
      fingerprint = "";
    }
  });

  function add() {
    const last = rows[rows.length - 1];
    rows = [...rows, { start: last ? last.end : 0, end: last ? last.end : 0, category: "sponsor", action_type: "skip", description: "" }];
  }

  function remove(i: number) {
    rows = rows.filter((_, k) => k !== i);
  }

  async function lookup() {
    if (!url.trim()) return;
    try {
      const r = await invoke<Existing>("tool_sponsorblock", { url, categories: null });
      if (r.video_duration > 0) duration = r.video_duration;
      showToast("info", `${r.segments.length} ${$t("tools.sbsubmit.already_there")}`);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function submit() {
    if (!valid || !confirmed || busy) return;
    busy = true;
    try {
      sent = await invoke<SubmitResult>("tool_sb_submit", {
        opts: {
          url,
          segments: rows.map((r) => ({ ...r, start: Number(r.start), end: Number(r.end) })),
          video_duration: Number(duration) || 0,
          confirmed: true,
        },
      });
      confirmed = false;
      showToast("success", `${sent.accepted} ${$t("tools.sbsubmit.accepted")}`);
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
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.common.yt_url")} /></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={!url.trim()} onclick={lookup}>{$t("tools.sbsubmit.lookup")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.sbsubmit.duration")}</div><div class="group-row-sub">{$t("tools.sbsubmit.duration_sub")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" step="1" bind:value={duration} style:width="7em" /><span class="dim">{duration ? fmtSecs(duration) : "—"}</span></div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.sbsubmit.segments")}</span>
    <div class="group">
      {#each rows as r, i (i)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="row-fields">
              <input class="input" type="number" min="0" step="0.1" bind:value={r.start} aria-label={$t("tools.sbsubmit.start")} />
              <input class="input" type="number" min="0" step="0.1" bind:value={r.end} aria-label={$t("tools.sbsubmit.end")} />
              <select class="input" bind:value={r.category}>{#each CATEGORIES as c (c)}<option value={c}>{c}</option>{/each}</select>
              <select class="input" bind:value={r.action_type}>{#each ACTIONS as a (a)}<option value={a}>{a}</option>{/each}</select>
            </div>
            {#if r.action_type === "chapter"}<input class="input" type="text" bind:value={r.description} placeholder={$t("tools.sbsubmit.chapter_title")} />{/if}
            <div class="group-row-sub">{fmtSecs(Number(r.start) || 0)} → {fmtSecs(Number(r.end) || 0)}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => remove(i)}>×</button></div>
        </div>
      {/each}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{$t("tools.sbsubmit.rows_hint")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={add}>{$t("tools.common.add")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.sbsubmit.public_label")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.sbsubmit.public_title")}</div>
          <div class="group-row-sub">{$t("tools.sbsubmit.public_note")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl("https://wiki.sponsor.ajay.app/w/Guidelines")}>{$t("tools.sbsubmit.guidelines")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.sbsubmit.identity")}</div><div class="group-row-sub mono">{fingerprint || "—"}</div><div class="group-row-sub">{$t("tools.sbsubmit.identity_sub")}</div></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><label class="chk"><input type="checkbox" bind:checked={confirmed} /> {$t("tools.sbsubmit.confirm")}</label></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !valid || !confirmed} onclick={submit}>{busy ? $t("tools.common.working") : $t("tools.sbsubmit.send")}</button></div>
      </div>
    </div>
  </section>

  {#if sent}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title"><span class="tag tag-success">{sent.accepted} {$t("tools.sbsubmit.accepted")}</span></div>
            <div class="group-row-sub mono">{sent.uuids.join(" · ") || sent.message}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(`https://sb.ltn.fi/video/${sent!.video_id}/`)}>{$t("tools.common.open")}</button></div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chk { display: inline-flex; align-items: center; gap: 6px; }
  .row-fields { display: flex; gap: var(--space-2); flex-wrap: wrap; }
  .row-fields :global(input[type="number"]) { width: 7em; }
</style>
