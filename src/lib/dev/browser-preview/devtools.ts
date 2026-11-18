import type {
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
} from "$lib/devtools/types";

type Args = Record<string, unknown>;

function fail(code: string, message: string, nextStep: string, candidates: string[] = []): never {
  const detail: Record<string, unknown> = { message, retryable: false, next_step: nextStep };
  if (candidates.length > 0) detail.candidates = candidates;
  throw `${code}: ${JSON.stringify(detail)}`;
}

function plan(request: PackRequest): PackPlan {
  const budget = request.maxTokens ?? Infinity;
  const reduced = budget < 9000;
  return {
    root: request.path || "/preview/project",
    files: [
      { path: "src/lib/catalog.ts", tokens: reduced ? 310 : 6120, mode: reduced ? "outline" : "full" },
      { path: "src/lib/search.ts", tokens: 1480, mode: "full" },
      { path: "README.md", tokens: 920, mode: "full" },
      { path: "src/lib/pins.ts", tokens: 410, mode: "full" },
    ],
    total: 4,
    truncated: false,
    excluded: [
      { path: ".env", reason: "secret" },
      { path: "assets/logo.png", reason: "binary" },
      { path: "node_modules/", reason: "gitignored" },
      { path: "pnpm-lock.yaml", reason: "lockfile", summary: "pnpm-lock.yaml: 8421 lines (lockfile omitted)" },
      { path: "src/config.ts", reason: "secret" },
    ],
    secrets: [
      { path: ".env", line: null, rule: "env_file" },
      { path: "src/config.ts", line: 12, rule: "api_token" },
    ],
    reduced: reduced ? [{ path: "src/lib/catalog.ts", to: "outline", tokensBefore: 6120, tokensAfter: 310 }] : [],
    degradeOrder: ["full", "outline", "path"],
    totals: {
      files: 4,
      tokens: reduced ? 3120 : 8930,
      excluded: 5,
      secrets: 2,
      notIncluded: 0,
      overBudget: false,
    },
  };
}

const PAGE = "# Example Domain\n\nThis domain is for use in documentation examples without needing permission.\n\n[Learn more](https://iana.org/domains/example)";

const LISTENERS: PortListener[] = [
  { port: 1420, pid: 48211, process: "node", command: "node node_modules/vite/bin/vite.js dev", user: "preview", addresses: ["127.0.0.1"] },
  { port: 5432, pid: 912, process: "postgres", command: "postgres -D /usr/local/var/postgres", user: "preview", addresses: ["127.0.0.1", "[::1]"] },
  { port: 8080, pid: 50977, process: "python3", command: "python3 -m http.server 8080", user: "preview", addresses: ["*"] },
];

