import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const dir = path.resolve(__dirname, "../i18n");
const load = (f: string) => JSON.parse(readFileSync(path.join(dir, f), "utf8"));
const en = load("en.json");
const locales = readdirSync(dir).filter((f) => f.endsWith(".json") && f !== "en.json");

function flat(obj: Record<string, unknown>, prefix = ""): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object") Object.assign(out, flat(v as Record<string, unknown>, key));
    else out[key] = String(v);
  }
  return out;
}

const placeholders = (s: string) => [...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();

describe("Home copy in every locale", () => {
  const enHome = flat(en.home);
  it.each(locales)("%s has exactly the Home keys of en", (file) => {
    const home = flat(load(file).home ?? {});
    expect(Object.keys(home).sort()).toEqual(Object.keys(enHome).sort());
  });

  it.each(locales)("%s keeps placeholders and one [link] span", (file) => {
    const home = flat(load(file).home ?? {});
    for (const [key, value] of Object.entries(enHome)) {
      expect(placeholders(home[key] ?? ""), `${file} home.${key}`).toEqual(placeholders(value));
    }
    for (const key of ["first_sites", "first_terms"]) {
      expect(home[key], `${file} home.${key}`).toMatch(/^[^[\]]*\[[^[\]]+\][^[\]]*$/);
    }
  });

  it.each(locales)("%s drops the old terms footer", (file) => {
    expect(load(file).terms_note).toBeUndefined();
  });
});
