const CHUNK = 256 * 1024;
const MAX_CHUNKS = 96;

class RemoteFile {
  name: string;
  type: string;
  size: number;
  private url: string;
  private chunks = new Map<number, Promise<Uint8Array>>();

  private constructor(url: string, name: string, type: string, size: number) {
    this.url = url;
    this.name = name;
    this.type = type;
    this.size = size;
  }

  static async open(url: string, name: string, type = ""): Promise<RemoteFile> {
    const r = await fetch(url, { headers: { Range: "bytes=0-0" } });
    if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
    const range = r.headers.get("content-range");
    const size = range ? Number(range.split("/")[1]) : Number(r.headers.get("content-length") ?? 0);
    await r.arrayBuffer().catch(() => undefined);
    return new RemoteFile(url, name, type || r.headers.get("content-type") || "", size);
  }

  private chunk(i: number): Promise<Uint8Array> {
    let p = this.chunks.get(i);
    if (p) {
      this.chunks.delete(i);
      this.chunks.set(i, p);
      return p;
    }
    const start = i * CHUNK;
    const end = Math.min(this.size, start + CHUNK) - 1;
    p = fetch(this.url, { headers: { Range: `bytes=${start}-${end}` } }).then(async (r) => {
      if (!r.ok) throw new Error(`${r.status}`);
      return new Uint8Array(await r.arrayBuffer());
    });
    p.catch(() => this.chunks.delete(i));
    this.chunks.set(i, p);
    while (this.chunks.size > MAX_CHUNKS) {
      const first = this.chunks.keys().next().value;
      if (first === undefined) break;
      this.chunks.delete(first);
    }
    return p;
  }

  async read(start: number, end: number): Promise<Uint8Array> {
    start = Math.max(0, Math.min(start, this.size));
    end = Math.max(start, Math.min(end, this.size));
    const out = new Uint8Array(end - start);
    if (!out.length) return out;
    const first = Math.floor(start / CHUNK);
    const last = Math.floor((end - 1) / CHUNK);
    const parts = await Promise.all(
      Array.from({ length: last - first + 1 }, (_, k) => this.chunk(first + k)),
    );
    let off = 0;
    parts.forEach((c, k) => {
      const cs = (first + k) * CHUNK;
      const a = Math.max(start, cs) - cs;
      const b = Math.min(end, cs + c.length) - cs;
      out.set(c.subarray(a, b), off);
      off += b - a;
    });
    return out;
  }

  slice(start = 0, end = this.size, contentType = "") {
    if (start < 0) start = Math.max(0, this.size + start);
    if (end < 0) end = Math.max(0, this.size + end);
    const self = this;
    const size = Math.max(0, Math.min(end, this.size) - start);
    return {
      size,
      type: contentType,
      arrayBuffer: async () => (await self.read(start, end)).slice().buffer,
      text: async () => new TextDecoder().decode(await self.read(start, end)),
      slice: (a = 0, b = size) => self.slice(start + a, start + Math.min(b, size)),
      stream: () =>
        new ReadableStream({
          async start(ctrl) {
            ctrl.enqueue(await self.read(start, end));
            ctrl.close();
          },
        }),
    };
  }

  async arrayBuffer() {
    return (await this.read(0, this.size)).slice().buffer;
  }

  async text() {
    return new TextDecoder().decode(await this.read(0, this.size));
  }

  stream() {
    return this.slice(0, this.size).stream();
  }
}

const DROP = new Set(["script", "iframe", "frame", "frameset", "object", "embed", "applet", "base", "portal"]);
const URL_ATTRS = new Set(["href", "src", "xlink:href", "action", "formaction", "data", "poster", "background", "srcset"]);

function badUrl(v: string) {
  const s = v.replace(/[\u0000- ]/g, "").toLowerCase();
  return s.startsWith("javascript:") || s.startsWith("vbscript:") || s.startsWith("data:text/html") || s.startsWith("data:application/");
}

function clean(root: Element) {
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
  const remove: Element[] = [];
  let n = walker.currentNode as Element | null;
  while (n) {
    const name = n.localName.toLowerCase();
    if (DROP.has(name) || (name === "meta" && n.hasAttribute("http-equiv"))) {
      remove.push(n);
    } else {
      for (const a of Array.from(n.attributes)) {
        const an = a.name.toLowerCase();
        if (an.startsWith("on") || an === "srcdoc" || (URL_ATTRS.has(an) && badUrl(a.value))) {
          n.removeAttributeNode(a);
        }
      }
    }
    n = walker.nextNode() as Element | null;
  }
  for (const r of remove) r.remove();
}

