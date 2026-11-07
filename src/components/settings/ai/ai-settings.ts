export const MODEL_HINTS: Record<string, string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-sonnet-4-5",
  deepseek: "deepseek-chat",
  openrouter: "openai/gpt-4o-mini",
  gemini: "gemini-2.5-flash",
  groq: "llama-3.3-70b-versatile",
  xai: "grok-3",
  mistral: "mistral-large-latest",
  siliconflow: "deepseek-ai/DeepSeek-V3",
  newapi: "gpt-4o-mini",
  ollama: "llama3.2",
};

export function modelHint(kind: string): string {
  return MODEL_HINTS[kind] ?? "";
}

export function modelForKind(next: string, saved: { kind: string; model: string } | null): string {
  if (saved && saved.kind === next && saved.model.trim()) return saved.model;
  return modelHint(next);
}

export function modelNeedsAttention(kind: string, model: string): boolean {
  if (!kind || kind === "none") return false;
  return !model.trim() || !modelHint(kind);
}

export function endpointNeedsAttention(kind: string, editable: boolean): boolean {
  if (!kind || kind === "none") return false;
  return kind === "custom" || (editable && kind !== "ollama");
}

export type EnvKind = "tmux" | "ssh" | "docker" | "sandbox" | "custom" | "wsl";

export const ENV_KINDS: readonly EnvKind[] = ["tmux", "ssh", "docker", "sandbox", "custom", "wsl"];

export function visibleEnvKinds(onWindows: boolean, enabled: Partial<Record<EnvKind, boolean>> | null): EnvKind[] {
  return ENV_KINDS.filter((kind) => kind !== "wsl" || onWindows || !!enabled?.wsl);
}

export const BUILTIN_AGENTS = ["shell", "claude", "codex", "gemini", "opencode", "ollama"] as const;

export function agentChoices(presets: readonly string[], current: string): string[] {
  const list: string[] = [...new Set<string>([...BUILTIN_AGENTS, ...presets])];
  if (current && !list.includes(current)) list.push(current);
  return list;
}

export function isBuiltinAgent(name: string): boolean {
  return (BUILTIN_AGENTS as readonly string[]).includes(name.trim());
}
