<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import RouteRow from "./RouteRow.svelte";
  import PasteLinks from "./PasteLinks.svelte";
  import DoctorReport from "./DoctorReport.svelte";
  import MihomoCard from "./MihomoCard.svelte";
  import { net, errText, formatDate, type DoctorReport as Report, type NetOpts, type RouteView } from "./api";
  import { netState, scanNet, refreshNet, isScanning, CLOCK_WARN_SECS } from "./net.svelte";

  let status = $derived(netState.status);
  let scanning = $derived(isScanning());
  let filter = $state<"all" | "ok" | "saved">("all");

  let routes = $derived.by<RouteView[]>(() => {
    const list = [...(status?.routes ?? [])];
    const rank = (v: RouteView) => (v.active ? 0 : v.health.ok === true ? 1 : v.health.ok == null ? 2 : 3);
    list.sort((a, b) => rank(a) - rank(b) || (a.health.rtt_ms ?? 1e9) - (b.health.rtt_ms ?? 1e9));
    if (filter === "ok") return list.filter((v) => v.health.ok === true);
    if (filter === "saved") return list.filter((v) => v.saved);
    return list;
  });
  let failedSaved = $derived((status?.routes ?? []).filter((v) => v.saved && v.health.ok === false).length);
  let okCount = $derived((status?.routes ?? []).filter((v) => v.health.ok === true).length);

  // subscriptions
  let subUrl = $state("");
  let subName = $state("");
  let subBusy = $state<string | null>(null);

  // advanced
  let opts = $state<NetOpts>({ prefer_ipv6: false, prefer_doh: false, connect_timeout_secs: 10 });
  let useRescue = $state(true);
  let optsLoadedAt = -1;
  $effect(() => {
    // Take the backend values once per status snapshot the person has not edited.
    const s = status;
    if (!s || optsLoadedAt === s.scanned_at) return;
    optsLoadedAt = s.scanned_at;
    opts = { ...s.opts };
    useRescue = s.use_rescue;
  });

  // doctor
  let report = $state<Report | null>(null);
  let doctorBusy = $state(false);

  async function setMode(mode: "auto" | "fixed") {
    if (mode === status?.mode) return;
    try {
      if (mode === "fixed") {
        const id = status?.active?.route.id;
        if (!id) {
          showToast("info", $t("telegram.net.fixed_needs_route") as string);
          return;
        }
        await net.setMode("fixed", id);
      } else {
        await net.setMode("auto");
      }
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function removeFailed() {
    try {
      const n = await net.removeFailed();
      showToast("success", $t("telegram.net.removed_failed", { count: n }) as string);
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function addSub() {
    const url = subUrl.trim();
    if (!url) return;
    subBusy = "add";
    try {
      const entry = await net.addSubscription(url, subName.trim());
      showToast("success", $t("telegram.net.sub_added", { count: entry.count }) as string);
      subUrl = "";
      subName = "";
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      subBusy = null;
    }
  }

  async function refreshSubs() {
    subBusy = "refresh";
    try {
      const res = await net.refreshSubscriptions();
      const failed = res.filter(([, r]) => r && "Err" in r && r.Err);
      if (failed.length) showToast("error", failed.map(([u, r]) => `${u}: ${r.Err}`).join("\n"));
      else showToast("success", $t("telegram.net.sub_refreshed") as string);
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      subBusy = null;
    }
  }

  async function removeSub(url: string) {
    subBusy = url;
    try {
      await net.removeSubscription(url);
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      subBusy = null;
    }
  }

  async function saveOpts() {
    try {
      const timeout = Math.min(60, Math.max(3, Math.round(Number(opts.connect_timeout_secs) || 10)));
      await net.setOpts({ ...opts, connect_timeout_secs: timeout }, useRescue);
      await refreshNet();
      showToast("success", $t("telegram.net.opts_saved") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function runDoctor() {
    doctorBusy = true;
    try {
      report = await net.doctor();
      netState.status = report.status;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      doctorBusy = false;
    }
  }

  function unsupportedText(list: [string, number][]): string {
    return list.map(([s, n]) => `${s} ×${n}`).join(", ");
  }
</script>

<div class="conn">
  {#if status && status.clock_offset_secs != null && Math.abs(status.clock_offset_secs) > CLOCK_WARN_SECS}
    <div class="notice notice-warning">
      <TgIcon name="clock" size={16} />
      <div class="notice-body">
        <div class="notice-title">{$t("telegram.net.clock_title")}</div>
        <div class="notice-text">{$t("telegram.status.clock_warn", { secs: status.clock_offset_secs > 0 ? `+${status.clock_offset_secs}` : `${status.clock_offset_secs}` })}</div>
      </div>
    </div>
  {/if}

  <!-- Mode + routes -->
  <section>
    <div class="sec-head">
      <h3>{$t("telegram.net.routes_title")}</h3>
      <div class="segmented" role="radiogroup" aria-label={$t("telegram.net.mode")}>
        <button class="segmented-btn" role="radio" aria-checked={status?.mode !== "fixed"} onclick={() => setMode("auto")}>{$t("telegram.net.mode_auto")}</button>
        <button class="segmented-btn" role="radio" aria-checked={status?.mode === "fixed"} onclick={() => setMode("fixed")}>{$t("telegram.net.mode_fixed")}</button>
      </div>
    </div>
    <p class="sec-sub">{status?.mode === "fixed" ? $t("telegram.net.mode_fixed_hint") : $t("telegram.net.mode_auto_hint")}</p>

    <div class="toolbar">
      <div class="segmented" role="tablist">
        <button class="segmented-btn" role="tab" aria-selected={filter === "all"} onclick={() => (filter = "all")}>{$t("telegram.net.filter_all", { count: status?.routes.length ?? 0 })}</button>
        <button class="segmented-btn" role="tab" aria-selected={filter === "ok"} onclick={() => (filter = "ok")}>{$t("telegram.net.filter_ok", { count: okCount })}</button>
        <button class="segmented-btn" role="tab" aria-selected={filter === "saved"} onclick={() => (filter = "saved")}>{$t("telegram.net.filter_saved")}</button>
      </div>
      <span class="grow"></span>
      {#if failedSaved > 0}
        <button class="btn btn-ghost btn-sm" onclick={removeFailed}><TgIcon name="trash" /> {$t("telegram.net.remove_failed", { count: failedSaved })}</button>
      {/if}
      <button class="btn btn-secondary btn-sm" onclick={() => void scanNet()} disabled={scanning}>
        {#if scanning}<span class="spinner"></span>{:else}<TgIcon name="bolt" />{/if}
        {$t("telegram.net.test_all")}
      </button>
    </div>

    {#if routes.length}
      <div class="group">
        {#each routes as view (view.route.id)}
          <RouteRow {view} pending={scanning} fixed={status?.mode === "fixed"} />
        {/each}
      </div>
    {:else}
      <div class="group empty">{scanning ? $t("telegram.status.scanning") : $t("telegram.net.no_routes")}</div>
    {/if}
    {#if status?.scanned_at}
      <p class="foot">{$t("telegram.net.scanned_at", { when: new Date(status.scanned_at * 1000).toLocaleTimeString() })}</p>
    {/if}
  </section>

  <!-- Paste links -->
  <section>
    <h3>{$t("telegram.net.paste_title")}</h3>
    <p class="sec-sub">{$t("telegram.net.paste_hint")}</p>
    <PasteLinks />
  </section>

  <!-- Subscriptions -->
  <section>
    <div class="sec-head">
      <h3>{$t("telegram.net.subs_title")}</h3>
      {#if status?.subscriptions.length}
        <button class="btn btn-ghost btn-sm" onclick={refreshSubs} disabled={!!subBusy}>
          {#if subBusy === "refresh"}<span class="spinner"></span>{:else}<TgIcon name="refresh" />{/if}
          {$t("telegram.net.sub_refresh_all")}
        </button>
      {/if}
    </div>
    <p class="sec-sub">{$t("telegram.net.subs_hint")}</p>
    <div class="sub-form">
      <input class="input grow" type="url" bind:value={subUrl} placeholder={$t("telegram.net.sub_url")} spellcheck="false" autocomplete="off" />
      <input class="input name" type="text" bind:value={subName} placeholder={$t("telegram.net.sub_name")} />
      <button class="btn btn-primary" onclick={addSub} disabled={!subUrl.trim() || !!subBusy}>
        {#if subBusy === "add"}<span class="spinner"></span>{/if}
        {$t("telegram.net.sub_add")}
      </button>
    </div>
    {#if status?.subscriptions.length}
      <div class="group">
        {#each status.subscriptions as sub (sub.url)}
          <div class="group-row">
            <div class="group-row-content">
              <span class="group-row-title">{sub.name || sub.url}</span>
              <span class="group-row-sub">
                {$t("telegram.net.sub_count", { count: sub.count })}
                {#if sub.updated_at} · {$t("telegram.net.sub_updated", { when: formatDate(sub.updated_at) })}{/if}
                {#if sub.unsupported.length} · {$t("telegram.net.sub_unsupported", { list: unsupportedText(sub.unsupported) })}{/if}
              </span>
            </div>
            <div class="group-row-trailing">
              <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.remove")} aria-label={$t("telegram.net.remove")} disabled={!!subBusy} onclick={() => removeSub(sub.url)}>
                {#if subBusy === sub.url}<span class="spinner"></span>{:else}<TgIcon name="trash" />{/if}
              </button>
            </div>
          </div>
        {/each}
      </div>
      {#if status.subscriptions.some((s) => s.unsupported.length)}
        <p class="foot">{$t("telegram.net.needs_clash")}</p>
      {/if}
    {/if}
  </section>

  <MihomoCard />

  <!-- Diagnosis -->
  <section>
    <div class="sec-head">
      <h3>{$t("telegram.doctor.title")}</h3>
      <button class="btn btn-secondary btn-sm" onclick={runDoctor} disabled={doctorBusy}>
        {#if doctorBusy}<span class="spinner"></span>{:else}<TgIcon name="shield" />{/if}
        {$t("telegram.doctor.run")}
      </button>
    </div>
    <p class="sec-sub">{$t("telegram.doctor.hint")}</p>
    {#if report}<DoctorReport {report} />{/if}
  </section>

  <!-- Advanced -->
  <details class="adv">
    <summary>{$t("telegram.net.advanced")}</summary>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <span class="group-row-title">{$t("telegram.net.opt_rescue")}</span>
          <span class="group-row-sub">{$t("telegram.net.opt_rescue_hint")}</span>
        </div>
        <button class="toggle" role="switch" aria-checked={useRescue} aria-label={$t("telegram.net.opt_rescue")} onclick={() => (useRescue = !useRescue)}><span class="toggle-knob"></span></button>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <span class="group-row-title">{$t("telegram.net.opt_ipv6")}</span>
          <span class="group-row-sub">{$t("telegram.net.opt_ipv6_hint")}</span>
        </div>
        <button class="toggle" role="switch" aria-checked={opts.prefer_ipv6} aria-label={$t("telegram.net.opt_ipv6")} onclick={() => (opts.prefer_ipv6 = !opts.prefer_ipv6)}><span class="toggle-knob"></span></button>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <span class="group-row-title">{$t("telegram.net.opt_doh")}</span>
          <span class="group-row-sub">{$t("telegram.net.opt_doh_hint")}</span>
        </div>
        <button class="toggle" role="switch" aria-checked={opts.prefer_doh} aria-label={$t("telegram.net.opt_doh")} onclick={() => (opts.prefer_doh = !opts.prefer_doh)}><span class="toggle-knob"></span></button>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <span class="group-row-title">{$t("telegram.net.opt_timeout")}</span>
          <span class="group-row-sub">{$t("telegram.net.opt_timeout_hint")}</span>
        </div>
        <input class="input num" type="number" min="3" max="60" bind:value={opts.connect_timeout_secs} aria-label={$t("telegram.net.opt_timeout")} />
      </div>
    </div>
    <div class="adv-foot">
      <button class="btn btn-primary btn-sm" onclick={saveOpts}>{$t("telegram.common.save")}</button>
    </div>
  </details>
</div>

<style>
  .conn {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .sec-sub,
  .foot {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-top: var(--space-1);
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .empty {
    padding: var(--space-5);
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .sub-form {
    display: flex;
    gap: var(--space-2);
  }

  .sub-form .name {
    width: 160px;
  }

  .adv summary {
    cursor: pointer;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    margin-bottom: var(--space-2);
  }

  .num {
    width: 72px;
    text-align: right;
  }

  .adv-foot {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-2);
  }

  @media (max-width: 640px) {
    .sub-form {
      flex-wrap: wrap;
    }
    .sub-form .name {
      width: auto;
      flex: 1;
    }
  }
</style>