function sanitizeMarkup(data: string, type: string): string {
  const isXml = /xml|svg/.test(type);
  const parser = new DOMParser();
  let doc = parser.parseFromString(data, (isXml ? type.split(";")[0] : "text/html") as DOMParserSupportedType);
  if (isXml && doc.getElementsByTagName("parsererror").length) {
    doc = parser.parseFromString(data, "text/html");
    clean(doc.documentElement);
    return "<!DOCTYPE html>\n" + doc.documentElement.outerHTML;
  }
  clean(doc.documentElement);
  if (isXml) {
    const s = new XMLSerializer().serializeToString(doc);
    return s.startsWith("<?xml") ? s : `<?xml version="1.0" encoding="UTF-8"?>\n${s}`;
  }
  return "<!DOCTYPE html>\n" + doc.documentElement.outerHTML;
}

function guardBook(book: any) {
  const t: EventTarget | undefined = book?.transformTarget;
  if (!t) return;
  t.addEventListener("load", (e: Event) => {
    const d = (e as CustomEvent).detail;
    if (d?.isScript) d.allow = false;
  });
  t.addEventListener("data", (e: Event) => {
    const d = (e as CustomEvent).detail;
    const type = String(d?.type ?? "");
    if (!/html|svg/.test(type)) return;
    d.data = Promise.resolve(d.data).then((x: unknown) => (typeof x === "string" ? sanitizeMarkup(x, type) : x));
  });
}

let loaded: Promise<unknown> | null = null;

export function loadFoliate() {
  loaded ??= import("$lib/vendor/foliate-js/view.js");
  return loaded;
}

export async function openBookFile(url: string, name: string, type = "") {
  await loadFoliate();
  const file = await RemoteFile.open(url, name, type);
  const { makeBook } = (await import("$lib/vendor/foliate-js/view.js")) as any;
  const book = await makeBook(file);
  guardBook(book);
  return book;
}

export type ReaderPrefs = {
  theme: string;
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  paragraphSpacing: number;
  letterSpacing: number;
  wordSpacing: number;
  textIndent: number;
  justify: boolean;
  hyphenate: boolean;
  fontWeight: number;
  overrideFont: boolean;
  overrideColors: boolean;
  bionic: boolean;
  flow: "paginated" | "scrolled";
  maxColumns: number;
  maxWidth: number;
  margin: number;
  gap: number;
  animated: boolean;
  brightness: number;
  invertImages: boolean;
  customCss: string;
  comicMode: "ltr" | "rtl" | "vertical" | "webtoon";
  comicFit: "width" | "height" | "contain" | "original";
  comicSpread: "single" | "double" | "auto";
  comicCoverAlone: boolean;
  comicBackground: string;
  comicGap: number;
  comicCrop: boolean;
  comicPreload: number;
  comicFilter: "none" | "grayscale" | "sepia" | "invert";
  comicAutoNext: boolean;
  tapZones: "sides" | "kindle" | "edge" | "off";
  ttsRate: number;
  ttsPitch: number;
  ttsVoice: string;
  dictionaryLang: string;
  translateTo: string;
  showClock: boolean;
  showProgress: boolean;
  keepAwake: boolean;
  rulerLines: number;
  translatorUrl: string;
  translatorLang: string;
  rsvpWpm: number;
  incognito: boolean;
};

export const THEMES: Record<string, { bg: string; fg: string; link: string; name: string }> = {
  light: { bg: "#ffffff", fg: "#1a1a1a", link: "#1e6fe8", name: "Light" },
  sepia: { bg: "#f4ecd8", fg: "#5b4636", link: "#8a5a00", name: "Sepia" },
  paper: { bg: "#f7f4ee", fg: "#2b2b2b", link: "#2f6f9f", name: "Paper" },
  gray: { bg: "#e6e6e6", fg: "#222222", link: "#1e5bb8", name: "Gray" },
  mint: { bg: "#e6f2ea", fg: "#1f3326", link: "#2c7a4b", name: "Mint" },
  dusk: { bg: "#2b2a33", fg: "#d6d3cc", link: "#9ab8ff", name: "Dusk" },
  dark: { bg: "#121212", fg: "#dcdcdc", link: "#7fb0ff", name: "Dark" },
  black: { bg: "#000000", fg: "#c8c8c8", link: "#6fa0ff", name: "OLED" },
  solarized: { bg: "#fdf6e3", fg: "#586e75", link: "#268bd2", name: "Solarized" },
  nord: { bg: "#2e3440", fg: "#d8dee9", link: "#88c0d0", name: "Nord" },
};

