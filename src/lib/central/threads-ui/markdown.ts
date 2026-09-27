// Incremental Markdown for streamed answers: the text is split at the last
// block boundary outside an open code fence. The stable prefix renders once
// (cached by its exact text) and only the short tail re-renders per delta.
// File-looking inline code becomes a clickable chip after sanitising.

import { escapeHtml, renderSafeMarkdownSync } from "$lib/llm/markdown";

/** Index where the still-growing tail starts (0 = everything is tail). */
export function stableSplit(text: string): number {
  let inFence = false;
  let fenceMark = "";
  let lastBoundary = 0;
  let pos = 0;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const lineEnd = pos + line.length + 1;
    const fence = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (!inFence) {
        inFence = true;
        fenceMark = fence[1][0];
      } else if (fence[1][0] === fenceMark) {
        inFence = false;
        if (i < lines.length - 1) lastBoundary = Math.min(lineEnd, text.length);
      }
    } else if (!inFence && line.trim() === "" && i < lines.length - 1) {
      lastBoundary = Math.min(lineEnd, text.length);
    }
    pos = lineEnd;
  }
  return lastBoundary;
}

const PATHISH = /^(?:\.{0,2}\/)?(?:[\w@.+-]+\/)*[\w@+-][\w.@+-]*\.[A-Za-z0-9]{1,8}(?::\d+(?::\d+)?)?$|^(?:\.{0,2}\/)?(?:[\w@.+-]+\/)+[\w@.+-]+\/?$/;

export function looksLikePath(s: string): boolean {
  if (s.length > 240 || /\s/.test(s)) return false;
  if (/^https?:/.test(s)) return false;
  if (/^\d+(\.\d+)*$/.test(s)) return false;
  return PATHISH.test(s);
}

/** Marks path-like `<code>` as file chips (data-path) — runs on sanitised HTML. */
export function linkFileChips(html: string): string {
  return html.replace(/<code>([^<]{1,240})<\/code>/g, (whole, inner: string) => {
    const raw = inner.replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"');
    if (!looksLikePath(raw)) return whole;
    const path = raw.replace(/:\d+(?::\d+)?$/, "");
    return `<code class="file-chip" role="button" tabindex="0" data-path="${escapeHtml(path)}">${inner}</code>`;
  });
}

export class IncrementalMarkdown {
  #head = new Map<string, string>();
  #tail = new Map<string, string>();
  #onReady: () => void;

  constructor(onReady: () => void) {
    this.#onReady = onReady;
  }

  render(text: string, streaming: boolean): string {
    if (!text) return "";
    if (!streaming) return linkFileChips(renderSafeMarkdownSync(text, this.#head, this.#onReady));
    const cut = stableSplit(text);
    const head = cut > 0 ? renderSafeMarkdownSync(text.slice(0, cut), this.#head, this.#onReady) : "";
    const tailText = text.slice(cut);
    const tail = tailText ? renderSafeMarkdownSync(tailText, this.#tail, this.#onReady) : "";
    trim(this.#tail, 6);
    trim(this.#head, 12);
    return linkFileChips(head + tail);
  }

  clear() {
    this.#head.clear();
    this.#tail.clear();
  }
}

function trim(m: Map<string, string>, max: number) {
  if (m.size <= max) return;
  for (const k of [...m.keys()].slice(0, m.size - max)) m.delete(k);
}