function util(op: string, args: UtilArgs): unknown {
  const text = args.text ?? "";
  const capped = (value: string) => ({ text: value, truncated: false, total: value.length });
  try {
    switch (op) {
      case "json_format":
        return capped(JSON.stringify(JSON.parse(text), null, 2));
      case "json_minify":
        return capped(JSON.stringify(JSON.parse(text)));
      case "base64_encode":
        return capped(btoa(unescape(encodeURIComponent(text))));
      case "base64_decode":
        return capped(decodeURIComponent(escape(atob(text))));
      case "url_encode":
        return capped(encodeURIComponent(text));
      case "url_decode":
        return capped(decodeURIComponent(text));
      case "uuid":
        return { uuid: crypto.randomUUID() };
      case "timestamp": {
        const date = args.value === undefined ? new Date() : new Date(typeof args.value === "number" ? args.value * 1000 : args.value);
        return { unix: Math.floor(date.getTime() / 1000), unixMs: date.getTime(), iso: date.toISOString() };
      }
      case "hash":
        return { hash: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", algorithm: args.algorithm ?? "sha256", bytes: text.length, source: "text" };
      case "jwt_decode":
        return {
          header: { alg: "HS256", typ: "JWT" },
          payload: { sub: "preview", exp: 1900000000 },
          expiresAt: "2030-03-17T17:46:40Z",
          expired: false,
          verified: false,
          note: "decoded only: the signature was not verified, so the claims are untrusted",
        };
      case "json_query":
        return { ...capped('"preview"'), value: "preview", type: "string" };
      case "regex_test":
        return { matches: [{ match: text.slice(0, 4), start: 0, end: 4, groups: [], named: {} }], total: 1, truncated: false };
      case "text_diff":
        return { ...capped("--- text\n+++ other\n@@ -1,1 +1,1 @@\n-before\n+after\n"), added: 1, removed: 1, identical: false };
    }
  } catch (error) {
    return fail("INVALID_INPUT", String(error), "fix the argument named in the message and call again");
  }
  return fail("INVALID_INPUT", `unknown op "${op}"`, "fix the argument named in the message and call again");
}

export const devtoolsFixtures: Record<string, (args: Args) => unknown> = {
  devtools_pack_plan: ({ request }): PackPlan => plan(request as PackRequest),
  devtools_pack_build: ({ request }): PackBuilt => {
    const manifest = plan(request as PackRequest);
    return {
      path: (request as PackRequest).output ?? "/preview/tmp/omniget-devtools/pack-preview.txt",
      tokens: manifest.totals.tokens + 60,
      bytes: (manifest.totals.tokens + 60) * 4,
      sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
      manifest,
    };
  },
  devtools_scrape_fetch: ({ request }): ScrapePage => {
    const r = request as ScrapeFetchRequest;
    if (r.url.includes("blocked")) {
      fail("BLOCKED", "the site blocked the request: cloudflare_challenge (HTTP 403)", "plain HTTP was the only level tried; open the page with gd_browser_open and read it with gd_snapshot, which use a real browser session");
    }
    return {
      url: r.url,
      finalUrl: r.url,
      status: 200,
      via: "http",
      title: "Example Domain",
      format: r.format ?? "markdown",
      content: PAGE,
      tokens: 34,
      total: 34,
      truncated: false,
      note: null,
      removed: r.raw ? {} : { nav: 1, script: 3, style: 1, footer: 1 },
      structured: [{ kind: "json_ld", id: null, chars: 48, truncated: false, data: { "@type": "WebPage", name: "Example Domain" }, preview: null }],
      hash: "9f86d081884c7d65",
      unchanged: false,
      attempts: 1,
      untrusted: true,
    };
  },
  devtools_scrape_select: ({ request }): ScrapeSelection => {
    const r = request as ScrapeSelectRequest;
    if (r.selector === ".price") {
      fail("NO_MATCH", "no match for selector .price", "retry with one of the candidates", [".product-price", ".price-box"]);
    }
    return { matches: ["Example Domain", "Learn more"], total: 2, truncated: false, untrusted: true };
  },
  devtools_http_send: ({ request }): HttpResponse => {
    const r = request as HttpRequest;
    const headers = Object.fromEntries(
      Object.entries(r.headers ?? {}).map(([name, value]) => [name, /authorization|cookie|api-?key|token|secret|password/i.test(name) ? "***" : value]),
    );
    return {
      status: 200,
      headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
      redirects: r.url.startsWith("http://") ? [{ status: 301, url: r.url.replace("http://", "https://") }] : [],
      finalUrl: r.url.replace("http://", "https://"),
      timing: { totalMs: 182, firstByteMs: 141 },
      body: { preview: '{\n  "ok": true,\n  "items": [\n    1,\n    2,\n    3\n  ]\n}', truncated: false, total: 49, json: true, binary: false },
      bodyBytes: 27,
      bodyComplete: true,
      bodyPath: r.saveBodyTo ?? null,
      request: { method: r.method, url: r.url, headers },
      curl: `curl -X ${r.method} '${r.url}'`,
      untrusted: true,
    };
  },
  devtools_ports_list: (): PortListener[] => LISTENERS,
  devtools_ports_kill: ({ port, pid, confirm }): PortKillResult => {
    const hit = LISTENERS.filter((l) => l.port === port || l.pid === pid);
    if (hit.length === 0) {
      fail("NOT_FOUND", port ? `nothing is listening on TCP port ${String(port)}` : "pass port or pid", "call dev_ports with op=list to see what is listening; nothing was killed");
    }
    const targets = hit.map((l) => ({ pid: l.pid, process: l.process, command: l.command, ports: [l.port] }));
    return { dryRun: confirm !== true, targets, killed: confirm === true ? targets.map((t) => t.pid) : [], failed: [] };
  },
  devtools_util: ({ op, args }) => util(String(op), (args ?? {}) as UtilArgs),
};
