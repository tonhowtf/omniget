<script lang="ts">
  // The canvas being visited, over the whole World screen while visiting
  // (joined from the Share panel, the CLI or MCP): read-only cards and
  // cords, the room chat, wave/react, refresh and leave. Mounted once by
  // Canvas.svelte, so it does not depend on the Share panel being open.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { toBody } from "$lib/canvas/to-body";
  import { errorText, visitChat, visitLeave, visitReact, visitRefresh } from "$lib/canvas/visit/api";
  import { visitSession } from "$lib/canvas/visit/store.svelte";
  import VisitViewer from "./VisitViewer.svelte";

  const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
  const vs = $derived(visitSession.state);
  let chatText = $state("");
  let error = $state("");

  onMount(() => {
    void visitSession.start();
    return () => visitSession.stop();
  });

  // Visiting without a picture yet (joined elsewhere): ask the host for one.
  $effect(() => {
    if (vs.role === "visitor" && !visitSession.scene) void visitRefresh().catch(() => {});
  });

  async function leave() {
    try {
      visitSession.state = await visitLeave();
      visitSession.reset();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function sendChat() {
    const text = chatText.trim();
    if (!text) return;
    chatText = "";
    await visitChat(text).catch((e) => (error = errorText(e)));
  }
</script>

{#if vs.role === "visitor" && visitSession.scene}
  <div class="overlay" use:toBody role="dialog" aria-label={$t("canvas.visit.viewer", { default: "Shared canvas" }) as string} data-canvas-chrome>
    <!-- Full window: the bar is the title bar (drag, room for the traffic lights). -->
    <div class="bar" class:mac={isMac} data-tauri-drag-region>
      <strong>{visitSession.scene.workspace.name} · {visitSession.scene.floor.name}</strong>
      <span class="muted">{$t("canvas.visit.read_only", { default: "read-only" })} · {vs.hostName ?? visitSession.scene.host ?? ""}</span>
      <span class="spacer"></span>
      <button type="button" class="ghost" onclick={() => void visitReact("wave")}>👋</button>
      <button type="button" class="ghost" onclick={() => void visitReact("react", undefined, undefined, "🎉")}>🎉</button>
      <button type="button" class="ghost" onclick={() => void visitRefresh()}>{$t("canvas.visit.refresh", { default: "Refresh" })}</button>
      <button type="button" class="danger" onclick={leave}>{$t("canvas.visit.leave", { default: "Leave" })}</button>
    </div>
    <div class="stage">
      <VisitViewer scene={visitSession.scene} onPoint={(x, y) => void visitReact("point", x, y)} />
      <aside class="side">
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="chat">
          {#each visitSession.chat.slice(-20) as m (m.ts + ":" + m.from)}
            <div class="line"><b>{m.name}</b> {m.text}</div>
          {/each}
          <form class="row" onsubmit={(e) => { e.preventDefault(); void sendChat(); }}>
            <input type="text" bind:value={chatText} maxlength="500" placeholder={$t("canvas.visit.chat", { default: "Message" }) as string} />
            <button type="submit" disabled={!chatText.trim()}>{$t("canvas.visit.send", { default: "Send" })}</button>
          </form>
        </div>
      </aside>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: var(--bg, #000);
    color: var(--text);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .bar.mac {
    padding-left: 84px;
  }
  .spacer {
    flex: 1;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .stage {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .stage > :global(svg) {
    flex: 1;
    min-width: 0;
  }
  .side {
    width: min(280px, 40vw);
    border-left: 1px solid var(--border);
    padding: 8px;
    overflow: auto;
    font-size: 13px;
  }
  .chat {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .row input {
    flex: 1;
    min-width: 0;
  }
  .error {
    color: var(--danger, #e5484d);
  }
  button.danger {
    color: var(--danger, #e5484d);
  }
  @media (max-width: 640px) {
    .stage {
      flex-direction: column;
    }
    .side {
      width: auto;
      border-left: 0;
      border-top: 1px solid var(--border);
      max-height: 35vh;
    }
  }
</style>
