// Saúde da config por ferramenta e monitor de releases (/llm/tools).
// Invokes de `clitools_config_files` e `clitools_updates` + helpers puros
// (agrupamento, prompt do "otimizar com um agente", relógio diário).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { StatsKind } from "$lib/central/guard";

export const UPDATES_EVENT = "clitools://updates";

export type ConfigEntry = {
  scope: "global" | "project" | "local" | string;
  key: string;
  stat_kind: StatsKind | null;
  path: string;
  template: string;
  exists: boolean;
  is_dir: boolean;
  size: number;
  files: number;
  truncated: boolean;
  /** Pasta que contém outras entradas (ex.: ~/.cursor): não somada nem varrida. */
  container: boolean;
  alt: boolean;
  surface: string | null;
  format: string | null;
};

export type ConfigFiles = {
  tool: string;
  name: string;
  target: string | null;
  project_dir: string | null;
  entries: ConfigEntry[];
  stats_input: Record<string, Partial<Record<StatsKind, string[]>>>;
  scan_paths: Record<string, string[]>;
  skipped: string[];
  total_bytes: number;
};

export type Changelog = {
  tag: string;
  name: string | null;
  url: string;
  published_at: string | null;
  excerpt: string;
  truncated: boolean;
};

export type ToolUpdate = {
  id: string;
  name: string;
  installed: string | null;
  latest: string | null;
  state: "unknown" | "current" | "behind_latest";
  source: string;
  update_available: boolean;
  auto_updates: boolean;
  changelog: Changelog | null;
  error: string | null;
};

export type UpdatesReport = {
  checked_at: number;
  next_auto_at: number;
  from_cache: boolean;
  available: number;
  tools: ToolUpdate[];
};

export const configFiles = (tool: string, projectDir: string | null) =>
  invoke<ConfigFiles>("clitools_config_files", { tool, projectDir });

export const checkUpdates = (opts: { auto?: boolean; force?: boolean } = {}) =>
  invoke<UpdatesReport>("clitools_updates", { auto: opts.auto ?? false, force: opts.force ?? false });

export const cachedUpdates = () => invoke<UpdatesReport | null>("clitools_updates_cached");

export function onUpdates(fn: (r: UpdatesReport) => void): Promise<UnlistenFn> {
  return listen<UpdatesReport>(UPDATES_EVENT, (e) => fn(e.payload));
}

/** Entradas agrupadas por escopo, na ordem global → projeto → local. */
export function byScope(entries: ConfigEntry[]): [string, ConfigEntry[]][] {
  const order = ["global", "project", "local"];
  const map = new Map<string, ConfigEntry[]>();
  for (const e of entries) {
    const list = map.get(e.scope) ?? [];
    list.push(e);
    map.set(e.scope, list);
  }
  return [...map.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0]));
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Junta a entrada de stats de vários escopos (o guard aceita uma só). */
export function mergeStatsInput(
  input: ConfigFiles["stats_input"],
  scopes: string[],
): Partial<Record<StatsKind, string[]>> {
  const out: Partial<Record<StatsKind, string[]>> = {};
  for (const s of scopes) {
    for (const [k, paths] of Object.entries(input[s] ?? {})) {
      const key = k as StatsKind;
      out[key] = [...new Set([...(out[key] ?? []), ...(paths ?? [])])];
    }
  }
  return out;
}

/**
 * Prompt do Job "otimizar com um agente": revisar os arquivos listados (só
 * leitura, propor mudanças), com os números das estatísticas como contexto.
 */
export function optimizePrompt(args: {
  toolName: string;
  files: ConfigEntry[];
  facts: string[];
  locale: string;
}): string {
  const list = args.files
    .filter((f) => f.exists)
    .map((f) => `- [${f.scope}/${f.key}] ${f.path}${f.is_dir ? ` (pasta, ${f.files} arquivos)` : ""} · ${formatBytes(f.size)}`)
    .join("\n");
  const facts = args.facts.length ? `\nNúmeros medidos pelo OmniGet:\n${args.facts.map((f) => `- ${f}`).join("\n")}\n` : "";
  const pt = args.locale.startsWith("pt");
  if (pt) {
    return `Revise a configuração do ${args.toolName} nestes arquivos:\n${list}\n${facts}
Objetivo: deixar a configuração mais enxuta e correta, sem perder comportamento.
1. Aponte regras/agentes/comandos pesados em tokens ou duplicados e proponha versões mais curtas.
2. Hooks: comandos cujo programa não existe, variáveis de ambiente inexistentes, eventos errados.
3. MCPs: servidores que não sobem, duplicados, desnecessários ou com segredo escrito no arquivo.
4. Formatos ou caminhos deprecados.
Não abra arquivos de credencial (auth.json, credentials*, oauth*, *token*). Não altere nada ainda: entregue um relatório com o diff proposto por arquivo.`;
  }
  return `Review the ${args.toolName} configuration in these files:\n${list}\n${facts}
Goal: make the configuration leaner and correct without losing behaviour.
1. Point out token-heavy or duplicated rules/agents/commands and propose shorter versions.
2. Hooks: commands whose program does not exist, env vars that do not exist, wrong events.
3. MCP servers that do not start, duplicates, unneeded ones or secrets written in the file.
4. Deprecated formats or paths.
Do not open credential files (auth.json, credentials*, oauth*, *token*). Do not change anything yet: deliver a report with the proposed diff per file.`;
}

const LAST_AUTO_KEY = "omniget.central.tools.updates.last-auto";
const DAY_MS = 24 * 3600 * 1000;

/**
 * Relógio da checagem diária: roda só enquanto a janela está visível. Ao
 * ficar visível, se venceu (≥ 24 h desde a última), pede `clitools_updates`
 * com `auto` (o backend também respeita as 24 h) e agenda a próxima. Devolve
 * a função que desliga.
 */
export function startDailyUpdates(onReport?: (r: UpdatesReport) => void): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  const last = () => {
    try {
      return Number(localStorage.getItem(LAST_AUTO_KEY) ?? 0) || 0;
    } catch {
      return 0;
    }
  };
  const mark = (at: number) => {
    try {
      localStorage.setItem(LAST_AUTO_KEY, String(at));
    } catch {
      /* sem storage: o backend ainda segura as 24 h */
    }
  };
  const clear = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };
  const schedule = () => {
    clear();
    if (stopped || document.visibilityState !== "visible") return;
    const wait = Math.max(0, last() + DAY_MS - Date.now());
    // setTimeout aceita até ~24,8 dias; 24 h cabe.
    timer = setTimeout(run, Math.min(wait, DAY_MS) + 1000);
  };
  const run = async () => {
    timer = null;
    if (stopped || document.visibilityState !== "visible") return;
    if (Date.now() - last() >= DAY_MS) {
      try {
        const r = await checkUpdates({ auto: true });
        mark(r.from_cache ? r.checked_at * 1000 : Date.now());
        onReport?.(r);
      } catch {
        mark(Date.now() - DAY_MS + 3600 * 1000); // sem rede: tenta em 1 h
      }
    }
    schedule();
  };
  const onVisibility = () => {
    if (document.visibilityState === "visible") void run();
    else clear();
  };
  document.addEventListener("visibilitychange", onVisibility);
  void run();
  return () => {
    stopped = true;
    clear();
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
