/**
 * Typed wrappers for the plugin inventory of the coding tools
 * (`agentkit_plugins_list`, `agentkit_plugin_set_enabled`).
 */
import { invoke } from "@tauri-apps/api/core";

export type PluginScope = "user" | "project" | "local";

export interface PluginComponents {
  skills?: string[];
  agents?: string[];
  commands?: string[];
  hooks?: string[];
  mcp_servers?: string[];
  lsp_servers?: string[];
  rules?: string[];
  other?: string[];
}

export interface PluginInfo {
  tool: string;
  id: string;
  name: string;
  version?: string;
  description: string;
  marketplace?: string;
  source?: string;
  scope: PluginScope;
  installed: boolean;
  /** `undefined` = the tool keeps the state where OmniGet cannot read it (Cursor). */
  enabled?: boolean;
  enabled_in?: Partial<Record<PluginScope, boolean>>;
  toggle_scopes: PluginScope[];
  path?: string;
  components: PluginComponents;
  notes?: string[];
}

export interface ToggleReport {
  tx: string;
  file: string;
  plugin: PluginInfo | null;
}

export const COMPONENT_KEYS = [
  "skills",
  "agents",
  "commands",
  "hooks",
  "mcp_servers",
  "lsp_servers",
  "rules",
  "other",
] as const satisfies readonly (keyof PluginComponents)[];

export function pluginsList(projectDir: string | null): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("agentkit_plugins_list", { projectDir });
}

export function pluginSetEnabled(
  tool: string,
  plugin: string,
  enabled: boolean,
  scope: PluginScope,
  projectDir: string | null,
): Promise<ToggleReport> {
  return invoke<ToggleReport>("agentkit_plugin_set_enabled", { tool, plugin, enabled, scope, projectDir });
}

/** Default scope to switch in: the project when one is open and the tool allows it. */
export function defaultScope(p: PluginInfo, projectDir: string | null): PluginScope {
  if (projectDir && p.toggle_scopes.includes("project")) return "project";
  return p.toggle_scopes.includes("user") ? "user" : (p.toggle_scopes[0] ?? "user");
}
