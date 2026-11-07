import { invoke } from "@tauri-apps/api/core";
import { getSettings } from "./settings-store.svelte";

let reopened = $state(false);

export function reopenOnboarding() {
  reopened = true;
}

export function needsOnboarding(): boolean {
  const settings = getSettings();
  if (!settings) return false;
  return reopened || !settings.onboarding_completed;
}

export async function completeOnboarding(): Promise<void> {
  await invoke("mark_onboarding_complete");
  reopened = false;
  const settings = getSettings();
  if (settings) {
    settings.onboarding_completed = true;
  }
}
