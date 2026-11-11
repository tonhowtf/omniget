import { invoke } from "@tauri-apps/api/core";
import type {
  DevtoolsError,
  HttpRequest,
  HttpResponse,
  PackBuilt,
  PackPlan,
  PackRequest,
  PortKillResult,
  PortListener,
  ScrapeFetchRequest,
  ScrapePage,
  ScrapeSelectRequest,
  ScrapeSelection,
  UtilArgs,
  UtilOp,
} from "./types";

export function packPlan(request: PackRequest): Promise<PackPlan> {
  return invoke<PackPlan>("devtools_pack_plan", { request });
}

export function packBuild(request: PackRequest): Promise<PackBuilt> {
  return invoke<PackBuilt>("devtools_pack_build", { request });
}

export function scrapeFetch(request: ScrapeFetchRequest): Promise<ScrapePage> {
  return invoke<ScrapePage>("devtools_scrape_fetch", { request });
}

export function scrapeSelect(request: ScrapeSelectRequest): Promise<ScrapeSelection> {
  return invoke<ScrapeSelection>("devtools_scrape_select", { request });
}

export function httpSend(request: HttpRequest): Promise<HttpResponse> {
  return invoke<HttpResponse>("devtools_http_send", { request });
}

export function portsList(): Promise<PortListener[]> {
  return invoke<PortListener[]>("devtools_ports_list");
}

export function portsKill(target: { port?: number; pid?: number }, confirm: boolean): Promise<PortKillResult> {
  return invoke<PortKillResult>("devtools_ports_kill", { ...target, confirm });
}

export function util<T = Record<string, unknown>>(op: UtilOp, args: UtilArgs = {}): Promise<T> {
  return invoke<T>("devtools_util", { op, args });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function parseError(raw: unknown): DevtoolsError {
  const text = typeof raw === "string" ? raw : String(raw);
  const split = text.indexOf(": ");
  const code = split > 0 ? text.slice(0, split) : "";
  if (/^[A-Z0-9_]+$/.test(code)) {
    try {
      const detail = JSON.parse(text.slice(split + 2)) as {
        message?: string;
        retryable?: boolean;
        next_step?: string;
        candidates?: string[];
      };
      return {
        code,
        message: detail.message ?? text,
        retryable: detail.retryable === true,
        nextStep: detail.next_step ?? "",
        candidates: detail.candidates ?? [],
      };
    } catch {
      return { code, message: text.slice(split + 2), retryable: false, nextStep: "", candidates: [] };
    }
  }
  return { code: "OPERATION_FAILED", message: text, retryable: false, nextStep: "", candidates: [] };
}
