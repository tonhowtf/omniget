import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CatalogFont, FontDetails, InstallOutcome, InstallProgress, InstalledFamily } from "./types";

export const INSTALL_EVENT = "fonts-install-progress";

export function installed(refresh = false): Promise<InstalledFamily[]> {
  return invoke<InstalledFamily[]>("fonts_installed", { refresh });
}

export function catalog(refresh = false): Promise<CatalogFont[]> {
  return invoke<CatalogFont[]>("fonts_catalog", { refresh });
}

export function details(id: string): Promise<FontDetails> {
  return invoke<FontDetails>("fonts_details", { id });
}

export function userDir(): Promise<string | null> {
  return invoke<string | null>("fonts_user_dir");
}

export function install(jobId: string, id: string, subset: string, outDir: string | null): Promise<InstallOutcome> {
  return invoke<InstallOutcome>("fonts_install", { jobId, id, subset, outDir });
}

export function cancel(jobId: string): Promise<void> {
  return invoke<void>("fonts_cancel", { jobId });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function onInstallProgress(handler: (event: InstallProgress) => void): Promise<UnlistenFn> {
  return listen<InstallProgress>(INSTALL_EVENT, (event) => handler(event.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}
