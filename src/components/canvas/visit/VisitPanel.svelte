<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import {
    EVENT_REMOTE_CHANGED,
    errorText,
    remoteDevices,
    remotePairCode,
    remoteRevoke,
    remoteStart,
    remoteStatus,
    remoteStop,
    visitChat,
    visitClose,
    visitJoin,
    visitLeave,
    visitOpen,
    visitReact,
    visitShare,
    type PairCodeView,
    type RemoteDeviceView,
    type RemoteStatusView,
  } from "$lib/canvas/visit/api";
  import { visitSession } from "$lib/canvas/visit/store.svelte";

  interface Props {
    workspaceId: string;
    floorId: string;
    terminals?: Array<{ id: string; title: string }>;
    onClose?: () => void;
  }
  let { workspaceId, floorId, terminals = [], onClose }: Props = $props();

  type Tab = "share" | "visit" | "phone";
  let tab = $state<Tab>("share");
  let busy = $state(false);
  let error = $state("");
  let server = $state("");
  let code = $state("");
  let chatText = $state("");
  let copied = $state(false);

  let remote = $state<RemoteStatusView | null>(null);
  let devices = $state<RemoteDeviceView[]>([]);
  let pair = $state<PairCodeView | null>(null);
  let pairScope = $state<"read" | "drive">("read");
  let bindIp = $state("");

  const vs = $derived(visitSession.state);
  const shared = $derived(new Set(vs.shared));

  onMount(() => {
    void visitSession.start();
    let stop: (() => void) | undefined;
    void listen(EVENT_REMOTE_CHANGED, () => {
      if (tab === "phone") void loadRemote();
    }).then((u) => (stop = u));
    return () => {
      visitSession.stop();
      stop?.();
    };
  });

  async function run(f: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await f();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  const open = () => run(async () => { visitSession.reset(); visitSession.state = await visitOpen(workspaceId, floorId, server); });
  const closeHost = () => run(async () => { visitSession.state = await visitClose(); });
  const toggleShare = (id: string, on: boolean) => run(async () => { visitSession.state = await visitShare(id, on); });
  const join = () =>
    run(async () => {
      visitSession.reset();
      const r = await visitJoin(code, server);
      visitSession.state = r.state;
      visitSession.scene = r.scene;
    });
  const leave = () => run(async () => { visitSession.state = await visitLeave(); visitSession.reset(); });

  async function sendChat() {
    const text = chatText.trim();
    if (!text) return;
    chatText = "";
    await visitChat(text).catch((e) => (error = errorText(e)));
  }

  async function copyCode() {
    if (!vs.code) return;
    try {
      await navigator.clipboard.writeText(vs.code);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
    }
  }

  async function loadRemote() {
    try {
      remote = await remoteStatus();
      devices = await remoteDevices();
      if (!bindIp) bindIp = remote.status.interfaces.find((i) => i.kind === "tailscale")?.ip ?? remote.status.bindIp;
    } catch (e) {
      error = errorText(e);
    }
  }

  function pick(next: Tab) {
    tab = next;
    error = "";
    if (next === "phone") void loadRemote();
  }

  const startRemote = () => run(async () => { await remoteStart(bindIp); await loadRemote(); });
  const stopRemote = () => run(async () => { await remoteStop(); pair = null; await loadRemote(); });
  const newCode = () => run(async () => { pair = await remotePairCode(pairScope); });
  const revoke = (id: string) => run(async () => { await remoteRevoke(id); await loadRemote(); });

  function closedText(reason: string): string {
    const d: Record<string, string> = {
      host_left: "The host closed the canvas.",
      ttl: "The visit expired.",
      connection_lost: "The connection dropped.",
      left: "",
    };
    return $t(`canvas.visit.closed.${reason}`, { default: d[reason] ?? "" }) as string;
  }
</script>

<section class="panel" aria-label={$t("canvas.visit.title", { default: "Share" }) as string}>
  <header>
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={tab === "share"} class:on={tab === "share"} onclick={() => pick("share")}>{$t("canvas.visit.tab_share", { default: "Share" })}</button>
      <button type="button" role="tab" aria-selected={tab === "visit"} class:on={tab === "visit"} onclick={() => pick("visit")}>{$t("canvas.visit.tab_visit", { default: "Visit" })}</button>
      <button type="button" role="tab" aria-selected={tab === "phone"} class:on={tab === "phone"} onclick={() => pick("phone")}>{$t("canvas.remote.tab", { default: "Phone" })}</button>
    </div>
    {#if onClose}<button type="button" class="ghost" aria-label={$t("canvas.visit.close_panel", { default: "Close" }) as string} onclick={onClose}>✕</button>{/if}
  </header>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if visitSession.closedReason && closedText(visitSession.closedReason)}
    <p class="muted" role="status">{closedText(visitSession.closedReason)}</p>
  {/if}

  {#if tab === "share"}
    {#if vs.role === "host"}
      <p class="muted">{$t("canvas.visit.host_hint", { default: "Give this code to a friend. They see your agents and cords, read-only." })}</p>
      <div class="code-row">
        <span class="code">{vs.code}</span>
        <button type="button" class="ghost" onclick={copyCode}>{copied ? $t("canvas.visit.copied", { default: "Copied" }) : $t("canvas.visit.copy", { default: "Copy" })}</button>
      </div>
      <h4>{$t("canvas.visit.visitors", { default: "Visitors" })} ({vs.visitors.length})</h4>
      {#each vs.visitors as v (v.visitorId)}<div class="line">{v.name}</div>{:else}<p class="muted">{$t("canvas.visit.no_visitors", { default: "Nobody yet." })}</p>{/each}
      {#if visitSession.reactions.length}
        <div class="line muted">
          {#each visitSession.reactions as r (r.at + ":" + r.visitorId)}<span>{r.emoji ?? (r.type === "wave" ? "👋" : "👉")} {r.name}</span> {/each}
        </div>
      {/if}
      {#if terminals.length}
        <h4>{$t("canvas.visit.shared_screens", { default: "Shared screens" })}</h4>
        <p class="muted">{$t("canvas.visit.shared_hint", { default: "Terminal contents stay private unless you share a node. Visitors never type." })}</p>
        {#each terminals as n (n.id)}
          <label class="check">
            <input type="checkbox" checked={shared.has(n.id)} disabled={busy} onchange={(e) => toggleShare(n.id, e.currentTarget.checked)} />
            <span>{n.title}</span>
          </label>
        {/each}
      {/if}
      {@render chatBox()}
      <button type="button" class="danger" disabled={busy} onclick={closeHost}>{$t("canvas.visit.close", { default: "Stop sharing" })}</button>
    {:else if vs.role === "visitor"}
      <p class="muted">{$t("canvas.visit.busy_visiting", { default: "You are visiting a canvas. Leave it to share yours." })}</p>
    {:else}
      <p class="muted">{$t("canvas.visit.share_hint", { default: "Open this floor for visits: friends see nodes, cords and states, never terminal contents unless you share one." })}</p>
      <details>
        <summary>{$t("canvas.visit.server", { default: "Room server" })}</summary>
        <input type="text" bind:value={server} placeholder="wss://chat.tonho.wtf/v1/room" />
      </details>
      <button type="button" disabled={busy || !workspaceId || !floorId} onclick={open}>{$t("canvas.visit.open", { default: "Open for visits" })}</button>
    {/if}
  {:else if tab === "visit"}
    {#if vs.role === "visitor"}
      <p class="muted">{$t("canvas.visit.visiting", { default: "Visiting {{name}}", name: vs.hostName ?? "" })}</p>
    {:else if vs.role === "host"}
      <p class="muted">{$t("canvas.visit.busy_hosting", { default: "Stop sharing your canvas to visit another one." })}</p>
    {:else}
      <label>
        <span>{$t("canvas.visit.code", { default: "Code" })}</span>
        <input type="text" bind:value={code} placeholder="ABCD-EFGH" autocapitalize="characters" />
      </label>
      <details>
        <summary>{$t("canvas.visit.server", { default: "Room server" })}</summary>
        <input type="text" bind:value={server} placeholder="wss://chat.tonho.wtf/v1/room" />
      </details>
      <button type="button" disabled={busy || code.trim().length < 8} onclick={join}>{busy ? $t("canvas.visit.joining", { default: "Joining…" }) : $t("canvas.visit.join", { default: "Visit" })}</button>
    {/if}
  {:else}
    {#if remote}
      {#if !remote.status.running}
        <p class="muted">{$t("canvas.remote.hint", { default: "Control the canvas from a phone on the same network or your tailnet. Pick the address the phone will use." })}</p>
        <label>
          <span>{$t("canvas.remote.interface", { default: "Address" })}</span>
          <select bind:value={bindIp}>
            {#each remote.status.interfaces as i (i.ip)}<option value={i.ip}>{i.ip} ({i.kind})</option>{/each}
          </select>
        </label>
        <button type="button" disabled={busy || !bindIp} onclick={startRemote}>{$t("canvas.remote.start", { default: "Turn on phone access" })}</button>
        {#if remote.status.lastError}<p class="error">{remote.status.lastError}</p>{/if}
      {:else}
        <p class="muted">{$t("canvas.remote.open_on_phone", { default: "On the phone, open:" })}</p>
        {#each remote.canvasUrls as u (u)}<div class="mono">{u}</div>{/each}
        <div class="row">
          <select bind:value={pairScope} aria-label={$t("canvas.remote.scope", { default: "Access" }) as string}>
            <option value="read">{$t("canvas.remote.scope_read", { default: "Read-only" })}</option>
            <option value="drive">{$t("canvas.remote.scope_full", { default: "Full control" })}</option>
          </select>
          <button type="button" disabled={busy} onclick={newCode}>{$t("canvas.remote.new_code", { default: "Pairing code" })}</button>
        </div>
        {#if pair}
          <div class="code-row">
            <span class="code">{pair.pair.code}</span>
            <span class="muted">{$t("canvas.remote.expires", { default: "valid 5 min, once" })}</span>
          </div>
          {#if pair.qrSvg}<div class="qr" aria-hidden="true">{@html pair.qrSvg}</div>{/if}
        {/if}
        <h4>{$t("canvas.remote.devices", { default: "Paired devices" })}</h4>
        {#each devices as d (d.id)}
          <div class="row line">
            <span>{d.name} <span class="muted">· {d.scope === "drive" ? $t("canvas.remote.scope_full", { default: "Full control" }) : $t("canvas.remote.scope_read", { default: "Read-only" })}</span></span>
            <button type="button" class="ghost" disabled={busy} onclick={() => revoke(d.id)}>{$t("canvas.remote.revoke", { default: "Revoke" })}</button>
          </div>
        {:else}
          <p class="muted">{$t("canvas.remote.no_devices", { default: "No device paired." })}</p>
        {/each}
        <button type="button" class="danger" disabled={busy} onclick={stopRemote}>{$t("canvas.remote.stop", { default: "Turn off phone access" })}</button>
      {/if}
    {:else}
      <p class="muted">…</p>
    {/if}
  {/if}
</section>

{#snippet chatBox()}
  <div class="chat">
    {#each visitSession.chat.slice(-20) as m (m.ts + ":" + m.from)}
      <div class="line"><b>{m.name}</b> {m.text}</div>
    {/each}
    <form class="row" onsubmit={(e) => { e.preventDefault(); void sendChat(); }}>
      <input type="text" bind:value={chatText} maxlength="500" placeholder={$t("canvas.visit.chat", { default: "Message" }) as string} />
      <button type="submit" disabled={!chatText.trim()}>{$t("canvas.visit.send", { default: "Send" })}</button>
    </form>
  </div>
{/snippet}

<style>
  .panel {
    width: min(380px, calc(100vw - 32px));
    max-height: 75vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.3);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  header,
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tabs button {
    background: transparent;
    color: var(--text-muted);
    border: 1px solid transparent;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--border);
  }
  h4 {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label > span {
    color: var(--text-muted);
    font-size: 12px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]),
  select {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
    flex: 1;
    width: 100%;
  }
  button {
    font: inherit;
    border: 0;
    border-radius: 8px;
    padding: 5px 10px;
    background: var(--accent);
    color: #fff;
    cursor: default;
  }
  button:disabled {
    opacity: 0.5;
  }
  button.ghost {
    background: transparent;
    color: var(--text-muted);
    border: 1px solid var(--border);
  }
  button.danger {
    background: #d96a62;
  }
  .code-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .code {
    font: 600 22px/1.2 var(--font-mono, ui-monospace, monospace);
    letter-spacing: 0.08em;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    word-break: break-all;
  }
  .qr {
    width: 180px;
    align-self: center;
    background: #fff;
    border-radius: 8px;
    padding: 6px;
  }
  .qr :global(svg) {
    width: 100%;
    height: auto;
    display: block;
  }
  .line {
    font-size: 13px;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin: 0;
  }
  .error {
    color: #d96a62;
    margin: 0;
  }
  .chat {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  details summary {
    color: var(--text-muted);
    font-size: 12px;
    cursor: default;
  }
</style>
