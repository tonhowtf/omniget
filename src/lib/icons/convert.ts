const RENAMES: Record<string, string> = {
  class: "className",
  for: "htmlFor",
  "xlink:href": "xlinkHref",
  "xml:space": "xmlSpace",
  "xml:lang": "xmlLang",
  "xmlns:xlink": "xmlnsXlink",
  tabindex: "tabIndex",
};

export function jsxAttrName(name: string): string {
  if (RENAMES[name]) return RENAMES[name];
  if (name.startsWith("data-") || name.startsWith("aria-")) return name;
  return name.replace(/[-:]([a-z])/g, (_, c: string) => c.toUpperCase());
}

export function styleObject(style: string): string {
  const pairs = style
    .split(";")
    .map((s) => s.trim())
    .filter(Boolean)
    .map((decl) => {
      const i = decl.indexOf(":");
      if (i < 0) return null;
      const key = decl.slice(0, i).trim();
      const value = decl.slice(i + 1).trim();
      const prop = key.startsWith("--") ? JSON.stringify(key) : key.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());
      return `${prop}: ${JSON.stringify(value)}`;
    })
    .filter((p): p is string => p !== null);
  return `{{ ${pairs.join(", ")} }}`;
}

const TAG = /<([a-zA-Z][\w:.-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'))?)*)\s*(\/?)>/g;
const ATTR = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'))?/g;

function jsxAttrs(raw: string): string {
  const out: string[] = [];
  for (const m of raw.matchAll(ATTR)) {
    const name = jsxAttrName(m[1]);
    const value = m[2] ?? m[3];
    if (value === undefined) out.push(name);
    else if (name === "style") out.push(`style=${styleObject(value)}`);
    else out.push(`${name}=${JSON.stringify(value)}`);
  }
  return out.length ? " " + out.join(" ") : "";
}

function escapeJsxText(text: string): string {
  return text.replace(/[{}]/g, (c) => `{"${c}"}`);
}

export function svgToJsx(svg: string): string {
  let out = "";
  let last = 0;
  for (const m of svg.matchAll(TAG)) {
    out += escapeJsxText(svg.slice(last, m.index));
    out += `<${m[1]}${jsxAttrs(m[2] ?? "")}${m[3] ? " /" : ""}>`;
    last = (m.index ?? 0) + m[0].length;
  }
  return out + escapeJsxText(svg.slice(last));
}

export function componentName(name: string): string {
  const pascal = name
    .split(/[^a-zA-Z0-9]+/)
    .filter(Boolean)
    .map((p) => p[0].toUpperCase() + p.slice(1))
    .join("");
  if (!pascal) return "Icon";
  return /^[0-9]/.test(pascal) ? `Icon${pascal}` : pascal;
}

function spreadRoot(markup: string, spread: string): string {
  return markup.replace(/^<svg\b([^>]*?)(\s*\/?)>/, (_, attrs: string, close: string) => `<svg${attrs} ${spread}${close}>`);
}

export function toReact(svg: string, name: string): string {
  const comp = componentName(name);
  const jsx = spreadRoot(svgToJsx(svg), "{...props}");
  return [
    `import type { SVGProps } from "react";`,
    ``,
    `export function ${comp}(props: SVGProps<SVGSVGElement>) {`,
    `  return (`,
    `    ${jsx}`,
    `  );`,
    `}`,
    ``,
  ].join("\n");
}

export function escapeSvelte(svg: string): string {
  return svg.replace(/\{/g, "&#123;").replace(/\}/g, "&#125;");
}

export function toSvelte(svg: string): string {
  const markup = spreadRoot(escapeSvelte(svg), "{...props}");
  return [
    `<script lang="ts">`,
    `  import type { SVGAttributes } from "svelte/elements";`,
    ``,
    `  let props: SVGAttributes<SVGSVGElement> = $props();`,
    `</script>`,
    ``,
    markup,
    ``,
  ].join("\n");
}
