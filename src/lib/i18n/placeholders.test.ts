import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));

function walk(node: unknown, path: string, out: string[]) {
  if (typeof node === "string") {
    if (/\{\{\s*[A-Za-z]\s*\}\}/.test(node)) out.push(path);
    return;
  }
  if (node && typeof node === "object") {
    for (const [k, v] of Object.entries(node)) walk(v, path ? `${path}.${k}` : k, out);
  }
}

const LITERAL_BRACES = new Set([
  "music.convert.tokens",
  "settings.download.bilibili_naming_desc",
  "settings.download.bilibili_naming_help",
  "read.settings.organize_hint",
]);

function walkSingle(node: unknown, path: string, out: string[]) {
  if (typeof node === "string") {
    if (/(?<!\{)\{[A-Za-z_]+\}(?!\})/.test(node) && !LITERAL_BRACES.has(path)) out.push(path);
    return;
  }
  if (node && typeof node === "object") {
    for (const [k, v] of Object.entries(node)) walkSingle(v, path ? `${path}.${k}` : k, out);
  }
}

describe("i18n placeholders", () => {
  it("never uses single braces, which the parser leaves on screen as typed", () => {
    const bad: string[] = [];
    for (const file of readdirSync(dir).filter((f) => f.endsWith(".json"))) {
      const found: string[] = [];
      walkSingle(JSON.parse(readFileSync(join(dir, file), "utf8")), "", found);
      bad.push(...found.map((k) => `${file}: ${k}`));
    }
    expect(bad).toEqual([]);
  });

  it("never uses a one-letter name, which the parser renders as empty", () => {
    const bad: string[] = [];
    for (const file of readdirSync(dir).filter((f) => f.endsWith(".json"))) {
      const found: string[] = [];
      walk(JSON.parse(readFileSync(join(dir, file), "utf8")), "", found);
      bad.push(...found.map((k) => `${file}: ${k}`));
    }
    expect(bad).toEqual([]);
  });
});
