import { escapeHtml } from "$lib/markdown";

const ALLOWED = new Set(["p", "i", "b", "ol", "ul", "li", "em", "strong"]);
const TAG_OR_BRACKET = /<(\/?)([a-z][a-z0-9]*)\b(?:"[^"]*"|'[^']*'|[^>"'])*>|</gi;

type DictionaryEntry = {
  partOfSpeech?: unknown;
  definitions?: { definition?: unknown }[];
};

export function plainDefinitionHtml(html: string): string {
  return html
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<(script|style)\b[\s\S]*?<\/\1\s*>/gi, "")
    .replace(TAG_OR_BRACKET, (_whole, slash: string | undefined, name: string | undefined) => {
      if (!name) return "&lt;";
      const tag = name.toLowerCase();
      return ALLOWED.has(tag) ? `<${slash}${tag}>` : "";
    });
}

export function definitionsHtml(entries: DictionaryEntry[]): string {
  return entries
    .slice(0, 4)
    .map((entry) => {
      const items = (entry.definitions ?? [])
        .slice(0, 4)
        .map((d) => `<li>${plainDefinitionHtml(String(d.definition ?? ""))}</li>`)
        .join("");
      return `<p><i>${escapeHtml(String(entry.partOfSpeech ?? ""))}</i></p><ol>${items}</ol>`;
    })
    .join("");
}
