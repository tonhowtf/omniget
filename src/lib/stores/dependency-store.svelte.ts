import { invoke } from "@tauri-apps/api/core";

let ytdlpAvailable: boolean = $state(false);
let checked: boolean = $state(false);

export function isYtdlpAvailable(): boolean {
  return ytdlpAvailable;
}

export function isDepsChecked(): boolean {
  return checked;
}

export async function refreshYtdlpStatus(): Promise<void> {
  try {
    ytdlpAvailable = await invoke<boolean>("check_ytdlp_available");
  } catch {
    ytdlpAvailable = false;
  }
  checked = true;
}
