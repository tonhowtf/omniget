<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { normalizeUrl, type StorageScope } from "$lib/canvas/portals/client";
  import { listDevices, type DeviceEntry } from "$lib/canvas/devices/client";

  let { onclose }: { onclose: () => void } = $props();

  const engine = useEngine();
  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  let tab = $state<"browser" | "devices">("browser");
  let url = $state("");
  let name = $state("");
  let ua = $state<"" | "ios" | "android">("");
  let scope = $state<StorageScope>("isolated");
  let urlInput = $state<HTMLInputElement | null>(null);

  let devices = $state<DeviceEntry[] | null>(null);
  let deviceError = $state("");
  let query = $state("");
  let platform = $state<"all" | "ios" | "ipados" | "android">("all");
  let picked = $state<DeviceEntry | null>(null);

  onMount(() => {
    urlInput?.focus();
  });

  function loadDevices() {
    if (devices || deviceError) return;
    listDevices()
      .then((l) => (devices = l.devices))
      .catch((e) => (deviceError = String(e)));
  }

  $effect(() => {
    if (tab === "devices") loadDevices();
  });

  function isIpad(d: DeviceEntry): boolean {
    return /ipad/i.test(d.name);
  }

  const groups = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = (devices ?? []).filter((d) => {
      if (q && !d.name.toLowerCase().includes(q)) return false;
      if (platform === "android") return d.platform === "android";
      if (platform === "ios") return d.platform === "ios" && !isIpad(d);
      if (platform === "ipados") return d.platform === "ios" && isIpad(d);
      return true;
    });
    const out = new Map<string, DeviceEntry[]>();
    for (const d of list) {
      const key = d.os || (d.platform === "ios" ? "iOS" : "Android");
      if (!out.has(key)) out.set(key, []);
      out.get(key)!.push(d);
    }
    return Array.from(out.entries());
  });

  const canCreate = $derived(tab === "browser" ? !!normalizeUrl(url) : !!picked);

  function create() {
    if (!canCreate) return;
    if (tab === "browser") {
      const data: Record<string, unknown> = { url: normalizeUrl(url), portalKind: "web", storageScope: scope };
      if (ua) data.userAgent = ua;
      engine.spawn("portal", null, { data, title: name.trim() || undefined });
    } else if (picked) {
      engine.spawn("portal", null, {
        data: { portalKind: picked.platform, deviceId: picked.id },
        title: name.trim() || picked.name,
        rect: undefined,
      });
    }
    onclose();
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    } else if (e.key === "Enter" && !(e.target instanceof HTMLSelectElement)) {
      e.preventDefault();
      create();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="dialog" role="dialog" aria-modal="true" aria-label={tr("canvas.newPortal.title", "New Portal")} tabindex="-1" onkeydown={key}>
  <h2>{tr("canvas.newPortal.title", "New Portal")}</h2>
  <div class="seg" role="tablist">
    <button type="button" role="tab" aria-selected={tab === "browser"} class:on={tab === "browser"} onclick={() => (tab = "browser")}>
      {tr("canvas.newPortal.browser", "Browser")}
    </button>
    <button type="button" role="tab" aria-selected={tab === "devices"} class:on={tab === "devices"} onclick={() => (tab = "devices")}>
      {tr("canvas.newPortal.devices", "Devices")}
    </button>
  </div>

  {#if tab === "browser"}
    <div class="grid">
      <label for="np-url">{tr("canvas.newPortal.url", "URL")}</label>
      <input id="np-url" bind:this={urlInput} bind:value={url} placeholder="https://example.com" spellcheck="false" autocomplete="off" />
      <label for="np-name">{tr("canvas.newPortal.name", "Name")}</label>
      <input id="np-name" bind:value={name} placeholder={tr("canvas.newPortal.optional", "Optional")} />
      <label for="np-ua">{tr("canvas.newPortal.userAgent", "User Agent")}</label>
      <select id="np-ua" bind:value={ua}>
        <option value="">{tr("canvas.newPortal.uaDefault", "Default")}</option>
        <option value="ios">{tr("canvas.newPortal.uaIos", "iPhone (Safari)")}</option>
        <option value="android">{tr("canvas.newPortal.uaAndroid", "Android (Chrome)")}</option>
      </select>
      <label for="np-scope">{tr("canvas.portal.storage.label", "Storage")}</label>
      <select id="np-scope" bind:value={scope}>
        <option value="isolated">{tr("canvas.portal.storage.isolated", "Isolated")}</option>
        <option value="workspace">{tr("canvas.portal.storage.workspace", "Workspace")}</option>
        <option value="global">{tr("canvas.portal.storage.global", "Global")}</option>
      </select>
    </div>
  {:else}
    <div class="devices">
      <input class="search" bind:value={query} placeholder={tr("canvas.newPortal.search", "Search devices…")} spellcheck="false" />
      <div class="filters">
        {#each [["all", tr("canvas.newPortal.all", "All")], ["ios", "iOS"], ["ipados", "iPadOS"], ["android", "Android"]] as [id, label] (id)}
          <button type="button" class:on={platform === id} onclick={() => (platform = id as typeof platform)}>{label}</button>
        {/each}
      </div>
      <div class="list">
        {#if deviceError}
          <p class="muted">{deviceError}</p>
        {:else if !devices}
          <p class="muted">{tr("canvas.newPortal.finding", "Finding devices…")}</p>
        {:else if !groups.length}
          <p class="muted">{tr("canvas.newPortal.none", "No devices found")}</p>
        {:else}
          {#each groups as [os, list] (os)}
            <div class="os">{os}</div>
            {#each list as d (d.platform + d.id)}
              <button type="button" class="dev" class:on={picked?.id === d.id} onclick={() => (picked = d)} ondblclick={create}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 3h8v18H8z M11 18h2" /></svg>
                <span class="dname">{d.name}</span>
                {#if /booted|device/i.test(d.state)}<span class="booted">{tr("canvas.newPortal.booted", "Booted")}</span>{/if}
              </button>
            {/each}
          {/each}
        {/if}
      </div>
      <p class="note">{tr("canvas.newPortal.shared", "Portals on the same device share one screen.")}</p>
    </div>
  {/if}

  <div class="actions">
    <button type="button" class="btn" onclick={onclose}>{tr("canvas.newPortal.cancel", "Cancel")}</button>
    <button type="button" class="btn primary" disabled={!canCreate} onclick={create}>{tr("canvas.newPortal.create", "Create")}</button>
  </div>
</div>

<style>
  .dialog {
    width: min(440px, calc(100vw - 32px));
    box-sizing: border-box;
    padding: 20px 22px 18px;
    border-radius: 16px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.28);
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 14px;
    outline: none;
  }
  h2 {
    margin: 0;
    text-align: center;
    font-size: 15px;
    font-weight: 600;
  }
  .seg {
    align-self: center;
    display: inline-flex;
    padding: 2px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .seg button,
  .filters button {
    all: unset;
    padding: 3px 12px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 10px;
    align-items: center;
  }
  .grid label {
    text-align: right;
    font-size: 13px;
    color: var(--text-muted);
  }
  input,
  select {
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    min-width: 0;
    outline: none;
  }
  input:focus,
  select:focus {
    border-color: var(--accent);
  }
  .devices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--text) 4%, transparent);
  }
  .filters {
    display: flex;
    gap: 2px;
  }
  .filters button {
    font-size: 12px;
    color: var(--text-muted);
  }
  .filters button.on {
    background: color-mix(in srgb, var(--text) 9%, transparent);
    color: var(--text);
    font-weight: 600;
  }
  .list {
    height: 200px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .os {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    padding: 6px 4px 2px;
  }
  .dev {
    all: unset;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .dev:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .dev.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    font-weight: 600;
  }
  .dev svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    flex: none;
  }
  .dname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .booted {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    color: var(--success, #3fa45b);
    background: color-mix(in srgb, var(--success, #3fa45b) 16%, transparent);
  }
  .muted,
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .muted {
    padding: 60px 0;
    text-align: center;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    font: inherit;
    font-size: 13px;
    padding: 5px 14px;
    border-radius: 7px;
    border: none;
    background: color-mix(in srgb, var(--text) 9%, transparent);
    color: var(--text);
    cursor: pointer;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
