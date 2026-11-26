<script lang="ts">
  import { onMount } from "svelte";
  import { pluralKey } from "$lib/fonts/text";
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/fonts/api";
  import {
    CURATED_IDS,
    categoryIndex,
    categoryKey,
    curatedFallback,
    filterCatalog,
    filterInstalled,
    hiddenCount,
    installedCategory,
    installedNames,
    mergeCatalog,
    normalize,
  } from "$lib/fonts/catalog";
  import { forgetFailures } from "$lib/fonts/loader";
  import { MAX_COMPARE, fromCatalog, fromInstalled, toggleCompare, type PreviewFont } from "$lib/fonts/preview";
  import { CODE_SAMPLE, defaultSample, type Sample } from "$lib/fonts/sample";
  import type { CatalogFont, InstalledFamily } from "$lib/fonts/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import StatusIcon from "#components/converter/StatusIcon.svelte";
  import FontList, { type ListItem, type Source } from "#components/fonts/FontList.svelte";
  import FontDetail from "#components/fonts/FontDetail.svelte";
  import SampleControls from "#components/fonts/SampleControls.svelte";
  import CompareView from "#components/fonts/CompareView.svelte";
  import PairingView from "#components/fonts/PairingView.svelte";
  import ScaleView from "#components/fonts/ScaleView.svelte";
  import type { PickerGroup } from "#components/fonts/FontPicker.svelte";

  type View = "fonts" | "compare" | "pairing" | "scale";
  type LoadStatus = "loading" | "ready" | "error";

  const PAGE = 120;
  const VIEWS: View[] = ["fonts", "compare", "pairing", "scale"];

  let view = $state<View>("fonts");
  let source = $state<Source>("catalog");
  let query = $state("");
  let category = $state<string | null>(null);
  let includeOther = $state(false);
  let limit = $state(PAGE);

  let catalog = $state<CatalogFont[]>(curatedFallback());
  let catalogStatus = $state<LoadStatus>("loading");
  let catalogError = $state<string | null>(null);
  let catalogRemote = $state(false);
  let installedList = $state<InstalledFamily[]>([]);
  let installedStatus = $state<LoadStatus>("loading");
  let installedError = $state<string | null>(null);
  let userDir = $state<string | null>(null);

  let selectedKey = $state<string | null>("c:geist");
  let compare = $state<string[]>(["c:geist", "c:inter"]);
  let headingKey = $state<string>("c:geist");
  let bodyKey = $state<string>("c:inter");
  let scaleKey = $state<string>("c:inter");
  let sample = $state<Sample>(defaultSample());

  let lang = $derived(($locale as string | undefined) ?? "en");
  let catIndex = $derived(categoryIndex(catalog));
  let names = $derived(installedNames(installedList));
  let catalogById = $derived(new Map(catalog.map((f) => [f.id, f])));
  let installedByName = $derived(new Map(installedList.map((f) => [f.family, f])));

  let filter = $derived({ query, category, includeOther });
  let catalogMatches = $derived(filterCatalog(catalog, filter));
  let hidden = $derived(source === "catalog" ? hiddenCount(catalog, filter) : 0);
  let installedMatches = $derived(filterInstalled(installedList, query, category, catIndex));

  let items = $derived.by((): ListItem[] => {
    if (source === "catalog") {
      return catalogMatches.slice(0, limit).map((f) => ({
        font: fromCatalog(f),
        installed: names.has(normalize(f.family)),
        licenseClass: f.license_class,
        detail: `${$t(categoryKey(f.category))} · ${$t(pluralKey("fonts.weights_count", f.weights.length), { count: f.weights.length })}${f.variable ? ` · ${$t("fonts.variable")}` : ""}`,
      }));
    }
    return installedMatches.slice(0, limit).map((f) => {
      const cat = installedCategory(f, catIndex);
      return {
        font: fromInstalled(f, cat),
        installed: true,
        licenseClass: null,
        detail: [cat ? $t(categoryKey(cat)) : null, $t(pluralKey("fonts.styles_count", f.styles.length), { count: f.styles.length }), f.is_variable ? $t("fonts.variable") : null].filter(Boolean).join(" · "),
      };
    });
  });
  let total = $derived(source === "catalog" ? catalogMatches.length : installedMatches.length);
  let listStatus = $derived(source === "catalog" ? catalogStatus : installedStatus);
  let listError = $derived(source === "catalog" ? catalogError : installedError);

  function resolve(key: string | null): PreviewFont | null {
    if (!key) return null;
    if (key.startsWith("c:")) {
      const f = catalogById.get(key.slice(2));
      return f ? fromCatalog(f) : null;
    }
    const fam = installedByName.get(key.slice(2));
    return fam ? fromInstalled(fam, installedCategory(fam, catIndex)) : null;
  }

  let selected = $derived(resolve(selectedKey));
  let selectedCatalog = $derived(selectedKey?.startsWith("c:") ? (catalogById.get(selectedKey.slice(2)) ?? null) : null);
  let selectedInstalled = $derived(selectedKey?.startsWith("i:") ? (installedByName.get(selectedKey.slice(2)) ?? null) : null);
  let compareFonts = $derived(compare.map(resolve).filter((f): f is PreviewFont => f !== null));
  let heading = $derived(resolve(headingKey));
  let body = $derived(resolve(bodyKey));
  let scaleFont = $derived(resolve(scaleKey));

  let presetText = $derived.by(() => {
    if (sample.preset === "paragraph") return $t("fonts.sample_paragraph") as string;
    if (sample.preset === "ui") return $t("fonts.sample_ui") as string;
    if (sample.preset === "code") return CODE_SAMPLE;
    return $t("fonts.sample_pangram") as string;
  });
  let text = $derived(sample.text.trim() ? sample.text : presetText);

  let groups = $derived.by((): PickerGroup[] => {
    const picks: PreviewFont[] = [];
    const seen = new Set<string>();
    for (const key of [selectedKey, ...compare, headingKey, bodyKey, scaleKey]) {
      if (!key || seen.has(key)) continue;
      const f = resolve(key);
      if (f) {
        seen.add(key);
        picks.push(f);
      }
    }
    const curated = CURATED_IDS.map((id) => catalogById.get(id)).filter((f): f is CatalogFont => !!f).map(fromCatalog);
    const local = installedList.map((f) => fromInstalled(f, installedCategory(f, catIndex)));
    return [
      { label: $t("fonts.group_picks") as string, fonts: picks },
      { label: $t("fonts.group_open") as string, fonts: curated },
      { label: $t("fonts.group_installed") as string, fonts: local },
    ];
  });

  let liveSummary = $derived.by(() => {
    if (view !== "fonts") return "";
    if (listStatus === "loading" && items.length === 0) return source === "catalog" ? ($t("fonts.loading_catalog") as string) : ($t("fonts.loading_installed") as string);
    return $t(pluralKey("fonts.results", total), { count: total }) as string;
  });

  $effect(() => {
    void query;
    void category;
    void source;
    void includeOther;
    limit = PAGE;
  });

  async function loadCatalog(refresh = false) {
    catalogStatus = "loading";
    catalogError = null;
    try {
      const remote = await api.catalog(refresh);
      catalog = mergeCatalog(remote);
      if (!catalogRemote) forgetFailures();
      catalogRemote = true;
      catalogStatus = "ready";
    } catch (e) {
      catalogError = $t("fonts.catalog_error", { error: api.errorText(e) }) as string;
      catalogStatus = "error";
    }
  }

  async function loadInstalled(refresh = false) {
    installedStatus = "loading";
    installedError = null;
    try {
      installedList = await api.installed(refresh);
      installedStatus = "ready";
    } catch (e) {
      installedError = $t("fonts.installed_error", { error: api.errorText(e) }) as string;
      installedStatus = "error";
    }
  }

  function retry() {
    if (source === "catalog") void loadCatalog(true);
    else void loadInstalled(true);
  }

  function pickSource(next: Source) {
    source = next;
    if (next === "installed" && category && !["monospace", "serif", "sans-serif"].includes(category)) category = null;
  }

  function toggle(key: string) {
    compare = toggleCompare(compare, key);
  }

  function addCompare(key: string) {
    if (!compare.includes(key) && compare.length < MAX_COMPARE) compare = [...compare, key];
  }

  function swapPair() {
    const h = headingKey;
    headingKey = bodyKey;
    bodyKey = h;
  }

  function onTabKey(event: KeyboardEvent) {
    const i = VIEWS.indexOf(view);
    let next = -1;
    if (event.key === "ArrowRight") next = (i + 1) % VIEWS.length;
    else if (event.key === "ArrowLeft") next = (i - 1 + VIEWS.length) % VIEWS.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = VIEWS.length - 1;
    if (next < 0) return;
    event.preventDefault();
    view = VIEWS[next];
    document.getElementById(`ty-tab-${view}`)?.focus();
  }

  onMount(() => {
    void loadCatalog();
    void loadInstalled();
    void api.userDir().then((d) => (userDir = d)).catch(() => {});
  });
