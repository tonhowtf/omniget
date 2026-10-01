import { invoke } from "@tauri-apps/api/core";

/**
 * One permission vocabulary for chat, jobs and Loops (2.7): what a turn may
 * do without asking. The backend maps it onto each CLI's flags, the native
 * harness grants and ACP options, capped by the account's sandbox.
 */
export type RunMode = "approval_required" | "auto_edits" | "auto" | "full_access";

/** Safest first: the order of every picker. */
export const RUN_MODES: RunMode[] = ["approval_required", "auto_edits", "auto", "full_access"];

/** The chat's default when a conversation never chose. */
const CHAT_DEFAULT: RunMode = "auto_edits";

let modes = $state<Record<string, RunMode>>({});

/** The mode of a chat conversation, as last read or set here. */
export function runModeOf(conversationId: string | null | undefined): RunMode {
  return (conversationId && modes[conversationId]) || CHAT_DEFAULT;
}

export async function loadRunMode(conversationId: string): Promise<void> {
  try {
    const r = await invoke<{ run_mode: RunMode }>("llm_conversation_run_mode", { conversationId });
    modes = { ...modes, [conversationId]: r.run_mode };
  } catch {
    // Unknown: the picker shows the chat default, which is what the backend uses.
  }
}

export async function setRunMode(conversationId: string, runMode: RunMode): Promise<boolean> {
  try {
    await invoke("llm_conversation_run_mode_set", { conversationId, runMode });
    modes = { ...modes, [conversationId]: runMode };
    return true;
  } catch {
    return false;
  }
}
