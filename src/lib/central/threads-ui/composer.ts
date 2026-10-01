import type { Chip } from "./ui-state.svelte";

type TriggerKind = "command" | "skill" | "file" | "pr";
export type Trigger = { kind: TriggerKind; query: string; start: number; end: number };

export function detectTrigger(text: string, caret: number): Trigger | null {
  const before = text.slice(0, caret);
  const lineStart = before.lastIndexOf("\n") + 1;
  const line = before.slice(lineStart);
  const slash = /^\/(\S*)$/.exec(line);
  if (slash) return { kind: "command", query: slash[1], start: lineStart, end: caret };
  const m = /(^|\s)([$@#])([^\s]*)$/.exec(before);
  if (!m) return null;
  const sym = m[2];
  const query = m[3];
  const start = caret - query.length - 1;
  if (sym === "#" && query && !/^[\p{L}\p{N}][\p{L}\p{N}_-]*$/u.test(query)) return null;
  if (sym === "$" && /^\d/.test(query)) return null;
  return { kind: sym === "$" ? "skill" : sym === "@" ? "file" : "pr", query, start, end: caret };
}

export function replaceTrigger(text: string, trig: Trigger, insert: string): { text: string; caret: number } {
  let end = trig.end;
  if (text[end] === " ") end++;
  const next = text.slice(0, trig.start) + insert + " " + text.slice(end);
  return { text: next, caret: trig.start + insert.length + 1 };
}

export function score(name: string, q: string): number {
  if (!q) return 50;
  const n = name.toLowerCase();
  const s = q.toLowerCase();
  if (n === s) return 0;
  if (n.startsWith(s)) return 2;
  if (new RegExp(`[-_/.:\\s]${s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`).test(n)) return 4;
  if (n.includes(s)) return 6;
  let i = 0;
  for (const ch of n) if (ch === s[i]) i++;
  return i === s.length ? 100 : -1;
}

export function rank<T>(items: T[], q: string, key: (x: T) => string, desc?: (x: T) => string): T[] {
  return items
    .map((x) => {
      let sc = score(key(x), q);
      if (sc < 0 && desc) {
        const d = score(desc(x), q);
        sc = d < 0 ? -1 : 20 + d;
      }
      return { x, sc };
    })
    .filter((r) => r.sc >= 0)
    .sort((a, b) => a.sc - b.sc)
    .map((r) => r.x);
}

function escapeEnvelope(s: string): string {
  return s.replace(/<\/?omniget_context[^>]*>/gi, (m) => m.replace(/</g, "&lt;"));
}

export function composeMessage(text: string, chips: Chip[]): string {
  const blocks = chips.filter((c) => c.kind === "pr" || c.kind === "terminal" || c.kind === "review");
  const attachments = chips.filter((c) => c.kind === "attachment");
  let out = text.trim();
  if (attachments.length) {
    const refs = attachments.map((a) => `@${a.path ?? a.label}`).join(" ");
    out = out ? `${out}\n\n${refs}` : refs;
  }
  if (!blocks.length) return out;
  const body = blocks
    .map((c) => `<item kind="${c.kind}" ref="omniget-context://v1/${c.kind}/${c.id}" label="${c.label.replace(/"/g, "'")}">\n${escapeEnvelope(c.value)}\n</item>`)
    .join("\n");
  const links = blocks.map((c) => `[${c.label}](omniget-context://v1/${c.kind}/${c.id})`).join(" ");
  return `${out}${out ? "\n\n" : ""}${links}\n\n<omniget_context>\n${body}\n</omniget_context>`;
}

export function recallable(text: string): string {
  return text
    .replace(/\n*<omniget_context>[\s\S]*?<\/omniget_context>\s*$/, "")
    .replace(/\[[^\]]*\]\(omniget-context:\/\/v1\/[^)]*\)/g, "")
    .trim();
}

export function attachmentsOf(chips: Chip[]): { kind: "file"; name: string; path: string }[] {
  return chips.filter((c) => c.kind === "attachment" && c.path).map((c) => ({ kind: "file" as const, name: c.label, path: c.path! }));
}

type SlashCommand = { name: string; descKey: string; only?: "empty" };

export const BUILTIN_COMMANDS: SlashCommand[] = [
  { name: "model", descKey: "llm.central.threads.cmd.model" },
  { name: "plan", descKey: "llm.central.threads.cmd.plan" },
  { name: "default", descKey: "llm.central.threads.cmd.default" },
  { name: "access", descKey: "llm.central.threads.cmd.access" },
  { name: "fork", descKey: "llm.central.threads.cmd.fork" },
  { name: "new", descKey: "llm.central.threads.cmd.new" },
  { name: "diff", descKey: "llm.central.threads.cmd.diff" },
  { name: "terminal", descKey: "llm.central.threads.cmd.terminal" },
  { name: "compact", descKey: "llm.central.threads.cmd.compact", only: "empty" },
];
