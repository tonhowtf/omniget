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

export function showChangelog(): void {
  showDialog = true;
}

export function storeChangelogForUpdate(body: string, version: string): void {
  localStorage.setItem(CHANGELOG_BODY_KEY, JSON.stringify({ body, version }));
}

export async function initChangelog(): Promise<void> {
  try {
    currentVersion = await getVersion();
  } catch {
    currentVersion = "0.2.8";
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

  try {
    const res = await fetch(
      `https://api.github.com/repos/tonhowtf/omniget/releases/tags/v${currentVersion}`,
      { headers: { Accept: "application/vnd.github.v3+json" } }
    );
    if (res.ok) {
      const data = await res.json();
      if (data.body) {
        changelogBody = data.body;
        return data.body;
      }
    }
  } catch {}

  try {
    const res = await fetch(
      "https://api.github.com/repos/tonhowtf/omniget/releases/latest",
      { headers: { Accept: "application/vnd.github.v3+json" } }
    );
    if (res.ok) {
      const data = await res.json();
      if (data.body) {
        changelogBody = data.body;
        return data.body;
      }
    }
  } catch {}

  return "";
}
