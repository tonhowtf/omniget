import type { Cell, Column, EndpointParam, Field, LinkCell, Result, Table, Workflow, WorkflowId } from "./types";

export const WORKFLOWS: Workflow[] = [
  {
    id: "creators",
    tool: "research_tiktok_creators",
    fields: [
      { key: "brief", kind: "area" },
      { key: "queries", kind: "lines", placeholder: "home gym setup\nsmall apartment workout" },
      { key: "sample", kind: "number", min: 3, max: 30, initial: "10" },
      { key: "max_creators", kind: "number", min: 1, max: 15, initial: "5" },
      { key: "transcripts_per_creator", kind: "number", min: 0, max: 5, initial: "2" },
      { key: "region", kind: "text", placeholder: "US" },
      { key: "date_posted", kind: "select", options: ["", "yesterday", "this-week", "this-month", "last-3-months", "last-6-months", "all-time"] },
      { key: "ai_transcripts", kind: "toggle", initial: "false" },
      { key: "follow_bio_link", kind: "toggle", initial: "true" },
    ],
  },
  {
    id: "brief",
    tool: "research_tiktok_brief",
    fields: [
      { key: "urls", kind: "lines", placeholder: "https://www.tiktok.com/@creator/video/123" },
      { key: "query", kind: "text" },
      { key: "hashtag", kind: "text" },
      { key: "handle", kind: "text" },
      { key: "count", kind: "number", min: 1, max: 20, initial: "5" },
      { key: "transcript_chars", kind: "number", min: 200, max: 6000, initial: "1500" },
      { key: "ai_transcripts", kind: "toggle", initial: "false" },
    ],
  },
  {
    id: "ads",
    tool: "research_meta_ads",
    fields: [
      { key: "op", kind: "select", options: ["find", "baseline", "compare", "list"], initial: "find" },
      { key: "query", kind: "text", ops: ["find"] },
      { key: "page_id", kind: "text", ops: ["baseline"] },
      { key: "company", kind: "text", ops: ["baseline"] },
      { key: "country", kind: "text", ops: ["baseline"], placeholder: "ALL" },
      { key: "status", kind: "select", options: ["ACTIVE", "ALL", "INACTIVE"], initial: "ACTIVE", ops: ["baseline"] },
      { key: "name", kind: "text", ops: ["baseline", "compare"] },
      { key: "max_ads", kind: "number", min: 1, max: 300, initial: "100", ops: ["baseline", "compare"] },
      { key: "update", kind: "toggle", initial: "false", ops: ["compare"] },
    ],
  },
  {
    id: "reels",
    tool: "research_reels",
    fields: [
      { key: "profiles", kind: "lines", placeholder: "nike\nadidas" },
      { key: "sample", kind: "number", min: 3, max: 50, initial: "12" },
      { key: "detail", kind: "select", options: ["outliers", "none"], initial: "outliers" },
    ],
  },
];

export function initialValues(fields: Field[]): Record<string, string> {
  return Object.fromEntries(fields.map((field) => [field.key, field.initial ?? ""]));
}

export function visibleFields(fields: Field[], op: string): Field[] {
  return fields.filter((field) => !field.ops || field.ops.includes(op));
}

export function buildArgs(fields: Field[], values: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const field of visibleFields(fields, values.op ?? "")) {
    const raw = (values[field.key] ?? "").trim();
    if (!raw) continue;
    if (field.kind === "lines") {
      const items = raw.split(/[\n,]/).map((item) => item.trim()).filter(Boolean);
      if (items.length) out[field.key] = items;
    } else if (field.kind === "number") {
      const n = Number(raw);
      if (Number.isFinite(n)) out[field.key] = n;
    } else if (field.kind === "toggle") {
      out[field.key] = raw === "true";
    } else {
      out[field.key] = raw;
    }
  }
  return out;
}

export function coerceParams(params: EndpointParam[], values: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const param of params) {
    const raw = (values[param.name] ?? "").trim();
    if (!raw) continue;
    if (param.type === "boolean") out[param.name] = raw === "true";
    else if ((param.type === "number" || param.type === "integer") && Number.isFinite(Number(raw))) out[param.name] = Number(raw);
    else out[param.name] = raw;
  }
  return out;
}

export function missingRequired(params: EndpointParam[], values: Record<string, string>): string[] {
  return params.filter((param) => param.required && !(values[param.name] ?? "").trim()).map((param) => param.name);
}

type Row = Record<string, unknown>;

function rows(value: unknown): Row[] {
  return Array.isArray(value) ? (value.filter((item) => item && typeof item === "object") as Row[]) : [];
}

function obj(value: unknown): Row {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Row) : {};
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function str(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}

