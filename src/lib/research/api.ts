import { invoke } from "@tauri-apps/api/core";
import type { Result, ToolName } from "./types";

export { parseError, reveal } from "$lib/devtools/api";

export function keyStatus(): Promise<{ configured: boolean }> {
  return invoke<{ configured: boolean }>("research_key_status");
}

export function keySet(key: string): Promise<{ configured: boolean }> {
  return invoke<{ configured: boolean }>("research_key_set", { key });
}

export function keyClear(): Promise<{ configured: boolean }> {
  return invoke<{ configured: boolean }>("research_key_clear");
}

export function tool<T = Result>(name: ToolName, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>("research_tool", { name, args });
}
