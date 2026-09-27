/// Sample answers of `wmb_apps_list`, `wmb_app_leftovers` and
/// `wmb_updates_list`, shaped exactly like the core's. Used by the tests
/// only (never imported by the app).

import type { AppsList, LeftoversResponse } from "./apps";
import type { UpdatesList } from "./updates";

export const NOW = 1_790_000_000; // 2026-09-21

export const APPS: AppsList = {
  total: 6,
  apps: [
    { id: "app:com.microsoft.VSCode", name: "Visual Studio Code", version: "1.104.2", publisher: "Microsoft", source: "app_bundle", path: "/Applications/Visual Studio Code.app", bytes: 612_000_000, native_id: "com.microsoft.VSCode", last_used: NOW - 3_600, removable: true },
    { id: "app:com.figma.Desktop", name: "Figma", version: "125.1", source: "homebrew_cask", path: "/Applications/Figma.app", bytes: 480_000_000, native_id: "com.figma.Desktop", last_used: NOW - 240 * 86_400, removable: true, extra: { cask: "figma" } },
    { id: "app:com.apple.Numbers", name: "Numbers", version: "14.4", publisher: "Apple", source: "app_bundle", path: "/Applications/Numbers.app", bytes: 700_000_000, removable: false, reason: "Part of macOS" },
    { id: "pkg:com.zoom.pkg.videomeeting", name: "zoom.us", version: "6.2", source: "pkg", bytes: null, last_used: null, removable: true, needs_admin: true },
    { id: "app:org.videolan.vlc", name: "VLC", version: "3.0.21", source: "app_bundle", bytes: 180_000_000, last_used: NOW - 800 * 86_400, removable: true },
    { id: "app:com.éclair.Émile", name: "Émile Éditeur", source: "app_bundle", bytes: 12_000_000, last_used: NOW - 12 * 86_400, removable: true },
  ],
};

export const FIGMA_LEFTOVERS: LeftoversResponse = {
  app: APPS.apps[1],
  total_bytes: 480_000_000 + 210_000_000 + 4_000 + 90_000 + 1_200,
  plan_id: "plan-1",
  items: [
    { id: "uninstall:app:/Applications/Figma.app", label: "Figma.app", detail: "/Applications/Figma.app", group: "app", bytes: 480_000_000, risk: "reversible", admin: false },
    { id: "uninstall:zap:figma", label: "brew uninstall --zap figma", detail: null, group: "zap", bytes: 0, risk: "destructive", admin: false },
    { id: "uninstall:support:/Users/me/Library/Application Support/Figma", label: "Figma", detail: "/Users/me/Library/Application Support/Figma", group: "support", bytes: 210_000_000, risk: "reversible", admin: false },
    { id: "uninstall:prefs:/Users/me/Library/Preferences/com.figma.Desktop.plist", label: "com.figma.Desktop.plist", detail: "/Users/me/Library/Preferences/com.figma.Desktop.plist", group: "preferences", bytes: 4_000, risk: "reversible", admin: false },
    { id: "uninstall:caches:/Users/me/Library/Caches/com.figma.agent", label: "com.figma.agent", detail: "/Users/me/Library/Caches/com.figma.agent", group: "caches", bytes: 90_000, risk: "reversible", admin: false },
    { id: "uninstall:name:/Users/me/Library/Logs/figma-helper.log", label: "figma-helper.log", detail: "/Users/me/Library/Logs/figma-helper.log", group: "logs", bytes: 1_200, risk: "reversible", admin: false },
  ],
  extra: {
    cask: "figma",
    running: true,
    notes: ["Figma is open."],
    leftovers: [
      { path: "/Users/me/Library/Application Support/Figma", bytes: 210_000_000, confidence: "high", evidence: "bundle id com.figma.Desktop", kind: "folder" },
      { path: "/Users/me/Library/Preferences/com.figma.Desktop.plist", bytes: 4_000, confidence: "high", evidence: "bundle id com.figma.Desktop", kind: "file" },
      { path: "/Users/me/Library/Caches/com.figma.agent", bytes: 90_000, confidence: "medium", evidence: "Team ID T8RA8NE3B7", kind: "folder" },
      { path: "/Users/me/Library/Logs/figma-helper.log", bytes: 1_200, confidence: "low", evidence: "name looks like figma", kind: "file" },
    ],
  },
};

export const UPDATES: UpdatesList = {
  plan_id: "plan-u",
  items: [
    { id: "packages:brew:node", name: "node", manager: "brew", current: "22.9.0", available: "22.10.0", size: 40_000_000, admin: false },
    { id: "packages:brew:ffmpeg", name: "ffmpeg", manager: "brew", current: "7.0", available: "7.1", admin: false },
    { id: "packages:apt:openssl", name: "openssl", manager: "apt", current: "3.0.13", available: "3.0.14", size: 2_000_000, admin: true },
    { id: "packages:winget:Mozilla.Firefox", name: "Mozilla Firefox", manager: "winget", current: null, available: "131.0", admin: false },
  ],
};
