<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { currentOs } from "$lib/platform";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { GROUP_ORDER, visibleModules, needsBadge, type Caps } from "#components/washmybelly/registry.ts";
  import EmptyState from "#components/washmybelly/ui/EmptyState.svelte";
  import Icon from "#components/washmybelly/ui/Icon.svelte";
  import OverviewModule from "#components/washmybelly/OverviewModule.svelte";
  import CleanModule from "#components/washmybelly/CleanModule.svelte";
  import DiskModule from "#components/washmybelly/DiskModule.svelte";
  import UninstallModule from "#components/washmybelly/UninstallModule.svelte";
  import StartupModule from "#components/washmybelly/StartupModule.svelte";
  import PrivacyModule from "#components/washmybelly/PrivacyModule.svelte";
  import TweaksModule from "#components/washmybelly/TweaksModule.svelte";
  import PackagesModule from "#components/washmybelly/PackagesModule.svelte";
  import PowerModule from "#components/washmybelly/PowerModule.svelte";
  import GamesModule from "#components/washmybelly/GamesModule.svelte";
  import PerfModule from "#components/washmybelly/PerfModule.svelte";
  import SecurityModule from "#components/washmybelly/SecurityModule.svelte";
  import MaintenanceModule from "#components/washmybelly/MaintenanceModule.svelte";
  import ReceiptsModule from "#components/washmybelly/ReceiptsModule.svelte";

  const VIEWS = {
    overview: OverviewModule,
    clean: CleanModule,
    disk: DiskModule,
    uninstall: UninstallModule,
    startup: StartupModule,
    privacy: PrivacyModule,
    tweaks: TweaksModule,
    packages: PackagesModule,
    power: PowerModule,
    games: GamesModule,
    perf: PerfModule,
    security: SecurityModule,
    maintenance: MaintenanceModule,
    receipts: ReceiptsModule,
  } as const;

  let enabled = $derived(getSettings()?.washmybelly?.enabled ?? true);
  let caps = $state<Caps>({ os: currentOs() });
  let loaded = $state(false);
  let refreshing = $state(false);
  let error = $state<string | null>(null);
  let resolving = $state(false);
  let resolveError = $state<string | null>(null);
  let brandFailed = $state(false);
  let side = $state<HTMLElement | null>(null);

  let modules = $derived(visibleModules(caps));
  let active = $derived.by(() => {
    const want = page.url.searchParams.get("m") ?? "overview";
    return modules.find((m) => m.mod.id === want) ?? modules[0];
  });
  let View = $derived(active ? VIEWS[active.mod.id as keyof typeof VIEWS] : null);
  let unavailable = $derived(active && active.availability.show && !active.availability.available ? active.availability : null);
  let systemLine = $derived.by(() => {
    const os = $t(`washmybelly.ui.os.${caps.os}`) as string;
    const parts = [caps.linux?.distro_name ?? os, caps.os_version].filter(Boolean);
    return parts.join(" ");
  });

  async function load(refresh = false) {
    error = null;
    if (loaded) refreshing = true;
    try {
      caps = await api.capabilities(refresh);
      loaded = true;
    } catch (e) {
      error = errorText(e);
    } finally {
      refreshing = false;
    }
  }

  function open(id: string) {
    void goto(`/washmybelly?m=${id}`, { reset: false, replace: true });
  }

  function onSideKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const ids = GROUP_ORDER.flatMap((g) => modules.filter((m) => m.mod.group === g).map((m) => m.mod.id));
    const at = ids.indexOf(active?.mod.id ?? "");
    const next = ids[Math.max(0, Math.min(ids.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))];
    if (!next || next === active?.mod.id) return;
    e.preventDefault();
    open(next);
    queueMicrotask(() => side?.querySelector<HTMLElement>(`[data-mod="${next}"]`)?.focus());
  }

  function fixFor(reasonKey: string): { labelKey: string; action: string } | null {
    if (reasonKey === "washmybelly.reason.full_disk_access") {
      return { labelKey: "washmybelly.ui.open_fda", action: "security.open_fda_settings" };
    }
    return null;
  }

  async function runFix(action: string) {
    resolving = true;
    resolveError = null;
    try {
      await api.ui(action);
    } catch (e) {
      resolveError = errorText(e);
    } finally {
      resolving = false;
    }
  }

  onMount(() => {
    if (enabled) void load();
  });
</script>

