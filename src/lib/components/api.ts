import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FetchResult, OutputLine, ProjectInfo, RegistrySource, RunOutcome } from "./types";

export const OUTPUT_EVENT = "components-output";

export function registries(): Promise<RegistrySource[]> {
  return invoke<RegistrySource[]>("components_registries");
}

export function fetchJson(url: string, force = false): Promise<FetchResult> {
  return invoke<FetchResult>("components_fetch", { url, force });
}

export function clearCache(): Promise<number> {
  return invoke<number>("components_cache_clear");
}

export function inspectProject(path: string): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("components_inspect_project", { path });
}

export function readCss(project: string, rel: string): Promise<string> {
  return invoke<string>("components_read_css", { project, rel });
}

export function writeCss(project: string, rel: string, expected: string, content: string): Promise<string> {
  return invoke<string>("components_write_css", { project, rel, expected, content });
}

export function run(runId: string, cwd: string, program: string, args: string[]): Promise<RunOutcome> {
  return invoke<RunOutcome>("components_run", { runId, cwd, program, args });
}

export function cancel(runId: string): Promise<void> {
  return invoke<void>("components_cancel", { runId });
}

export function onOutput(handler: (line: OutputLine) => void): Promise<UnlistenFn> {
  return listen<OutputLine>(OUTPUT_EVENT, (event) => handler(event.payload));
}

export function errorCode(error: unknown): { code: string; detail: string | null } {
  if (typeof error === "string") return { code: error, detail: null };
  if (error && typeof error === "object" && "code" in error) {
    const e = error as { code: unknown; detail?: unknown };
    return { code: String(e.code), detail: typeof e.detail === "string" ? e.detail : null };
  }
  if (error instanceof Error) return { code: "unknown", detail: error.message };
  return { code: "unknown", detail: String(error) };
}
