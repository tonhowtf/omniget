import type { PackExcluded, PackPlan, PortListener, UtilArgs, UtilOp } from "./types";

export function guessOp(input: string): UtilOp | null {
  const text = input.trim();
  if (!text) return null;
  if (/^eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/.test(text)) return "jwt_decode";
  if (text.startsWith("{") || text.startsWith("[")) {
    try {
      JSON.parse(text);
      return "json_format";
    } catch {
      return null;
    }
  }
  if (/^(\d{10}|\d{13})$/.test(text)) return "timestamp";
  if (text.length >= 12 && text.length % 4 === 0 && /^[A-Za-z0-9+/]+={0,2}$/.test(text) && /[0-9+/=]/.test(text)) return "base64_decode";
  return null;
}

export type UtilField = "text" | "other" | "query" | "pattern" | "algorithm" | "value";

export interface UtilForm {
  text: string;
  other: string;
  query: string;
  pattern: string;
  algorithm: NonNullable<UtilArgs["algorithm"]>;
  value: string;
}

export function utilFields(op: UtilOp): UtilField[] {
  switch (op) {
    case "uuid":
      return [];
    case "timestamp":
      return ["value"];
    case "hash":
      return ["algorithm", "text"];
    case "regex_test":
      return ["pattern", "text"];
    case "json_query":
      return ["query", "text"];
    case "text_diff":
      return ["text", "other"];
    default:
      return ["text"];
  }
}

export function buildUtilArgs(op: UtilOp, form: UtilForm): UtilArgs {
  const args: UtilArgs = {};
  for (const field of utilFields(op)) {
    if (field === "value") {
      const value = form.value.trim();
      if (value) args.value = /^\d+$/.test(value) ? Number(value) : value;
    } else if (field === "algorithm") {
      args.algorithm = form.algorithm;
    } else {
      args[field] = form[field];
    }
  }
  return args;
}

export function utilOutput(op: UtilOp, result: Record<string, unknown>): string {
  switch (op) {
    case "hash":
      return String(result.hash ?? "");
    case "uuid":
      return String(result.uuid ?? "");
    case "timestamp":
      return `unix    ${String(result.unix)}\nunixMs  ${String(result.unixMs)}\niso     ${String(result.iso)}`;
    case "jwt_decode":
      return JSON.stringify({ header: result.header, payload: result.payload, expiresAt: result.expiresAt, expired: result.expired }, null, 2);
    case "regex_test":
      return ((result.matches ?? []) as { match: string; start: number; end: number }[]).map((m) => `${m.start}-${m.end}\t${m.match}`).join("\n");
    default:
      return typeof result.text === "string" ? result.text : String(result.hex ?? "");
  }
}

export function parseGlobs(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((glob) => glob.trim())
    .filter(Boolean);
}

export function parseBudget(text: string): number | undefined {
  const digits = text.replace(/\s/g, "");
  if (!/^\d+$/.test(digits)) return undefined;
  const value = Number(digits);
  return value > 0 ? value : undefined;
}

export function budgetMeter(tokens: number, budget: number | undefined): { ratio: number; over: boolean } | null {
  if (!budget) return null;
  return { ratio: Math.min(1, tokens / budget), over: tokens > budget };
}

export interface ExcludedRow {
  where: string;
  reason: PackExcluded["reason"];
  detail: string;
}

export function excludedRows(plan: Pick<PackPlan, "excluded" | "secrets">): ExcludedRow[] {
  return plan.excluded.flatMap((entry) => {
    const hits = entry.reason === "secret" ? plan.secrets.filter((secret) => secret.path === entry.path) : [];
    if (hits.length === 0) return [{ where: entry.path, reason: entry.reason, detail: entry.summary ?? "" }];
    return hits.map((hit) => ({ where: hit.line === null ? hit.path : `${hit.path}:${hit.line}`, reason: entry.reason, detail: hit.rule }));
  });
}

export function removedSummary(removed: Record<string, number>): string {
  return Object.entries(removed)
    .sort(([a, x], [b, y]) => y - x || a.localeCompare(b))
    .map(([tag, count]) => `${count} ${tag}`)
    .join(", ");
}

export function headersToRecord(rows: { name: string; value: string }[]): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (name) headers[name] = row.value;
  }
  return headers;
}

export function statusTone(status: number): "success" | "info" | "warning" | "danger" {
  if (status < 300) return "success";
  if (status < 400) return "info";
  if (status < 500) return "warning";
  return "danger";
}

export function pushHistory<T>(history: T[], entry: T): T[] {
  return [entry, ...history].slice(0, 10);
}

export function filterPorts(listeners: PortListener[], query: string): PortListener[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return listeners;
  return listeners.filter((l) => [String(l.port), String(l.pid), l.process, l.command ?? ""].some((field) => field.toLowerCase().includes(needle)));
}