{#if !enabled}
  <div class="wmb-guard">
    <EmptyState art="sleeping" title={$t("washmybelly.disabled_title") as string} body={$t("washmybelly.disabled_body") as string}>
      {#snippet action()}
        <a class="btn btn-primary" href="/settings?tab=integrations&section=washmybelly">{$t("washmybelly.open_settings")}</a>
      {/snippet}
    </EmptyState>
  </div>
{:else}
  <div class="settings-mac wmb">
    <div class="settings-mac-body">
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <aside class="settings-mac-sidebar wmb-side" aria-label={$t("washmybelly.nav") as string} bind:this={side} onkeydown={onSideKey}>
        <div class="wmb-brand">
          {#if !brandFailed}
            <img
              class="wmb-brand-img"
              src="/icons/menu/washmybelly-64.webp"
              srcset="/icons/menu/washmybelly-64.webp 1x, /icons/menu/washmybelly-128.webp 2x"
              alt=""
              width="30"
              height="30"
              onerror={() => (brandFailed = true)}
            />
          {:else}
            <span class="wmb-brand-img wmb-brand-fallback" aria-hidden="true">
              <svg viewBox="0 0 32 32" fill="currentColor"><circle cx="13" cy="18" r="8" /><circle cx="22.5" cy="11" r="4.5" opacity="0.75" /><circle cx="23" cy="22" r="3" opacity="0.6" /><circle cx="10.5" cy="15.5" r="2.2" fill="#fff" opacity="0.6" /></svg>
            </span>
          {/if}
          <span class="wmb-brand-text">
            <span class="wmb-brand-name">{$t("washmybelly.nav")}</span>
            {#if loaded}
              <span class="wmb-brand-sub" title={systemLine}>{systemLine}</span>
            {:else}
              <span class="skeleton wmb-brand-skel"></span>
            {/if}
          </span>
          <button
            class="btn btn-icon btn-sm wmb-refresh"
            class:spin={refreshing}
            title={$t("washmybelly.ui.recheck") as string}
            aria-label={$t("washmybelly.ui.recheck") as string}
            disabled={refreshing || (!loaded && !error)}
            onclick={() => load(true)}
          >
            <Icon name="refresh" size={14} />
          </button>
        </div>

        {#each GROUP_ORDER as group (group)}
          {@const items = modules.filter((m) => m.mod.group === group)}
          {#if items.length}
            <div class="settings-mac-nav-group">
              <div class="mac-nav-section-header">{$t(`washmybelly.groups.${group}`)}</div>
              {#each items as { mod, availability } (mod.id)}
                {@const off = availability.show && !availability.available}
                <button
                  class="settings-mac-cat wmb-cat"
                  class:active={active?.mod.id === mod.id}
                  class:unavailable={off}
                  data-mod={mod.id}
                  aria-current={active?.mod.id === mod.id ? "page" : undefined}
                  onclick={() => open(mod.id)}
                >
                  <span class="wmb-tile" style:--from={mod.from} style:--to={mod.to} aria-hidden="true">
                    <span class="wmb-glyph" style:--glyph="url(/icons/{mod.glyph}.svg)"></span>
                  </span>
                  <span class="settings-mac-cat-label">{$t(mod.labelKey)}</span>
                  {#if off}
                    <span class="wmb-off" title={$t(availability.reasonKey) as string}><Icon name="lock" size={11} /></span>
                  {:else if needsBadge(mod)}
                    <span class="wmb-badge">{$t(`washmybelly.${mod.state}`)}</span>
                  {/if}
                </button>
              {/each}
            </div>
          {/if}
        {/each}
      </aside>

      <section class="wmb-main">
        <div class="wmb-main-inner">
          {#if error}
            <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
              {#snippet action()}
                <button class="btn btn-primary" onclick={() => load(true)}>{$t("washmybelly.retry")}</button>
              {/snippet}
            </EmptyState>
          {:else if !loaded}
            <div class="wmb-skel" aria-busy="true" aria-label={$t("washmybelly.loading") as string}>
              <div class="wmb-skel-head">
                <span class="skeleton" style:width="40px" style:height="40px" style:border-radius="10px"></span>
                <span class="wmb-skel-lines">
                  <span class="skeleton" style:width="180px" style:height="20px"></span>
                  <span class="skeleton" style:width="280px"></span>
                </span>
              </div>
              <div class="wmb-skel-tiles">
                {#each [0, 1, 2] as i (i)}<span class="skeleton" style:height="72px" style:border-radius="var(--radius-lg)"></span>{/each}
              </div>
              <span class="skeleton" style:height="220px" style:border-radius="var(--radius-lg)"></span>
              <p class="wmb-skel-note">{$t("washmybelly.loading")}</p>
            </div>
          {:else if active && unavailable}
            <EmptyState art="worried" title={$t(active.mod.labelKey) as string} body={$t(unavailable.reasonKey) as string}>
              {#snippet action()}
                {@const fix = fixFor(unavailable.reasonKey)}
                {#if fix}
                  <button class="btn btn-primary" disabled={resolving} onclick={() => runFix(fix.action)}>{$t(fix.labelKey)}</button>
                {/if}
                <button class="btn btn-secondary" disabled={refreshing} onclick={() => load(true)}>
                  {#if refreshing}<span class="spinner" aria-hidden="true"></span>{/if}
                  {$t("washmybelly.ui.recheck")}
                </button>
              {/snippet}
            </EmptyState>
            {#if resolveError}<p class="wmb-fix-err" role="alert">{resolveError}</p>{/if}
          {:else if View}
            {#key active?.mod.id}
              <View {caps} />
            {/key}
          {/if}
        </div>
      </section>
    </div>
  </div>
{/if}

<style>
  .wmb {
    height: 100%;
  }
  .wmb-side {
    width: 224px;
  }
  .wmb-brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 2px var(--space-1) var(--space-3) var(--space-2);
    margin-bottom: var(--space-1);
  }
  .wmb-brand-img {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    object-fit: contain;
  }
  .wmb-brand-fallback {
    display: grid;
    place-items: center;
    border-radius: 8px;
    color: #fff;
    background: linear-gradient(180deg, #ff9bc2, #e0527f);
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12);
  }
  .wmb-brand-fallback svg {
    width: 22px;
    height: 22px;
  }
  .wmb-brand-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .wmb-brand-name {
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 700;
    letter-spacing: var(--track-snug);
    color: var(--text);
    line-height: 1.15;
  }
  .wmb-brand-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wmb-brand-skel {
    width: 90px;
    height: 10px;
    min-height: 10px;
    margin-top: 3px;
  }
  .wmb-refresh.spin :global(svg) {
    animation: wmb-spin 0.9s linear infinite;
  }
  @keyframes wmb-spin {
    to { transform: rotate(360deg); }
  }
  .wmb-cat {
    height: 32px;
  }
  .wmb-cat:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }
  .wmb-tile {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: linear-gradient(180deg, var(--from), var(--to));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12), inset 0 1px 0 rgb(255 255 255 / 0.16);
  }
  .wmb-glyph {
    width: 14px;
    height: 14px;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
  }
  .wmb-cat.unavailable .wmb-tile {
    filter: saturate(0.2);
    opacity: 0.7;
  }
  .wmb-cat.unavailable .settings-mac-cat-label {
    opacity: 0.6;
  }
  .wmb-off {
    display: flex;
    color: var(--text-dim);
  }
  .wmb-cat.active .wmb-off {
    color: inherit;
  }
  .wmb-badge {
    flex-shrink: 0;
    height: 16px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
    font-size: 10px;
    font-weight: 600;
    line-height: 16px;
  }
  .wmb-cat.active .wmb-badge {
    background: color-mix(in srgb, var(--on-accent) 14%, transparent);
    color: inherit;
  }
  .wmb-main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .wmb-main-inner {
    max-width: 980px;
    margin: 0 auto;
    padding: var(--space-5) var(--space-6) var(--space-7);
  }
  .wmb-skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .wmb-skel-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .wmb-skel-lines {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .wmb-skel-tiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
  }
  .wmb-skel-note {
    margin: 0;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .wmb-fix-err {
    margin: 0;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--danger);
  }
  .wmb-guard {
    display: grid;
    place-items: center;
    height: 100%;
    padding: var(--space-6);
  }
  @media (max-width: 720px) {
    .wmb-side {
      width: 64px;
      padding-inline: var(--space-1);
    }
    .wmb-brand-text,
    .wmb-refresh,
    .settings-mac-cat-label,
    .wmb-badge,
    .wmb-off,
    .mac-nav-section-header {
      display: none;
    }
    .wmb-cat {
      justify-content: center;
    }
    .wmb-main-inner {
      padding: var(--space-4);
    }
  }
</style>