function list(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function day(value: unknown): string | null {
  const text = str(value);
  return text ? text.slice(0, 10) : null;
}

function link(href: unknown, label: unknown): LinkCell | string | null {
  const url = str(href);
  const text = str(label) ?? url;
  if (!text) return null;
  return url && /^https?:\/\//.test(url) ? { href: url, label: text } : text;
}

function cols(spec: string): Column[] {
  return spec.split(" ").map((part) => {
    const [key, kind] = part.split(":");
    return { key, kind: (kind ?? "text") as Column["kind"] };
  });
}

function percent(value: unknown): string | null {
  const n = num(value);
  return n === null ? null : `${(n * 100).toFixed(1)}%`;
}

function creatorTables(result: Result): Table[] {
  const creators = rows(result.creators);
  const main = creators.map((c) => {
    const recent = obj(c.recent);
    const views = obj(recent.views);
    const contact = obj(c.contact);
    const reach = [...list(contact.emails), ...(str(contact.link_in_bio) ? [str(contact.link_in_bio) as string] : list(contact.links).slice(0, 1))];
    const from = day(recent.date_from);
    const to = day(recent.date_to);
    return {
      creator: link(c.profile_url, c.handle ? `@${String(c.handle)}` : null),
      followers: num(c.followers),
      views_median: num(views.median),
      views_mean: num(views.mean),
      sample: num(recent.sample),
      confidence: str(recent.confidence),
      engagement: percent(recent.engagement_rate_median),
      period: from && to ? `${from} → ${to}` : null,
      contact: reach.length ? reach.join(", ") : null,
      missing: list(c.missing),
    };
  });
  const matched = creators.flatMap((c) =>
    rows(c.matched_videos).map((v) => {
      const transcript = obj(v.transcript);
      const excerpts = list(transcript.excerpts);
      return {
        creator: c.handle ? `@${String(c.handle)}` : null,
        video: link(v.url, str(v.caption)?.slice(0, 60) ?? v.url),
        views: num(v.views),
        lift: num(v.lift_vs_own_median),
        evidence: excerpts[0] ?? null,
        missing: transcript.available === true ? [] : [`no_transcript: ${str(transcript.reason) ?? "unknown"}`],
      };
    }),
  );
  return [
    { id: "creators", columns: cols("creator:link followers:number views_median:number views_mean:number sample:number confidence engagement period contact missing:flags"), rows: main },
    { id: "matched", columns: cols("creator video:link views:number lift:number evidence missing:flags"), rows: matched },
  ];
}

function briefTables(result: Result): Table[] {
  const videos = rows(result.videos).map((v) => {
    const flags = obj(v.flags);
    const stats = obj(v.stats);
    const author = obj(v.author);
    const missing: string[] = [];
    if (flags.no_transcript === true) missing.push("no_transcript");
    if (str(v.error)) missing.push(String(v.error));
    if (v.transcript_truncated === true) missing.push("transcript_truncated");
    return {
      video: link(v.url, v.url),
      author: author.handle ? `@${String(author.handle)}` : null,
      views: num(stats.views),
      likes: num(stats.likes),
      hook: str(v.hook),
      claims: rows(flags.needs_checking).length,
      missing,
    };
  });
  return [{ id: "videos", columns: cols("video:link author views:number likes:number hook claims:number missing:flags"), rows: videos }];
}

function adRow(ad: Row): Record<string, Cell> {
  return {
    ad: link(ad.url, ad.archive_id),
    format: str(ad.format),
    cta: str(ad.cta_text),
    running_days: num(ad.running_days),
    started: day(ad.started),
    text: str(ad.text)?.slice(0, 160) ?? str(ad.title),
  };
}

function shown(value: unknown): string {
  return value === null || value === undefined || value === "" ? "∅" : String(value);
}

function adsTables(result: Result): Table[] {
  const out: Table[] = [];
  const adColumns = cols("ad:link format cta running_days:number started text");
  if (Array.isArray(result.pages)) {
    out.push({
      id: "pages",
      columns: cols("name page_id category likes:number"),
      rows: rows(result.pages).map((p) => ({ name: str(p.name), page_id: str(p.page_id), category: str(p.category), likes: num(p.likes) })),
    });
  }
  if (Array.isArray(result.baselines)) {
    out.push({
      id: "baselines",
      columns: cols("name country status ads:number updated"),
      rows: rows(result.baselines).map((b) => ({
        name: str(b.name),
        country: str(b.country),
        status: str(b.status),
        ads: num(b.ads),
        updated: num(b.updated_ms) === null ? null : new Date(b.updated_ms as number).toISOString().slice(0, 10),
      })),
    });
  }
  for (const id of ["ads", "new_ads", "no_longer_listed"] as const) {
    if (Array.isArray(result[id])) out.push({ id, columns: adColumns, rows: rows(result[id]).map(adRow) });
  }
  if (Array.isArray(result.changed)) {
    out.push({
      id: "changed",
      columns: cols("ad:link running_days:number changes"),
      rows: rows(result.changed).map((c) => ({
        ad: link(c.url, c.archive_id),
        running_days: num(c.running_days),
        changes: rows(c.changes)
          .map((change) => `${String(change.field)}: ${shown(change.before)} → ${shown(change.after)}`)
          .join("; "),
      })),
    });
  }
  if (rows(result.not_seen_in_partial_fetch).length) {
    out.push({ id: "not_seen", columns: adColumns, rows: rows(result.not_seen_in_partial_fetch).map(adRow) });
  }
  return out;
}

function reelsTables(result: Result): Table[] {
  const profiles = rows(result.profiles);
  const main = profiles.map((p) => {
    const plays = obj(p.plays);
    const cadence = obj(p.cadence);
    return {
      profile: link(p.profile_url, p.handle ? `@${String(p.handle)}` : null),
      sample: num(p.sample),
      plays_median: num(plays.median),
      plays_mean: num(plays.mean),
      likes_median: num(p.likes_median),
      comments_median: num(p.comments_median),
      per_week: num(cadence.reels_per_week),
      confidence: str(p.confidence),
      missing: [...list(p.flags), ...(str(p.error) ? [String(p.error)] : [])],
    };
  });
  const reels = profiles.flatMap((p) =>
    rows(p.reels).map((r) => ({
      profile: p.handle ? `@${String(p.handle)}` : null,
      reel: link(r.url, str(r.caption)?.slice(0, 60) ?? r.url),
      plays: num(r.plays),
      likes: num(r.likes),
      comments: num(r.comments),
      posted: day(r.posted),
      lift: num(r.lift_vs_own_median),
    })),
  );
  return [
    { id: "profiles", columns: cols("profile:link sample:number plays_median:number plays_mean:number likes_median:number comments_median:number per_week:number confidence missing:flags"), rows: main },
    { id: "reels", columns: cols("profile reel:link plays:number likes:number comments:number posted lift:number"), rows: reels },
  ];
}

export function tablesFor(id: WorkflowId, result: Result): Table[] {
  if (id === "creators") return creatorTables(result);
  if (id === "brief") return briefTables(result);
  if (id === "ads") return adsTables(result);
  return reelsTables(result);
}

function sortValue(cell: Cell): string | number | null {
  if (cell === null) return null;
  if (Array.isArray(cell)) return cell.length;
  if (typeof cell === "object") return cell.label.toLowerCase();
  return typeof cell === "string" ? cell.toLowerCase() : cell;
}

export function sortRows(list: Record<string, Cell>[], key: string, descending: boolean): Record<string, Cell>[] {
  return [...list].sort((a, b) => {
    const x = sortValue(a[key] ?? null);
    const y = sortValue(b[key] ?? null);
    if (x === null && y === null) return 0;
    if (x === null) return 1;
    if (y === null) return -1;
    const order = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
    return descending ? -order : order;
  });
}

export function cellText(cell: Cell): string {
  if (cell === null) return "—";
  if (Array.isArray(cell)) return cell.length ? cell.join(", ") : "—";
  if (typeof cell === "object") return `[${cell.label}](${cell.href})`;
  return String(cell);
}

function escapeCell(text: string): string {
  return text.replace(/\|/g, "\\|").replace(/\s*\n\s*/g, " ");
}

export interface MarkdownInput {
  title: string;
  guide: string;
  caveats: string[];
  tables: { title: string; columns: string[]; rows: Cell[][] }[];
  data: unknown;
  labels: { guide: string; caveats: string; data: string; cut: string; empty: string };
  maxDataChars?: number;
}

export function toMarkdown(input: MarkdownInput): string {
  const parts: string[] = [`# ${input.title}`];
  if (input.guide) parts.push(`## ${input.labels.guide}`, input.guide);
  if (input.caveats.length) parts.push(`## ${input.labels.caveats}`, input.caveats.map((c) => `- ${c}`).join("\n"));
  for (const table of input.tables) {
    parts.push(`## ${table.title}`);
    if (!table.rows.length) {
      parts.push(input.labels.empty);
      continue;
    }
    const head = `| ${table.columns.map(escapeCell).join(" | ")} |`;
    const rule = `|${table.columns.map(() => "---").join("|")}|`;
    const body = table.rows.map((row) => `| ${row.map((cell) => escapeCell(cellText(cell))).join(" | ")} |`);
    parts.push([head, rule, ...body].join("\n"));
  }
  const max = input.maxDataChars ?? 40_000;
  const json = JSON.stringify(input.data, null, 1) ?? "";
  parts.push(`## ${input.labels.data}`, `\`\`\`json\n${json.length > max ? json.slice(0, max) : json}\n\`\`\``);
  if (json.length > max) parts.push(input.labels.cut);
  return `${parts.join("\n\n")}\n`;
}

export function evidenceOnly(result: Result): Result {
  const { guide: _guide, caveats: _caveats, ...rest } = result;
  return rest;
}
