import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

export type LogLevel = "info" | "warn" | "error";
export type LogCategory = "download" | "network" | "auth" | "system" | "convert";

export type DebugLogEntry = {
  id: number;
  timestamp: number;
  level: LogLevel;
  category: LogCategory;
  message: string;
  details?: string;
};

const MAX_ENTRIES = 500;

let nextId = 0;
let enabled = $state(false);
let logs = $state<DebugLogEntry[]>([]);
let panelOpen = $state(false);

export function isDebugEnabled(): boolean {
  return enabled;
}

export function setDebugEnabled(value: boolean) {
  enabled = value;
  if (!value) {
    panelOpen = false;
  }
}

export function isDebugPanelOpen(): boolean {
  return panelOpen;
}

export function setDebugPanelOpen(value: boolean) {
  panelOpen = value;
}

export function toggleDebugPanel() {
  if (!enabled) {
    enabled = true;
  }
  panelOpen = !panelOpen;
}

export function getDebugLogs(): DebugLogEntry[] {
  return logs;
}

export function addLog(
  level: LogLevel,
  category: LogCategory,
  message: string,
  details?: string,
) {
  if (!enabled) return;

  const entry: DebugLogEntry = {
    id: nextId++,
    timestamp: Date.now(),
    level,
    category,
    message,
    details,
  };

  logs = [...logs, entry];

  if (logs.length > MAX_ENTRIES) {
    logs = logs.slice(logs.length - MAX_ENTRIES);
  }
}

export function clearLogs() {
  logs = [];
}

type DependencyStatus = {
  name: string;
  installed: boolean;
  version: string | null;
};

export async function exportDiagnostics(): Promise<string> {
  const lines: string[] = [];
  lines.push("--- OmniGet Diagnostic Report ---");

  try {
    const appVersion = await getVersion();
    lines.push(`Version: ${appVersion}`);
  } catch {
    lines.push("Version: unknown");
  }

  lines.push(`OS: ${navigator.userAgent}`);
  lines.push(`Platform: ${navigator.platform}`);
  lines.push(`Timestamp: ${new Date().toISOString()}`);

  try {
    const deps = await invoke<DependencyStatus[]>("check_dependencies");
    for (const dep of deps) {
      lines.push(`${dep.name}: ${dep.installed ? dep.version ?? "installed" : "not found"}`);
    }
  } catch {
    lines.push("Dependencies: check failed");
  }

  lines.push("");
  lines.push(`--- Activity Log (${logs.length} entries) ---`);

  for (const entry of logs) {
    const time = new Date(entry.timestamp).toLocaleTimeString("en-US", { hour12: false });
    const level = entry.level.toUpperCase().padEnd(5);
    lines.push(`[${time}] ${level} [${entry.category}] ${entry.message}`);
    if (entry.details) {
      lines.push(`         > ${entry.details}`);
    }
  }

  // B33: a caixa-preta do backend. Guarda as ultimas linhas de log de download
  // ja redigidas (sem token, cookie ou caminho com nome de usuario), que e
  // justamente o que falta quando alguem diz "parou de funcionar" e o log da
  // interface so tem o resultado final.
  try {
    const flight = await invoke<string[]>("flight_recorder_dump");
    if (flight.length > 0) {
      lines.push("");
      lines.push(`--- Download Trace (${flight.length} lines, redacted) ---`);
      lines.push(...flight);
    }
  } catch {
    lines.push("Download trace: unavailable");
  }

  lines.push("---");
  return lines.join("\n");
}
