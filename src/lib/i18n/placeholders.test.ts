import { afterEach, describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { get } from "svelte/store";
import { loadTranslations, locale, t } from "./index";

// sveltekit-i18n's default parser does not interpolate one-letter
// placeholders: "attempt {{n}}/{{max}}" renders as "attempt /3".
const here = dirname(fileURLToPath(import.meta.url));
const locales = readdirSync(here).filter((f) => f.endsWith(".json"));

function offenders(tree: unknown, path = ""): string[] {
  if (typeof tree === "string") return /\{\{\s*[A-Za-z]\s*(?:[;:,][^}]*)?\}\}/.test(tree) ? [path] : [];
  if (!tree || typeof tree !== "object") return [];
  return Object.entries(tree as Record<string, unknown>).flatMap(([k, v]) => offenders(v, path ? `${path}.${k}` : k));
}

describe("locale placeholders", () => {
  afterEach(() => locale.set("en"));

  it.each(locales)("%s has no one-letter {{x}} placeholder", (file) => {
    const tree = JSON.parse(readFileSync(join(here, file), "utf8"));
    expect(offenders(tree)).toEqual([]);
  });

  // N-5: the MCP login card showed "{client} asks for a {target} login" and
  // "Expires at {time}": single braces are not placeholders for this parser,
  // and a `{ values: {...} }` payload is not read either.
  it.each(locales)("%s MCP login card uses {{}} placeholders", (file) => {
    const tree = JSON.parse(readFileSync(join(here, file), "utf8"));
    expect(tree.mcp_auth.body).toContain("{{client}}");
    expect(tree.mcp_auth.body).toContain("{{target}}");
    expect(tree.mcp_auth.expires).toContain("{{time}}");
    for (const key of ["restored", "reconciled", "attention"]) expect(tree.recovery[key]).toContain("{{count}}");
  });

  it("renders the MCP login card and the recovery toast with their values", async () => {
    await loadTranslations("en");
    locale.set("en");
    const tr = get(t);
    const body = tr("mcp_auth.body", { client: "Codex", target: "YouTube" });
    expect(body).toContain("Codex asks for a YouTube login");
    expect(tr("mcp_auth.expires", { time: "14:05" })).toBe("Expires at 14:05");
    expect(tr("recovery.reconciled", { count: 2 })).toContain("2");
    expect(tr("recovery.attention", { count: 1 })).toContain("1");
    for (const s of [body, tr("recovery.restored", { count: 3 })]) expect(s).not.toMatch(/[{}]/);
  });

  it("no component wraps a $t payload in { values }", () => {
    // sveltekit-i18n reads the payload object itself; `{ values: {...} }`
    // drops every placeholder (the recovery toast read " download(s) resumed").
    const root = join(here, "..", "..");
    const walk = (dir: string): string[] =>
      readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
        e.isDirectory() ? walk(join(dir, e.name)) : e.name.endsWith(".svelte") ? [join(dir, e.name)] : [],
      );
    const offenders = walk(root).filter((f) => /\$t\([^)]*\{\s*values\s*:/.test(readFileSync(f, "utf8")));
    expect(offenders).toEqual([]);
  });

  it("interpolates a renamed counter through the real translator", async () => {
    await loadTranslations("en");
    locale.set("en");
    const rendered = get(t)("mission.detail.attempts", { count: 2, max: 3 });
    expect(rendered).toContain("2");
    expect(rendered).toContain("3");
    expect(rendered).not.toMatch(/[{}]/);
  });
});
