import { getVersion } from "@tauri-apps/api/app";

const STORAGE_KEY = "omniget_last_seen_version";
const CHANGELOG_BODY_KEY = "omniget_pending_changelog";

let showDialog = $state(false);
let changelogBody = $state("");
let currentVersion = $state("");

export function getChangelogVisible(): boolean {
  return showDialog;
}

export function getChangelogBody(): string {
  return changelogBody;
}

export function getCurrentVersion(): string {
  return currentVersion;
}

export function dismissChangelog(): void {
  showDialog = false;
  if (currentVersion) {
    localStorage.setItem(STORAGE_KEY, currentVersion);
  }
  localStorage.removeItem(CHANGELOG_BODY_KEY);
}

export function storeChangelogForUpdate(body: string, version: string): void {
  localStorage.setItem(CHANGELOG_BODY_KEY, JSON.stringify({ body, version }));
}

export async function initChangelog(): Promise<void> {
  try {
    currentVersion = await getVersion();
  } catch {
    currentVersion = "1.0.0";
  }

  const lastSeen = localStorage.getItem(STORAGE_KEY);
  const pending = localStorage.getItem(CHANGELOG_BODY_KEY);

  if (pending) {
    try {
      const { body, version } = JSON.parse(pending);
      if (version === currentVersion && body) {
        changelogBody = body;
        if (lastSeen !== currentVersion) {
          showDialog = true;
          localStorage.setItem(STORAGE_KEY, currentVersion);
        }
        localStorage.removeItem(CHANGELOG_BODY_KEY);
        return;
      }
    } catch {}
    localStorage.removeItem(CHANGELOG_BODY_KEY);
  }

  if (!lastSeen) {
    localStorage.setItem(STORAGE_KEY, currentVersion);
  }
}

export async function fetchChangelog(): Promise<string> {
  if (changelogBody) return changelogBody;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 5_000);
  try {
    const res = await fetch("https://getomniget.com/update/latest.json", { signal: controller.signal });
    if (!res.ok) return "";
    const data = await res.json();
    if (data?.version === currentVersion && typeof data.notes === "string" && data.notes) {
      changelogBody = data.notes;
      return data.notes;
    }
  } catch {
  } finally {
    clearTimeout(timeout);
  }
  return "";
}
