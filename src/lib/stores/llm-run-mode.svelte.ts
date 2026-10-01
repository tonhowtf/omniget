import { invoke } from "@tauri-apps/api/core";

export type RunMode = "approval_required" | "auto_edits" | "auto" | "full_access";

export const RUN_MODES: RunMode[] = ["approval_required", "auto_edits", "auto", "full_access"];

const CHAT_DEFAULT: RunMode = "auto_edits";

let modes = $state<Record<string, RunMode>>({});

export function runModeOf(conversationId: string | null | undefined): RunMode {
  return (conversationId && modes[conversationId]) || CHAT_DEFAULT;
}

export async function loadRunMode(conversationId: string): Promise<void> {
  try {
    const r = await invoke<{ run_mode: RunMode }>("llm_conversation_run_mode", { conversationId });
    modes = { ...modes, [conversationId]: r.run_mode };
  } catch {
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
