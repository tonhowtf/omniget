type Args = Record<string, unknown>;
type Entry = Record<string, unknown> & { id: string };

const base = {
  min_omniget: "1.0.0",
  sha256: "0".repeat(64),
  category: "courses",
  installed_version: null,
  dev: false,
  in_catalog: true,
  compatible: true,
  has_options: false,
};

let entries: Entry[] = [
  {
    ...base,
    id: "hotmart",
    name: "Hotmart",
    description: "Hotmart Club courses you bought: videos, audio, attachments and lesson texts, by module. Locked lessons are marked. DRM protected items are listed and skipped.",
    version: "0.1.0",
    author: "tonhowtf",
    match: ["https://*.club.hotmart.com/**"],
    domains: ["hotmart.com", "pandavideo.com.br"],
    login: { url: "https://consumer.hotmart.com/" },
    package: "https://getomniget.com/connectors/packages/hotmart-0.1.0.omniconnector",
    permissions: ["page_eval"],
    state: "not_installed",
  },
  {
    ...base,
    id: "udemy",
    name: "Udemy",
    description: "Courses you are enrolled in: videos, captions, articles, files, attachments, quizzes and practice tests. DRM lectures are listed and skipped.",
    description_i18n: {
      ru: "Курсы, на которые вы записаны: видео, субтитры, статьи, файлы, вложения, тесты и практические задания. Лекции с DRM показываются и пропускаются.",
    },
    version: "0.3.0",
    author: "tonhowtf",
    match: ["https://*.udemy.com/course/**"],
    domains: ["udemy.com", "udemycdn.com"],
    login: { url: "https://www.udemy.com/join/passwordless-auth/" },
    package: "https://getomniget.com/connectors/packages/udemy-0.3.0.omniconnector",
    permissions: ["cookies", "page_eval"],
    state: "update_available",
    installed_version: "0.2.0",
    has_options: true,
  },
  {
    ...base,
    id: "bunkr",
    name: "Bunkr",
    description: "Whole Bunkr albums (every page) or single files.",
    version: "0.1.0",
    author: "tonhowtf",
    match: ["https://bunkr.si/**"],
    domains: ["bunkr.si", "bunkr.cr", "bunkr.su", "bunkr.ph", "scdn.st"],
    package: "https://getomniget.com/connectors/packages/bunkr-0.1.0.omniconnector",
    category: "files",
    permissions: [],
    state: "installed",
    installed_version: "0.1.0",
  },
  {
    ...base,
    id: "future",
    name: "Needs a newer app",
    description: "Shows the state of a helper that needs a newer OmniGet.",
    version: "1.0.0",
    author: "tonhowtf",
    match: ["https://future.test/**"],
    domains: ["future.test"],
    min_omniget: "9.0.0",
    package: "https://getomniget.com/connectors/packages/future-1.0.0.omniconnector",
    permissions: [],
    state: "not_installed",
    compatible: false,
  },
];

function scenario() {
  return new URLSearchParams(location.search).get("market") ?? "";
}

