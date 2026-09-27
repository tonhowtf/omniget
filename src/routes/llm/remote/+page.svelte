<script lang="ts">
  /**
   * Remote access (Central, T11): turn the second listener on/off on a chosen
   * interface, pair a phone with a QR code (read or drive), manage paired
   * devices, see who accessed, and the Tailscale/SSH helpers. No cloud relay.
   *
   * `remote://changed` (pairing, revoke, access, socket open/close) refreshes
   * the page; the listener lives only while this page is mounted, and nothing
   * polls.
   */
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import AccessCard from "$components/central/remote/AccessCard.svelte";
  import PairCard from "$components/central/remote/PairCard.svelte";
  import DevicesCard from "$components/central/remote/DevicesCard.svelte";
  import TunnelCard from "$components/central/remote/TunnelCard.svelte";
  import AccessLogCard from "$components/central/remote/AccessLogCard.svelte";
  import type { AccessEntry, DeviceView, RemoteStatus, SshInfo } from "$components/central/remote/types";

  let status = $state<RemoteStatus | null>(null);
  let devices = $state<DeviceView[]>([]);
  let log = $state<AccessEntry[]>([]);
  let sshBase = $state<{ kind: string; url: string }[]>([]);
  let loadError = $state<string | null>(null);
  let unlisten: (() => void) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function refresh() {
    try {
      const [s, d, l] = await Promise.all([
        invoke<RemoteStatus>("remote_status"),
        invoke<DeviceView[]>("remote_devices"),
        invoke<AccessEntry[]>("remote_access_log", { limit: 100 }),
      ]);
      status = s;
      devices = d;
      log = l;
      loadError = null;
    } catch (e) {
      loadError = String(e);
    }
  }

  function soon() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(refresh, 250);
  }

  onMount(async () => {
    await refresh();
    try {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen("remote://changed", soon);
    } catch {
      // No event bridge (browser preview): the buttons still refresh.
    }
  });

  onDestroy(() => {
    unlisten?.();
    if (timer) clearTimeout(timer);
  });

  function onssh(info: SshInfo | null) {
    sshBase = info ? [{ kind: "ssh", url: info.base }] : [];
  }
</script>

<svelte:head><title>{$t("llm.central.remote.title")}</title></svelte:head>

<div class="page page-wide remote-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.remote.title")}</h1>
      <p class="page-sub">{$t("llm.central.remote.subtitle")}</p>
    </div>
  </header>

  {#if loadError}
    <div class="group"><div class="group-row"><div class="group-row-content"><div class="group-row-sub">{loadError}</div></div></div></div>
  {/if}

  {#if status}
    <AccessCard {status} onchange={(s) => { status = s; soon(); }} />
    <PairCard {status} extraBases={sshBase} />
    <DevicesCard {devices} onchange={soon} />
    <TunnelCard {status} onchange={soon} {onssh} />
    <AccessLogCard entries={log} onchange={soon} />
  {/if}
</div>

<style>
  .remote-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .remote-page :global(.group) {
    margin-bottom: var(--space-5);
  }
  .page-sub {
    margin: 4px 0 0;
    color: var(--text-muted);
    max-width: 640px;
  }
</style>
