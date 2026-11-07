// @ts-nocheck
const KEY_CALL = /\$t\(\s*["']([A-Za-z0-9_.-]+)["']/g;
const HARDCODED_TEXT = />\s*([A-ZÀ-Ý][a-zà-ÿ]+(?:[ ,'’-]+[a-zà-ÿ]+){1,}[.!?…]?)\s*</g;
const HARDCODED_ATTR = /\b(?:placeholder|title|aria-label)="([A-ZÀ-Ý][a-zà-ÿ]+(?: [a-zà-ÿ]+)+[.!?…]?)"/g;

export function missingKeys(source, keys) {
  return [...source.matchAll(KEY_CALL)].map((m) => m[1]).filter((k) => !keys.has(k));
}

export function hardcodedStrings(svelte) {
  const template = svelte
    .replace(/<script[\s\S]*?<\/script>/g, "")
    .replace(/<style[\s\S]*?<\/style>/g, "")
    .replace(/<svg[\s\S]*?<\/svg>/g, "")
    .replace(/<(pre|code)[\s\S]*?<\/\1>/g, "");
  return [...template.matchAll(HARDCODED_TEXT), ...template.matchAll(HARDCODED_ATTR)].map((m) => m[1]);
}
