<script lang="ts">
  import { listenTogether } from "$lib/study-music/listen-together-store.svelte";
  import { t } from "$lib/i18n";

  type Props = {
    open: boolean;
    onClose: () => void;
  };

  let { open, onClose }: Props = $props();

  let codeInput = $state("");
  let chatInput = $state("");
  let copied = $state(false);

  function copyCode() {
    if (!listenTogether.roomCode) return;
    void navigator.clipboard.writeText(listenTogether.roomCode).then(() => {
      copied = true;
      setTimeout(() => (copied = false), 1200);
    });
  }

  function createRoom() {
    listenTogether.createRoom();
  }

  function joinRoom() {
    const code = codeInput.trim().toUpperCase();
    if (code.length < 4) return;
    listenTogether.joinRoom(code);
    codeInput = "";
  }

  function leave() {
    listenTogether.disconnect();
  }

  function sendChat() {
    listenTogether.sendChat(chatInput);
    chatInput = "";
  }

  function onChatKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendChat();
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
    onkeydown={onKey}
  >
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <header class="head">
        <h3>{$t("study.music.listen_together_title")}</h3>
        <button
          type="button"
          class="close"
          onclick={onClose}
          aria-label={$t("study.common.close") as string}
        >×</button>
      </header>

      <div class="body">
        {#if listenTogether.status === "idle" || listenTogether.status === "error"}
          <p class="intro">{$t("study.music.listen_together_intro")}</p>

          <label class="field">
            <span class="field-label">{$t("study.music.listen_together_relay_url")}</span>
            <input
              type="text"
              class="text-input"
              placeholder="ws://localhost:8787"
              value={listenTogether.relayUrl}
              oninput={(e) =>
                listenTogether.setRelayUrl((e.currentTarget as HTMLInputElement).value)}
            />
            <span class="field-hint">{$t("study.music.listen_together_relay_hint")}</span>
          </label>

          <label class="field">
            <span class="field-label">{$t("study.music.listen_together_name")}</span>
            <input
              type="text"
              class="text-input"
              maxlength="40"
              value={listenTogether.displayName}
              oninput={(e) =>
                listenTogether.setDisplayName((e.currentTarget as HTMLInputElement).value)}
            />
          </label>

          {#if listenTogether.errorMessage}
            <div class="error-row">{listenTogether.errorMessage}</div>
          {/if}

          <div class="actions">
            <button
              type="button"
              class="primary"
              onclick={createRoom}
              disabled={!listenTogether.relayUrl || !listenTogether.displayName}
            >
              {$t("study.music.listen_together_create")}
            </button>
            <div class="join-row">
              <input
                type="text"
                class="text-input code"
                placeholder="ABCDEF"
                maxlength="12"
                bind:value={codeInput}
              />
              <button
                type="button"
                class="ghost"
                onclick={joinRoom}
                disabled={!listenTogether.relayUrl || !listenTogether.displayName || codeInput.trim().length < 4}
              >
                {$t("study.music.listen_together_join")}
              </button>
            </div>
          </div>
        {:else if listenTogether.status === "connecting"}
          <div class="status connecting">{$t("study.music.listen_together_connecting")}</div>
        {:else if listenTogether.status === "connected"}
          <div class="connected-head">
            <div class="room-info">
              <span class="room-label">{$t("study.music.listen_together_room")}</span>
              <button type="button" class="room-code" onclick={copyCode} title="Copy code">
                {listenTogether.roomCode}
                {#if copied}<em>{$t("study.music.listen_together_copied")}</em>{/if}
              </button>
            </div>
            <span class="role-badge" class:host={listenTogether.isHost}>
              {listenTogether.isHost
                ? $t("study.music.listen_together_role_host")
                : $t("study.music.listen_together_role_guest")}
            </span>
          </div>

          <div class="members">
            <h4>{$t("study.music.listen_together_members")} ({listenTogether.members.length})</h4>
            <ul>
              {#each listenTogether.members as m (m.id)}
                <li>
                  <span class="dot" class:host={m.role === "host"}></span>
                  <span class="member-name">{m.name}</span>
                  {#if m.role === "host"}<span class="member-role">host</span>{/if}
                  {#if m.id === listenTogether.selfId}<span class="member-you">{$t("study.music.listen_together_you")}</span>{/if}
                </li>
              {/each}
            </ul>
          </div>

          <div class="chat">
            <h4>{$t("study.music.listen_together_chat")}</h4>
            <div class="chat-log">
              {#each listenTogether.chat as line (line.at)}
                <div class="chat-line">
                  <strong>{line.name}:</strong>
                  <span>{line.text}</span>
                </div>
              {:else}
                <span class="chat-empty">{$t("study.music.listen_together_chat_empty")}</span>
              {/each}
            </div>
            <div class="chat-input-row">
              <input
                type="text"
                class="text-input"
                placeholder={$t("study.music.listen_together_chat_placeholder") as string}
                bind:value={chatInput}
                onkeydown={onChatKey}
              />
              <button type="button" class="ghost" onclick={sendChat} disabled={!chatInput.trim()}>
                {$t("study.music.listen_together_send")}
              </button>
            </div>
          </div>

          <div class="actions end">
            <button type="button" class="danger" onclick={leave}>
              {$t("study.music.listen_together_leave")}
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 999;
  }
  .dialog {
    background: var(--surface);
    border: none;
    border-radius: 12px;
    width: min(520px, 92vw);
    max-height: 86vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 18px;
    border-bottom: none;
  }
  .head h3 {
    margin: 0;
    font-size: 16px;
  }
  .close {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 22px;
    cursor: pointer;
    line-height: 1;
  }
  .body {
    padding: 16px 18px 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .intro {
    margin: 0;
    color: rgba(255, 255, 255, 0.6);
    font-size: 13px;
    line-height: 1.5;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-label {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.55);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .field-hint {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.4);
  }
  .text-input {
    appearance: none;
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: inherit;
    padding: 8px 12px;
    border-radius: 8px;
    font: inherit;
    font-size: 14px;
  }
  .text-input.code {
    font-family: ui-monospace, "Cascadia Code", monospace;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    width: 140px;
  }
  .error-row {
    background: rgba(255, 100, 80, 0.1);
    border: 1px solid rgba(255, 100, 80, 0.3);
    color: #ffb5a4;
    padding: 8px 12px;
    border-radius: 8px;
    font-size: 12px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 4px;
  }
  .actions.end {
    flex-direction: row;
    justify-content: flex-end;
    margin-top: 6px;
  }
  .primary {
    appearance: none;
    background: var(--accent);
    border: none;
    color: var(--on-accent);
    padding: 9px 16px;
    border-radius: 8px;
    cursor: pointer;
    font: inherit;
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ghost {
    appearance: none;
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: inherit;
    padding: 8px 14px;
    border-radius: 8px;
    cursor: pointer;
    font: inherit;
  }
  .ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger {
    appearance: none;
    background: transparent;
    border: 1px solid rgba(255, 100, 80, 0.3);
    color: #ffb5a4;
    padding: 7px 14px;
    border-radius: 8px;
    cursor: pointer;
    font: inherit;
  }
  .danger:hover {
    background: rgba(255, 100, 80, 0.1);
  }
  .join-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .status {
    text-align: center;
    color: rgba(255, 255, 255, 0.7);
    padding: 24px 0;
  }
  .connected-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 12px;
    background: rgba(111, 78, 255, 0.08);
    border: 1px solid rgba(111, 78, 255, 0.18);
    border-radius: 8px;
  }
  .room-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .room-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: rgba(255, 255, 255, 0.5);
  }
  .room-code {
    appearance: none;
    background: transparent;
    border: none;
    color: inherit;
    font-family: ui-monospace, "Cascadia Code", monospace;
    font-size: 18px;
    letter-spacing: 0.12em;
    font-weight: 700;
    cursor: pointer;
    padding: 0;
    text-align: left;
  }
  .room-code em {
    font-size: 11px;
    font-style: normal;
    color: #6f4eff;
    margin-left: 8px;
    letter-spacing: 0;
  }
  .role-badge {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 3px 8px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.7);
  }
  .role-badge.host {
    background: rgba(255, 213, 100, 0.15);
    color: #ffd564;
  }
  .members h4,
  .chat h4 {
    margin: 0 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: rgba(255, 255, 255, 0.5);
    font-weight: 500;
  }
  .members ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .members li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.4);
  }
  .dot.host {
    background: #ffd564;
  }
  .member-name {
    flex: 1;
    color: rgba(255, 255, 255, 0.85);
  }
  .member-role {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: rgba(255, 213, 100, 0.8);
  }
  .member-you {
    font-size: 11px;
    color: rgba(111, 78, 255, 0.8);
  }
  .chat-log {
    max-height: 160px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 8px;
    padding: 8px 10px;
  }
  .chat-line {
    font-size: 13px;
    line-height: 1.35;
  }
  .chat-line strong {
    color: rgba(255, 255, 255, 0.85);
    margin-right: 4px;
  }
  .chat-line span {
    color: rgba(255, 255, 255, 0.7);
    word-break: break-word;
  }
  .chat-empty {
    color: rgba(255, 255, 255, 0.35);
    font-size: 12px;
    font-style: italic;
    text-align: center;
    padding: 16px 0;
  }
  .chat-input-row {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  .chat-input-row .text-input {
    flex: 1;
  }
</style>
