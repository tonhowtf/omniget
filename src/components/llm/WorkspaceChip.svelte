<script lang="ts">
  // The context of THIS conversation: Personal (no folder: the project tools
  // are absent) or Project: <folder>. Picking a folder here never changes
  // another conversation nor the process-wide folder.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rewindConversation } from "$lib/stores/llm-store.svelte";

  // `conversationId` binds the folder to one conversation; `refreshKey` changes
  // when a turn ends, so the undo counter follows what the agent wrote.
  let { conversationId = null, refreshKey = 0 }: { conversationId?: string | null; refreshKey?: number } = $props();

  type Workspace = {
    /** `projectless` (personal) or `project`. */
    context?: "projectless" | "project";
    path: string | null;
    name: string | null;
    sandbox: string;
    undo_depth?: number;
    /** Why a turn of this folder cannot be undone (no git, snapshot failed). */
    undo_error?: string | null;
  };
  type Undone = {
    files: string[];
    undo_depth: number;
    messages_removed: number;
    conversation_restored: boolean;
    user_messages_kept: number;
    shell_commands: string[];
  };
  let ws = $state<Workspace>({ path: null, name: null, sandbox: "none" });
  let undoing = $state(false);

  async function refresh() {
    // No conversation, no context: never show the process-wide folder here.
    if (!conversationId) {
      ws = { path: null, name: null, sandbox: "none" };
      return;
    }
    try {
      ws = (await invoke("llm_workspace_get", { conversationId })) as Workspace;
    } catch {
      // Backend without the coding tools: the chip stays empty.
    }
  }

  onMount(refresh);
  $effect(() => {
    void conversationId;
    void refreshKey;
    void refresh();
  });

  async function undo() {
    if (!conversationId || undoing) return;
    undoing = true;
    try {
      const out = (await invoke("llm_turn_undo", { conversationId })) as Undone;
      ws = { ...ws, undo_depth: out.undo_depth };
      if (out.conversation_restored) rewindConversation(conversationId, out.user_messages_kept);
      showToast(
        "success",
        $t("llm.workspace.undone_full", { count: out.files.length, messages: out.messages_removed }) as string,
      );
      // Undo only knows the folder. What a command did elsewhere stays done.
      if (out.shell_commands.length > 0) {
        showToast(
          "info",
          $t("llm.workspace.undo_not_restored", { commands: out.shell_commands.join(" · ") }) as string,
          12000,
        );
      }
    } catch (error) {
      showToast("error", String(error));
    } finally {
      undoing = false;
    }
  }

  async function pick() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({ directory: true, multiple: false });
      if (typeof dir !== "string") return;
      ws = (await invoke("llm_workspace_set", { path: dir, conversationId })) as Workspace;
    } catch (error) {
      showToast("error", String(error));
    }
  }

  async function detach() {
    ws = (await invoke("llm_workspace_set", { path: null, conversationId })) as Workspace;
  }
</script>

<div class="ws-chip" class:attached={ws.path !== null} title={ws.path ?? ($t("assist.conversation.context_personal_hint") as string)}>
  <button
    type="button"
    class="ws-main"
    onclick={pick}
    disabled={!conversationId}
    aria-label={(ws.path
      ? $t("assist.conversation.context_project_label", { name: ws.name ?? ws.path })
      : $t("assist.conversation.context_personal_label")) as string}
  >
    <span class="ws-glyph" aria-hidden="true"></span>
    <span class="ws-name">
      {#if ws.path}{$t("assist.conversation.context_project", { name: ws.name ?? ws.path })}{:else}{$t("assist.conversation.context_personal")}{/if}
    </span>
  </button>
  {#if ws.path}
    <span class="ws-sandbox" class:on={ws.sandbox !== "none"}>
      {ws.sandbox === "none" ? $t("llm.workspace.no_sandbox") : $t("llm.workspace.sandboxed")}
    </span>
    {#if ws.undo_error}
      <span class="ws-undo-error" title={ws.undo_error} role="status">{$t("llm.workspace.undo_unavailable")}</span>
    {/if}
    {#if (ws.undo_depth ?? 0) > 0}
      <button type="button" class="ws-undo" onclick={undo} disabled={undoing} title={$t("llm.workspace.undo_hint") as string}>
        {$t("llm.workspace.undo")}
      </button>
    {/if}
    <button type="button" class="ws-x" onclick={detach} aria-label={$t("llm.workspace.detach") as string}>×</button>
  {/if}
</div>

<style>
  .ws-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.12));
    font-size: var(--text-sm);
    color: var(--text-dim);
    /* Never wider than the header column it sits in: the folder name gives
       way (ellipsis), Undo and the detach button do not. */
    max-width: min(320px, 100%);
    min-width: 0;
  }
  .ws-chip.attached {
    color: var(--text);
  }
  .ws-undo {
    border: 0;
    border-radius: var(--radius-full);
    padding: 1px 8px;
    background: var(--accent, #0a84ff);
    color: #fff;
    font: inherit;
    font-size: var(--text-xs, 11px);
    cursor: pointer;
  }
  .ws-undo-error {
    border-radius: var(--radius-full);
    padding: 1px 8px;
    background: color-mix(in srgb, var(--danger, #ff453a) 18%, transparent);
    color: var(--danger, #ff453a);
    font-size: var(--text-xs, 11px);
    white-space: nowrap;
  }
  .ws-undo:disabled {
    opacity: 0.5;
  }
  .ws-main {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
    min-width: 0;
  }
  .ws-glyph {
    width: 14px;
    height: 14px;
    background: currentColor;
    -webkit-mask: url(/icons/folder-simple.svg) center / contain no-repeat;
    mask: url(/icons/folder-simple.svg) center / contain no-repeat;
    flex-shrink: 0;
  }
  .ws-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ws-sandbox {
    font-size: var(--text-xs, 11px);
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: rgba(255, 159, 10, 0.18);
    color: #c77700;
    white-space: nowrap;
  }
  .ws-sandbox.on {
    background: rgba(52, 199, 89, 0.18);
    color: #1f8f43;
  }
  .ws-main:focus-visible, .ws-x:focus-visible, .ws-undo:focus-visible { outline: var(--focus-ring); border-radius: var(--radius-sm); }
  .ws-main:disabled { cursor: default; }
  .ws-x {
    background: none;
    border: 0;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
</style>
