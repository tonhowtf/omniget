export type PackMode = "full" | "outline" | "path";
export type PackReason = "gitignored" | "binary" | "lockfile" | "too_large" | "secret" | "excluded";

export interface PackRequest {
  path: string;
  include?: string[];
  exclude?: string[];
  maxTokens?: number;
  output?: string;
  limit?: number;
}

export interface PackFile {
  path: string;
  tokens: number;
  mode: PackMode;
}

export interface PackExcluded {
  path: string;
  reason: PackReason;
  summary?: string;
}

export interface PackSecret {
  path: string;
  line: number | null;
  rule: string;
}

export interface PackReduced {
  path: string;
  to: PackMode;
  tokensBefore: number;
  tokensAfter: number;
}

export interface PackPlan {
  root: string;
  files: PackFile[];
  total: number;
  truncated: boolean;
  excluded: PackExcluded[];
  secrets: PackSecret[];
  reduced: PackReduced[];
  degradeOrder: PackMode[];
  totals: {
    files: number;
    tokens: number;
    excluded: number;
    secrets: number;
    notIncluded: number;
    overBudget: boolean;
  };
}

export interface PackBuilt {
  path: string;
  tokens: number;
  bytes: number;
  sha256: string;
  manifest: PackPlan;
}

export type ScrapeFormat = "markdown" | "text" | "html";

export interface ScrapeFetchRequest {
  url: string;
  format?: ScrapeFormat;
  maxTokens?: number;
  raw?: boolean;
}

export interface ScrapeStructured {
  kind: "json_ld" | "embedded_state";
  id: string | null;
  chars: number;
  truncated: boolean;
  data: unknown;
  preview: string | null;
}

export interface ScrapePage {
  url: string;
  finalUrl: string;
  status: number;
  via: "http";
  title: string | null;
  format: ScrapeFormat;
  content: string;
  tokens: number;
  total: number;
  truncated: boolean;
  note: string | null;
  removed: Record<string, number>;
  structured: ScrapeStructured[];
  hash: string;
  unchanged: boolean;
  proxy?: string;
  attempts: number;
  untrusted: true;
}

export interface ScrapeSelectRequest {
  url?: string;
  html?: string;
  selector: string;
  attr?: string;
  limit?: number;
}

export interface ScrapeSelection {
  matches: string[];
  total: number;
  truncated: boolean;
  relocated?: string;
  untrusted: true;
}

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";

export interface HttpRequest {
  method: HttpMethod;
  url: string;
  headers?: Record<string, string>;
  body?: string;
  timeoutMs?: number;
  maxBodyChars?: number;
  saveBodyTo?: string;
}

export interface HttpResponse {
  status: number;
  headers: Record<string, string>;
  redirects: { status: number; url: string }[];
  finalUrl: string;
  timing: { totalMs: number; firstByteMs: number };
  body: { preview: string; truncated: boolean; total: number; json: boolean; binary: boolean; parseError?: string };
  bodyBytes: number;
  bodyComplete: boolean;
  bodyPath: string | null;
  request: { method: string; url: string; headers: Record<string, string> };
  curl: string;
  untrusted: true;
}

export interface PortListener {
  port: number;
  pid: number;
  process: string;
  command: string | null;
  user: string | null;
  addresses: string[];
}

export interface PortTarget {
  pid: number;
  process: string;
  command: string | null;
  ports: number[];
}

export interface PortKillResult {
  dryRun: boolean;
  targets: PortTarget[];
  killed: number[];
  failed: { pid: number; reason: string }[];
}

export type UtilOp =
  | "json_format"
  | "json_minify"
  | "json_query"
  | "jwt_decode"
  | "base64_encode"
  | "base64_decode"
  | "url_encode"
  | "url_decode"
  | "hash"
  | "uuid"
  | "timestamp"
  | "regex_test"
  | "text_diff";

export interface UtilArgs {
  text?: string;
  other?: string;
  query?: string;
  pattern?: string;
  algorithm?: "md5" | "sha1" | "sha256" | "sha512";
  file?: string;
  value?: string | number;
  limit?: number;
  maxChars?: number;
}

export interface DevtoolsError {
  code: string;
  message: string;
  retryable: boolean;
  nextStep: string;
  candidates: string[];
}