</script>

<section class="ty" aria-busy={listStatus === "loading"}>
  <header class="ty-head">
    <NavIcon icon="fonts" size={56} />
    <div class="ty-head-copy">
      <h1>{$t("fonts.title")}</h1>
      <p class="ty-subtitle">{$t("fonts.subtitle")}</p>
    </div>
  </header>

  <p class="ty-live" role="status" aria-live="polite">{liveSummary}</p>

  <div class="segmented segmented-lg ty-tabs" role="tablist" aria-label={$t("fonts.views_label") as string} tabindex="-1" onkeydown={onTabKey}>
    {#each VIEWS as v (v)}
      <button
        type="button"
        id="ty-tab-{v}"
        class="segmented-btn"
        role="tab"
        aria-selected={view === v}
        aria-controls="ty-panel"
        tabindex={view === v ? 0 : -1}
        onclick={() => (view = v)}
      >
        {$t(`fonts.view_${v}`)}
        {#if v === "compare" && compare.length > 0}<span class="ty-tab-count">{compare.length}</span>{/if}
      </button>
    {/each}
  </div>

  <div id="ty-panel" class="ty-panel" role="tabpanel" aria-labelledby="ty-tab-{view}">
    {#if view === "fonts" || view === "compare"}
      <SampleControls bind:sample placeholder={presetText} />
    {/if}

    {#if view === "fonts"}
      {#if catalogStatus === "error" && source === "catalog"}
        <div class="ty-notice">
          <StatusIcon name="warning" />
          <p>{$t("fonts.catalog_offline", { count: CURATED_IDS.length })}</p>
        </div>
      {/if}
      <div class="ty-browse">
        <aside class="ty-side">
          <FontList
            {source}
            bind:query
            bind:category
            bind:includeOther
            {items}
            {total}
            {hidden}
            status={listStatus}
            error={listError}
            {selectedKey}
            {compare}
            {lang}
            catalogCount={catalogRemote ? catalog.filter((f) => f.license_class === "open").length : null}
            installedCount={installedStatus === "ready" ? installedList.length : null}
            onsource={pickSource}
            onselect={(key) => (selectedKey = key)}
            oncompare={toggle}
            onretry={retry}
            onmore={() => (limit += PAGE)}
          />
          {#if source === "catalog"}
            <p class="ty-side-note">{$t("fonts.satoshi_note")}</p>
          {:else if installedStatus === "ready"}
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => loadInstalled(true)}>{$t("fonts.rescan")}</button>
          {/if}
        </aside>

        <div class="ty-main">
          {#if selected}
            <FontDetail
              font={selected}
              catalogFont={selectedCatalog}
              installedFamily={selectedInstalled}
              alreadyInstalled={selectedCatalog ? names.has(normalize(selectedCatalog.family)) : true}
              {sample}
              {text}
              {lang}
              {userDir}
              inCompare={compare.includes(selected.key)}
              compareFull={compare.length >= MAX_COMPARE}
              isHeading={headingKey === selected.key}
              isBody={bodyKey === selected.key}
              oncompare={() => toggle(selected!.key)}
              onheading={() => (headingKey = selected!.key)}
              onbody={() => (bodyKey = selected!.key)}
              onscale={() => {
                scaleKey = selected!.key;
                view = "scale";
              }}
              oninstalled={() => loadInstalled(true)}
            />
          {:else}
            <div class="ty-pick-empty">
              <p>{$t("fonts.select_hint")}</p>
            </div>
          {/if}
        </div>
      </div>
    {:else if view === "compare"}
      <CompareView fonts={compareFonts} {sample} {text} {lang} {groups} onadd={addCompare} onremove={toggle} onbrowse={() => (view = "fonts")} />
    {:else if view === "pairing"}
      <PairingView {heading} {body} {groups} theme={sample.theme} {lang} onheading={(k) => (headingKey = k)} onbody={(k) => (bodyKey = k)} onswap={swapPair} />
    {:else}
      <ScaleView font={scaleFont} {groups} theme={sample.theme} {lang} onfont={(k) => (scaleKey = k)} />
    {/if}
  </div>
</section>

<style>
  .ty {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .ty-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .ty-head-copy {
    min-width: 0;
  }

  .ty-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .ty-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .ty-live {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .ty-tabs {
    align-self: flex-start;
  }

  .ty-tab-count {
    min-width: 16px;
    padding: 0 4px;
    border-radius: var(--radius-full);
    background: var(--fill-3);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .ty-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .ty-notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-notice p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ty-browse {
    display: grid;
    grid-template-columns: 300px minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }

  @media (max-width: 820px) {
    .ty-browse {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .ty-side {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    position: sticky;
    top: var(--space-2);
  }

  .ty-side-note {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ty-main {
    min-width: 0;
  }

  .ty-pick-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 240px;
    border: 1.5px dashed var(--content-border);
    border-radius: var(--radius-xl);
  }

  .ty-pick-empty p {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }
</style>