function wait(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function list() {
  const mode = scenario();
  await wait(mode === "loading" ? 60_000 : 300);
  const shared = { invalid: [], rejected: [], fetched_at: 1793800000 };
  if (mode === "empty") return { ...shared, entries: [], source: "network", error: null };
  if (mode === "error") return { ...shared, entries: [], source: "none", fetched_at: null, error: "signature: the catalog signature is not valid: preview" };
  if (mode === "offline") return { ...shared, entries, source: "cache", error: "network: https://getomniget.com/connectors/catalog.json: preview" };
  if (mode === "invalid") return { ...shared, entries, source: "network", error: null, invalid: [{ id: "broken", dir: "/preview/connectors/broken", dev: false, error: "this helper needs OmniGet 9.0.0 or newer (this is 1.0.0)" }] };
  return { ...shared, entries, source: "network", error: null };
}

async function install(args: Args) {
  await wait(1200);
  if (scenario() === "install-fails") throw "hash: the package does not match the catalog (expected abc, got def)";
  entries = entries.map((e) => (e.id === args.id ? { ...e, state: "installed", installed_version: e.version } : e));
  return { id: args.id };
}

function lookup(args: Args) {
  const url = String(args.url ?? "");
  if (/kiwify/.test(url)) return { opted_out: true, installed: null, catalog: null };
  const id = /hotmart\.com/.test(url) ? "hotmart" : /udemy\.com/.test(url) ? "udemy" : /bunkr/.test(url) ? "bunkr" : null;
  const entry = entries.find((e) => e.id === id) ?? null;
  const installed = entry && entry.state !== "not_installed"
    ? { id: entry.id, name: entry.name, login_url: (entry.login as { url: string } | undefined)?.url ?? null, dev: false }
    : null;
  return { opted_out: false, installed, catalog: entry };
}

export const marketFixtures: Record<string, (args: Args) => unknown> = {
  gd_market_list: list,
  gd_find: (args) => {
    const url = String(args.url ?? "");
    if (scenario() === "auth-error") {
      throw `resolve: Error: Not signed in. Sign in in the browser, then try again. (${url})`;
    }
    const connector = /udemy/i.test(url) ? "udemy" : "hotmart";
    return {
      connector,
      ms: 420,
      items: 7,
      drm: 1,
      logs: [`preview: resolved ${url}`],
      tree: {
        id: "library",
        title: connector === "udemy" ? "Udemy" : "Hotmart Club",
        kind: "folder",
        children: [
          {
            id: "course-1",
            title: "Rust & Tauri Desktop Engineering",
            kind: "folder",
            children: [
              {
                id: "c1-m1",
                title: "01. Architecture & IPC",
                kind: "folder",
                children: [
                  { id: "c1-l1", title: "01. Project structure & workspace crates", kind: "video", source: { format: "hls", url: "https://example.com/1.m3u8" } },
                  { id: "c1-l2", title: "02. Tauri commands & event streams", kind: "video", source: { format: "hls", url: "https://example.com/2.m3u8" } },
                  { id: "c1-l3", title: "Architecture cheatsheet.pdf", kind: "document", source: { format: "direct", url: "https://example.com/cheat.pdf" } },
                ],
              },
              {
                id: "c1-m2",
                title: "02. Performance & Packaging",
                kind: "folder",
                children: [
                  { id: "c1-l4", title: "03. Memory profiling & async streams", kind: "video", source: { format: "hls", url: "https://example.com/3.m3u8" } },
                  { id: "c1-l5", title: "04. Protected bonus session", kind: "video", drm: true },
                ],
              },
            ],
          },
          {
            id: "course-2",
            title: "Design Systems & Modern Svelte 5",
            kind: "folder",
            children: [
              {
                id: "c2-m1",
                title: "01. Runes & Fine-grained Reactivity",
                kind: "folder",
                children: [
                  { id: "c2-l1", title: "01. $state, $derived and $effect", kind: "video", source: { format: "hls", url: "https://example.com/4.m3u8" } },
                  { id: "c2-l2", title: "02. Building accessible Spotlight Cards", kind: "video", source: { format: "hls", url: "https://example.com/5.m3u8" } },
                ],
              },
            ],
          },
          {
            id: "course-3",
            title: "Local-First AI Agents with MCP",
            kind: "folder",
            children: [
              {
                id: "c3-m1",
                title: "01. Designing Dense Agent Tools",
                kind: "folder",
                children: [
                  { id: "c3-l1", title: "01. Tool schemas & structured outputs", kind: "video", source: { format: "hls", url: "https://example.com/6.m3u8" } },
                ],
              },
            ],
          },
        ],
      },
    };
  },
  gd_market_install: install,
  gd_market_update: install,
  gd_connector_for_url: lookup,
  gd_connector_remove: (args) => {
    entries = entries.map((e) => (e.id === args.id ? { ...e, state: "not_installed", installed_version: null } : e));
    return null;
  },
  gd_connector_options: () => ({
    defs: [{ id: "captions", label: "Caption languages", type: "string", default: "auto", help: "auto, all, none, or a list like en,pt" }],
    values: { captions: "auto" },
  }),
  gd_connector_options_set: () => ({}),
  gd_browser_show: () => ({ tabs: [], mode: "window" }),
};
