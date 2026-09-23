<script lang="ts">
  /**
   * Out of the house without a relay: `tailscale serve` of the remote port
   * (the command is shown first and only runs after a confirmation) and an
   * `ssh -L` tunnel command. `tailscale` is spawned only when asked here.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, type CommandRun, type RemoteStatus, type SshInfo, type TailscaleInfo } from "./types";

  let {
    status,
    onchange,
    onssh,
  }: { status: RemoteStatus; onchange: () => void; onssh: (info: SshInfo | null) => void } = $props();

  let ts = $state<TailscaleInfo | null>(null);
  let checking = $state(false);
  let running = $state(false);
  let lastRun = $state<CommandRun | null>(null);
  let ssh = $state<SshInfo | null>(null);
  let sshPort = $state<number | null>(null);
  let sshHost = $state("");

  async function detect() {
    checking = true;
    try {
      ts = await invoke<TailscaleInfo>("remote_tailscale_status", { httpsPort: null });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      checking = false;
    }
  }

  async function serve(enable: boolean) {
    if (!ts) return;
    const command = enable ? ts.serveCommand : ts.serveOffCommand;
    if (!command) return;
    const ok = await ask($t("llm.central.remote.ts_confirm", { command }) as string, {
      title: $t("llm.central.remote.ts_title") as string,
      kind: "warning",
    });
    if (!ok) return;
    running = true;
    try {
      lastRun = await invoke<CommandRun>("remote_tailscale_serve", { enable, confirm: true, httpsPort: ts.httpsPort });
      if (!lastRun.ok) showToast("error", $t("llm.central.remote.ts_failed") as string);
      onchange();
      await detect();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      running = false;
    }
  }

  async function makeSsh() {
    try {
      ssh = await invoke<SshInfo>("remote_ssh_command", { localPort: sshPort || null, host: sshHost.trim() || null });
      onssh(ssh);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("llm.central.remote.copied") as string);
  }
</script>

<div class="group">
  <div class="group-label">{$t("llm.central.remote.tunnels")}</div>

  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">Tailscale</div>
      <div class="group-row-sub">{$t("llm.central.remote.ts_hint")}</div>
      {#if ts}
        {#if !ts.installed}
          <div class="group-row-sub">{$t("llm.central.remote.ts_missing")}</div>
        {:else}
          <div class="group-row-sub">
            {ts.running ? $t("llm.central.remote.ts_running", { name: ts.dnsName ?? "—" }) : $t("llm.central.remote.ts_stopped", { state: ts.backendState ?? "?" })}
            {#if ts.ips.length}· {ts.ips.join(", ")}{/if}
          </div>
          {#if ts.error}<div class="group-row-sub err">{ts.error}</div>{/if}
          {#if ts.serveCommand}
            <pre class="cmd">{ts.serveCommand}</pre>
          {:else}
            <div class="group-row-sub">{$t("llm.central.remote.pair_needs_on")}</div>
          {/if}
          {#if ts.serveUrl && (ts.serveEnabled || status.tailscaleServeUrl)}
            <div class="group-row-sub">{$t("llm.central.remote.ts_url", { url: status.tailscaleServeUrl ?? ts.serveUrl })}</div>
          {/if}
          {#if lastRun}
            <pre class="cmd out">{lastRun.command}
{lastRun.stdout}{lastRun.stderr}</pre>
          {/if}
        {/if}
      {/if}
    </div>
    <div class="group-row-trailing col">
      <button type="button" class="button" disabled={checking} onclick={detect}>{$t("llm.central.remote.ts_detect")}</button>
      {#if ts?.installed && ts.running}
        <button type="button" class="button primary" disabled={running || !ts.serveCommand} onclick={() => serve(true)}>
          {$t("llm.central.remote.ts_serve_on")}
        </button>
        <button type="button" class="button" disabled={running} onclick={() => serve(false)}>
          {$t("llm.central.remote.ts_serve_off")}
        </button>
      {/if}
    </div>
  </div>

  <div class="group-row">
    <div class="group-row-content">
      <div class="group-row-title">SSH</div>
      <div class="group-row-sub">{$t("llm.central.remote.ssh_hint")}</div>
      <div class="ssh-form">
        <input type="text" placeholder={$t("llm.central.remote.ssh_host_ph")} bind:value={sshHost} />
        <input type="number" min="1" max="65535" placeholder={String(status.port)} bind:value={sshPort} />
        <button type="button" class="button" onclick={makeSsh}>{$t("llm.central.remote.ssh_make")}</button>
      </div>
      {#if ssh}
        <pre class="cmd">{ssh.command}</pre>
        <div class="group-row-sub">{$t("llm.central.remote.ssh_then", { url: `${ssh.base}/remote/` })}</div>
        <button type="button" class="button" onclick={() => ssh && copy(ssh.command)}>{$t("llm.central.remote.copy")}</button>
      {/if}
    </div>
  </div>
</div>

<style>
  .col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: stretch;
  }
  .cmd {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    background: var(--surface-2, rgba(127, 127, 127, 0.12));
    border-radius: 8px;
    padding: 8px 10px;
    white-space: pre-wrap;
    word-break: break-all;
    margin: 6px 0;
    user-select: all;
  }
  .out {
    max-height: 160px;
    overflow: auto;
  }
  .err {
    color: var(--danger, #d33b3b);
  }
  .ssh-form {
    display: flex;
    gap: 8px;
    margin: 8px 0;
    flex-wrap: wrap;
  }
  .ssh-form input[type="text"] {
    width: 200px;
  }
  .ssh-form input[type="number"] {
    width: 96px;
  }
</style>