export const FONTS: { id: string; label: string; css: string }[] = [
  { id: "publisher", label: "Publisher", css: "" },
  { id: "serif", label: "Serif", css: "Georgia, 'Iowan Old Style', 'Palatino Linotype', 'Times New Roman', serif" },
  { id: "sans", label: "Sans", css: "system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif" },
  { id: "literata", label: "Book", css: "Literata, 'Bookerly', Charter, 'Bitstream Charter', Georgia, serif" },
  { id: "mono", label: "Mono", css: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace" },
  { id: "dyslexic", label: "OpenDyslexic", css: "'OpenDyslexic', 'Comic Sans MS', sans-serif" },
];

export function isDarkTheme(p: Pick<ReaderPrefs, "theme">) {
  return ["dusk", "dark", "black", "nord"].includes(p.theme);
}

export function readerCss(p: ReaderPrefs): string {
  const t = THEMES[p.theme] ?? THEMES.light;
  const font = FONTS.find((f) => f.id === p.fontFamily)?.css ?? "";
  const imp = p.overrideFont ? " !important" : "";
  const colors = p.overrideColors || p.theme !== "light";
  return `
  @namespace epub "http://www.idpf.org/2007/ops";
  html {
    color-scheme: ${isDarkTheme(p) ? "dark" : "light"};
    ${colors ? `color: ${t.fg}; background: ${t.bg} !important;` : ""}
    font-size: ${p.fontSize}px !important;
    ${p.hyphenate ? "hyphens: auto; -webkit-hyphens: auto;" : "hyphens: manual;"}
    -webkit-text-size-adjust: none;
  }
  body { ${colors ? `color: ${t.fg} !important; background: transparent !important;` : ""} }
  ${colors ? `a:any-link { color: ${t.link} !important; }` : ""}
  ${
    font
      ? `body, p, div, span, li, blockquote, dd, td { font-family: ${font}${imp}; }`
      : ""
  }
  p, li, blockquote, dd {
    line-height: ${p.lineHeight}${imp};
    ${p.justify ? `text-align: justify${imp};` : ""}
    letter-spacing: ${p.letterSpacing}em;
    word-spacing: ${p.wordSpacing}em;
    font-weight: ${p.fontWeight === 400 ? "inherit" : p.fontWeight};
  }
  p { margin-block: ${p.paragraphSpacing}em${p.paragraphSpacing > 0 ? " !important" : ""};
      ${p.textIndent >= 0 ? `text-indent: ${p.textIndent}em${imp};` : ""} }
  pre { white-space: pre-wrap !important; }
  img, svg, video { max-width: 100%; height: auto; object-fit: contain; }
  ${isDarkTheme(p) && p.invertImages ? "img { filter: invert(1) hue-rotate(180deg); }" : ""}
  aside[epub|type~="footnote"], aside[epub|type~="endnote"] { display: none; }
  ${colors ? `* { border-color: color-mix(in srgb, ${t.fg} 25%, transparent) !important; }` : ""}
  .omniget-bionic b { font-weight: 700; }
  ${p.customCss ?? ""}
  `;
}

export function applyBionic(doc: Document) {
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      n.parentElement && !/^(script|style|b|strong|code|pre)$/i.test(n.parentElement.tagName) && n.nodeValue?.trim()
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_REJECT,
  });
  const nodes: Text[] = [];
  let n = walker.nextNode();
  while (n) {
    nodes.push(n as Text);
    n = walker.nextNode();
  }
  for (const t of nodes) {
    const frag = doc.createDocumentFragment();
    for (const part of (t.nodeValue ?? "").split(/(\s+)/)) {
      if (!part.trim() || part.length < 2) {
        frag.append(part);
        continue;
      }
      const k = Math.ceil(part.length / 2);
      const b = doc.createElement("b");
      b.textContent = part.slice(0, k);
      frag.append(b, part.slice(k));
    }
    const span = doc.createElement("span");
    span.className = "omniget-bionic";
    span.append(frag);
    t.replaceWith(span);
  }
}
