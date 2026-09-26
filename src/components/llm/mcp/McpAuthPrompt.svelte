<script lang="ts">
  // Desktop half of `auth_connection_request`: an AI client asked the person
  // to connect a login. The person opens the cookie settings, then marks the
  // request done or denies it. Nothing here is sent to the client.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type AuthRequest = { id: string; client: string; platform: string; host: string | null; reason: string; created_at: number; expires_at: number };
  const PLATFORM_LABELS: Record<string, string> = {
    youtube: "YouTube", youtube_music: "YouTube Music", soundcloud: "SoundCloud", spotify: "Spotify",
    twitch: "Twitch", instagram: "Instagram", x_twitter: "X / Twitter", vimeo: "Vimeo", tiktok: "TikTok",
    bilibili: "Bilibili", douyin: "Douyin", reddit: "Reddit", pinterest: "Pinterest", bluesky: "Bluesky",
  };
  let requests = $state<AuthRequest[]>([]);
  let busy = $state<string | null>(null);
  let error = $state("");

  function target(r: AuthRequest): string {
    return r.host ?? PLATFORM_LABELS[r.platform] ?? r.platform;
  }
  function time(secs: number): string {
    return new Date(secs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  async function refresh() {
    try { requests = await invoke<AuthRequest[]>("tool_mcp_auth_requests"); }
    catch { requests = []; }
  }
  async function answer(id: string, outcome: "resolved" | "denied") {
    busy = id; error = "";
    try { await invoke("tool_mcp_auth_request_answer", { id, outcome }); }
    catch (e) { error = String(e); }
    finally { busy = null; await refresh(); }
  }
  function openSettings() {
    void goto("/settings?tab=cookies");
  }

  onMount(() => {
    void refresh();
    let unlisten: (() => void) | null = null;
    listen("mcp-auth-request", () => void refresh()).then((un) => { unlisten = un; });
    // Expired requests leave the list without an event.
    const timer = setInterval(() => void refresh(), 60_000);
    return () => { unlisten?.(); clearInterval(timer); };
  });
</script>

{#if requests.length}
  <aside class="mcp-auth" aria-live="polite" aria-label={$t("mcp_auth.title")}>
    {#each requests as r (r.id)}
      <div class="card" role="alertdialog" aria-labelledby={`mcp-auth-${r.id}`}>
        <h4 id={`mcp-auth-${r.id}`}>{$t("mcp_auth.title")}</h4>
        <p>{$t("mcp_auth.body", { client: r.client, target: target(r) })}</p>
        {#if r.reason}
          <p class="reason"><span>{$t("mcp_auth.reason_label")}</span> {r.reason}</p>
        {/if}
        <p class="note">{$t("mcp_auth.note")}</p>
        <p class="note">{$t("mcp_auth.expires", { time: time(r.expires_at) })}</p>
        <div class="actions">
          <button class="btn btn-secondary btn-sm" type="button" onclick={openSettings}>{$t("mcp_auth.open_settings")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy === r.id} onclick={() => answer(r.id, "resolved")}>{$t("mcp_auth.done")}</button>
          <button class="btn btn-ghost btn-sm" type="button" disabled={busy === r.id} onclick={() => answer(r.id, "denied")}>{$t("mcp_auth.deny")}</button>
        </div>
      </div>
    {/each}
    {#if error}<p role="status" class="note">{error}</p>{/if}
  </aside>
{/if}

<style>
  .mcp-auth {
    position: fixed;
    bottom: calc(var(--space-3) + 28px);
    right: calc(var(--pane-inset, 8px) + var(--space-2));
    z-index: 9998;
    display: grid;
    gap: var(--space-2);
    max-width: min(380px, calc(100vw - 2 * var(--space-3)));
  }
  .card {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    border-radius: var(--radius-lg);
    box-shadow: var(--elev-2);
    color: var(--text);
  }
  h4, p { margin: 0; }
  h4 { font-size: var(--text-sm); }
  p { font-size: var(--text-sm); overflow-wrap: anywhere; }
  .reason span, .note { color: var(--text-muted); }
  .note { font-size: var(--text-xs); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
</style>
