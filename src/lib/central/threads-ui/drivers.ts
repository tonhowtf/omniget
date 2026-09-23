// Display metadata for drivers/instances: label, tint and the model hints
// offered by the composer's model field. Tints follow each vendor's brand
// colour so a thread's harness reads at a glance.

import type { AccessMode, InstanceView } from "$lib/central/threads/types";

const TINTS: Record<string, string> = {
  native: "var(--accent)",
  claude: "#D97757",
  codex: "#10A37F",
  gemini: "#4285F4",
  qwen: "#7C5CFC",
  opencode: "#8E8E93",
  kilo: "#F8C33C",
  crush: "#FF5FAF",
  cursor: "#6E6E73",
  copilot: "#8957E5",
  cline: "#5A9CF8",
  goose: "#2F2F33",
  droid: "#FF8A3D",
  kimi: "#1E6FE8",
  amp: "#F34E3F",
  grok: "#5C5C60",
  acp: "#34C759",
};

const NAMES: Record<string, string> = {
  native: "OmniGet",
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  qwen: "Qwen Code",
  opencode: "OpenCode",
  cursor: "Cursor",
  copilot: "Copilot",
  kimi: "Kimi",
  grok: "Grok",
  droid: "Droid",
  amp: "Amp",
  goose: "Goose",
  kilo: "Kilo",
  crush: "Crush",
  cline: "Cline",
};

export function driverTint(driver: string, inst?: InstanceView | null): string {
  return inst?.instance.color ?? TINTS[driver] ?? "var(--text-muted)";
}

export function driverName(driver: string, inst?: InstanceView | null): string {
  return inst?.instance.label ?? NAMES[driver] ?? driver.charAt(0).toUpperCase() + driver.slice(1);
}

export function driverInitial(driver: string): string {
  const n = NAMES[driver] ?? driver;
  return n.charAt(0).toUpperCase();
}

/** Model hints per driver (the field stays free text). */
export const MODEL_HINTS: Record<string, string[]> = {
  claude: ["sonnet", "opus", "haiku", "claude-sonnet-4-5", "claude-opus-4-1"],
  codex: ["gpt-5-codex", "gpt-5", "gpt-5-mini", "o3"],
  gemini: ["gemini-2.5-pro", "gemini-2.5-flash"],
  qwen: ["qwen3-coder-plus"],
  opencode: ["anthropic/claude-sonnet-4-5", "openai/gpt-5"],
  grok: ["grok-4", "grok-code-fast-1"],
  kimi: ["kimi-k2"],
};

export const ACCESS_MODES: AccessMode[] = ["approval-required", "auto-accept-edits", "auto", "full-access"];

export function accessIcon(mode: AccessMode): "lock-simple" | "shield-check" | "lightning" | "lock-simple-open" {
  switch (mode) {
    case "approval-required":
      return "lock-simple";
    case "auto-accept-edits":
      return "shield-check";
    case "auto":
      return "lightning";
    default:
      return "lock-simple-open";
  }
}
