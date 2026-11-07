import type { LicenseClass } from "./types";

export function classifyLicense(license: string | null | undefined): LicenseClass {
  const l = (license ?? "").trim().toLowerCase();
  if (l.startsWith("ofl") || l.includes("open font license") || l.startsWith("apache")) return "open";
  if (l === "mit" || l.startsWith("mit ") || l.startsWith("mit-")) return "open";
  return "other";
}

const NAMES: [RegExp, string, string][] = [
  [/^ofl/i, "SIL Open Font License", "https://openfontlicense.org"],
  [/^apache/i, "Apache License 2.0", "https://www.apache.org/licenses/LICENSE-2.0"],
  [/^mit/i, "MIT License", "https://opensource.org/license/mit"],
  [/^ufl/i, "Ubuntu Font License", "https://ubuntu.com/legal/font-licence"],
  [/^cc0/i, "CC0 1.0", "https://creativecommons.org/publicdomain/zero/1.0/"],
  [/^unlicense/i, "The Unlicense", "https://unlicense.org"],
];

export function licenseName(license: string): string {
  const trimmed = license.trim();
  const hit = NAMES.find(([re]) => re.test(trimmed));
  if (!hit) return trimmed;
  const version = trimmed.match(/(\d+(?:\.\d+)?)/)?.[1];
  return hit[0].source === "^ofl" && version ? `${hit[1]} ${version}` : hit[1];
}

export function licenseUrl(license: string): string | null {
  return NAMES.find(([re]) => re.test(license.trim()))?.[2] ?? null;
}
