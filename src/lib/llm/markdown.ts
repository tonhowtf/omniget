let loadPromise: Promise<typeof import("marked").marked> | null = null;

function getMarked(): Promise<typeof import("marked").marked> {
  if (!loadPromise) {
    loadPromise = import("marked").then((mod) => {
      mod.marked.setOptions({ gfm: true, breaks: true });
      return mod.marked;
    });
  }
  return loadPromise;
}

async function renderMarkdown(text: string): Promise<string> {
  if (!text) return "";
  try {
    const m = await getMarked();
    return m.parse(text, { async: false }) as string;
  } catch {
    return escapeHtml(text);
  }
}

const ALLOWED_TAGS = new Set([
  "p", "br", "hr", "em", "strong", "del", "code", "pre", "blockquote",
  "ul", "ol", "li", "a", "h1", "h2", "h3", "h4", "h5", "h6",
  "table", "thead", "tbody", "tr", "th", "td", "span", "div",
]);

const ALLOWED_ATTRS: Record<string, Set<string>> = {
  a: new Set(["href", "title"]),
  ol: new Set(["start"]),
  th: new Set(["align"]),
  td: new Set(["align"]),
  code: new Set(["class"]),
};

function isSafeUrl(url: string): boolean {
  const trimmed = (url ?? "").trim();
  if (!trimmed) return false;
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(trimmed)) {
    const scheme = trimmed.slice(0, trimmed.indexOf(":")).toLowerCase();
    return scheme === "http" || scheme === "https" || scheme === "mailto";
  }
  return !trimmed.startsWith("//");
}

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function hasDom(): boolean {
  return typeof document !== "undefined" && typeof document.createElement === "function";
}

interface SanitizeOptions {
  images?: boolean;
}

const SAFE_DATA_IMAGE = /^data:image\/(png|jpe?g|gif|webp|avif|bmp);base64,[a-z0-9+/=\s]+$/i;

function imageSource(src: string): { kind: "remote" | "data" | "local"; value: string } | null {
  const v = (src ?? "").trim();
  if (!v || v.startsWith("//")) return null;
  if (/^https?:/i.test(v)) return { kind: "remote", value: v };
  if (/^data:/i.test(v)) return SAFE_DATA_IMAGE.test(v) ? { kind: "data", value: v } : null;
  if (/^file:\/\//i.test(v)) {
    try {
      return { kind: "local", value: decodeURIComponent(v.slice(7)) };
    } catch {
      return null;
    }
  }
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(v) && !/^[a-zA-Z]:[\\/]/.test(v)) return null;
  try {
    return { kind: "local", value: decodeURIComponent(v) };
  } catch {
    return { kind: "local", value: v };
  }
}

function sanitizeHtml(html: string, opts: SanitizeOptions = {}): string {
  if (!hasDom()) return escapeHtml(html);
  const template = document.createElement("template");
  template.innerHTML = html;
  const walk = (node: Element) => {
    for (const child of Array.from(node.children)) {
      const tag = child.tagName.toLowerCase();
      if (tag === "img" && opts.images) {
        const src = imageSource(child.getAttribute("src") ?? "");
        const alt = child.getAttribute("alt") ?? "";
        const title = child.getAttribute("title");
        for (const attr of Array.from(child.attributes)) child.removeAttribute(attr.name);
        if (!src) {
          child.replaceWith(document.createTextNode(alt));
          continue;
        }
        if (src.kind === "local") child.setAttribute("data-local-src", src.value);
        else child.setAttribute("src", src.value);
        child.setAttribute("alt", alt);
        if (title) child.setAttribute("title", title);
        child.setAttribute("loading", "lazy");
        continue;
      }
      if (!ALLOWED_TAGS.has(tag)) {
        const text = tag === "script" || tag === "style" ? "" : child.textContent || "";
        child.replaceWith(document.createTextNode(text));
        continue;
      }
      const allowed = ALLOWED_ATTRS[tag];
      for (const attr of Array.from(child.attributes)) {
        const name = attr.name.toLowerCase();
        if (!allowed?.has(name)) {
          child.removeAttribute(attr.name);
          continue;
        }
        if (name === "href" && !isSafeUrl(attr.value)) child.removeAttribute(attr.name);
      }
      if (tag === "a") {
        child.setAttribute("rel", "noopener noreferrer");
        child.setAttribute("target", "_blank");
      }
      walk(child);
    }
  };
  walk(template.content as unknown as Element);
  return template.innerHTML;
}

export async function renderSafeMarkdown(text: string, opts: SanitizeOptions = {}): Promise<string> {
  if (!text) return "";
  return sanitizeHtml(await renderMarkdown(text), opts);
}

export function renderSafeMarkdownSync(
  text: string,
  cache: Map<string, string>,
  onReady?: () => void,
  opts: SanitizeOptions = {},
): string {
  if (!text) return "";
  const hit = cache.get(text);
  if (hit !== undefined) return hit;
  void renderSafeMarkdown(text, opts).then((html) => {
    cache.set(text, html);
    onReady?.();
  });
  return `<p>${escapeHtml(text).replace(/\n/g, "<br>")}</p>`;
}
