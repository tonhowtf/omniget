<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { toBody } from "$lib/canvas/to-body";
  import { attachmentSave, blobToBase64 } from "$lib/canvas/chat/client";
  import { designSend, type DesignEvent } from "$lib/canvas/portals/design";

  let {
    event,
    workspaceId,
    onclose,
    onsent,
  }: {
    event: DesignEvent;
    workspaceId: string;
    onclose: () => void;
    onsent?: (name: string) => void;
  } = $props();

  let comment = $state("");
  let files = $state<string[]>([]);
  // svelte-ignore state_referenced_locally
  let target = $state(event.targets[0]?.nodeId ?? "");
  let sending = $state(false);
  let error = $state("");
  let showHtml = $state(false);

  const img = $derived.by(() => {
    if (!event.screenshotPath) return "";
    try {
      return convertFileSrc(event.screenshotPath);
    } catch {
      return "";
    }
  });

  async function attach() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: true, directory: false });
      const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
      files = [...files, ...list.map(String).filter((f) => !files.includes(f))];
    } catch (e) {
      error = String(e);
    }
  }

  async function onPaste(e: ClipboardEvent) {
    const items = Array.from(e.clipboardData?.items ?? []).filter((i) => i.kind === "file");
    if (!items.length) return;
    e.preventDefault();
    for (const it of items) {
      const f = it.getAsFile();
      if (!f) continue;
      try {
        const path = await attachmentSave(workspaceId, f.name || "pasted.png", await blobToBase64(f));
        files = [...files, path];
      } catch (err) {
        error = String(err);
      }
    }
  }

  async function send() {
    if (!event.pick || !target || sending) return;
    sending = true;
    error = "";
    try {
      const r = await designSend({
        nodeId: event.nodeId,
        target,
        comment,
        pick: event.pick,
        screenshotPath: event.screenshotPath ?? null,
        attachments: files,
      });
      onsent?.(r.deliveredTo.name);
      onclose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      sending = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void send();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="scrim" use:toBody data-canvas-chrome onclick={(e) => e.target === e.currentTarget && onclose()} onkeydown={onKey}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label={$t("canvas.portal.design.title", { default: "Ask about this selection" })}>
    <h3>{$t("canvas.portal.design.title", { default: "Ask about this selection" })}</h3>
    {#if img}
      <img class="crop" src={img} alt={event.pick?.selector ?? ""} />
    {:else if event.screenshotError}
      <p class="muted">{$t("canvas.portal.design.noShot", { default: "No screenshot of the region; the selector and HTML still go." })}</p>
    {/if}
    {#if event.pick}
      <button type="button" class="sel" onclick={() => (showHtml = !showHtml)} title={$t("canvas.portal.design.showHtml", { default: "Show the HTML" })}>
        <code>{event.pick.selector || event.pick.tag}</code>
      </button>
      {#if showHtml}
        <pre class="html">{event.pick.html}</pre>
      {/if}
    {/if}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      bind:value={comment}
      rows="3"
      autofocus
      placeholder={$t("canvas.portal.design.comment", { default: "What should change here?" })}
      onpaste={onPaste}
    ></textarea>
    {#if files.length}
      <div class="files">
        {#each files as f, i (f)}
          <span class="chip" title={f}>
            {f.split(/[\\/]/).pop()}
            <button type="button" aria-label={$t("canvas.chat.remove", { default: "Remove" })} onclick={() => (files = files.filter((_, j) => j !== i))}>×</button>
          </span>
        {/each}
      </div>
    {/if}
    {#if event.targets.length}
      <label class="row">
        <span>{$t("canvas.portal.design.to", { default: "Send to" })}</span>
        <select bind:value={target}>
          {#each event.targets as tg (tg.nodeId)}
            <option value={tg.nodeId}>{tg.name}</option>
          {/each}
        </select>
      </label>
    {:else}
      <p class="warn">{$t("canvas.portal.design.noAgent", { default: "Connect an agent to this portal with a cord to send the selection." })}</p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" onclick={attach} title={$t("canvas.portal.design.attach", { default: "Attach files" })}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 12l-8 8a5 5 0 0 1-7-7l9-9a3.5 3.5 0 0 1 5 5l-9 9a2 2 0 0 1-3-3l8-8" /></svg>
      </button>
      <span class="spacer"></span>
      <button type="button" class="ghost" onclick={onclose}>{$t("canvas.portal.design.cancel", { default: "Cancel" })}</button>
      <button type="button" class="primary" disabled={!event.pick || !target || sending} onclick={send}>
        {$t("canvas.portal.design.send", { default: "Send" })}
      </button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.3);
  }
  .sheet {
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
    border-radius: 14px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    color: var(--text);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
  }
  h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .crop {
    max-width: 100%;
    max-height: 200px;
    object-fit: contain;
    align-self: center;
    border-radius: 6px;
    border: 1px solid var(--border);
    transform: rotate(-1.5deg);
  }
  .sel {
    all: unset;
    cursor: pointer;
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .html {
    margin: 0;
    max-height: 140px;
    overflow: auto;
    font-size: 11px;
    padding: 8px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--text) 6%, transparent);
    white-space: pre-wrap;
    word-break: break-all;
  }
  textarea,
  select {
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px;
    resize: vertical;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .row select {
    flex: 1;
    padding: 4px 8px;
  }
  .files {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .chip button {
    all: unset;
    cursor: pointer;
    color: var(--text-muted);
  }
  .muted,
  .warn,
  .error {
    margin: 0;
    font-size: 12px;
  }
  .muted {
    color: var(--text-muted);
  }
  .warn {
    color: #d97706;
  }
  .error {
    color: #ef4444;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .actions button {
    font: inherit;
    font-size: 13px;
    border-radius: 8px;
    padding: 6px 12px;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .actions .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .actions .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .actions .ghost {
    display: inline-grid;
    place-items: center;
  }
</style>
