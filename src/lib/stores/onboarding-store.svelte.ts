import { invoke } from "@tauri-apps/api/core";
import { getSettings } from "./settings-store.svelte";

export function needsOnboarding(): boolean {
  const settings = getSettings();
  if (!settings) return false;
  return !settings.onboarding_completed;
}

export async function completeOnboarding(): Promise<void> {
  await invoke("mark_onboarding_complete");
  const settings = getSettings();
  if (settings) {
    settings.onboarding_completed = true;
  }
}
