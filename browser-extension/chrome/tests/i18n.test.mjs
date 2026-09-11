import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";

const localesDir = new URL("../_locales/", import.meta.url);
// Every file that reads a message by key. Keys are always string literals, so
// they can be found without running the page.
const SOURCES = ["../popup/popup.js", "../pages/options.js", "../pages/error-content.js"];

async function readLocale(name) {
  return JSON.parse(await readFile(new URL(`${name}/messages.json`, localesDir), "utf8"));
}

async function localeNames() {
  const entries = await readdir(localesDir, { withFileTypes: true });
  return entries.filter(entry => entry.isDirectory()).map(entry => entry.name);
}

async function usedKeys() {
  const keys = new Set();
  for (const source of SOURCES) {
    let code;
    try {
      code = await readFile(new URL(source, import.meta.url), "utf8");
    } catch {
      continue;
    }
    for (const match of code.matchAll(/\btr\(\s*"([a-z0-9_]+)"/g)) {
      keys.add(match[1]);
    }
  }
  return keys;
}

test("every locale carries exactly the same set of keys", async () => {
  const names = await localeNames();
  assert.ok(names.includes("en"));
  const reference = Object.keys(await readLocale("en")).sort();

  for (const name of names) {
    const keys = Object.keys(await readLocale(name)).sort();
    assert.deepEqual(keys, reference, `${name} does not match en`);
  }
});

test("every key the UI asks for exists in every locale", async () => {
  const names = await localeNames();
  const used = await usedKeys();
  assert.ok(used.size > 0, "found no tr() calls to check");

  for (const name of names) {
    const messages = await readLocale(name);
    const missing = [...used].filter(key => !(key in messages)).sort();
    assert.deepEqual(missing, [], `${name} is missing keys used by the UI`);
  }
});

test("no locale ships an empty message or a stray placeholder", async () => {
  for (const name of await localeNames()) {
    const messages = await readLocale(name);
    for (const [key, entry] of Object.entries(messages)) {
      assert.equal(typeof entry.message, "string", `${name}/${key} has no message`);
      assert.notEqual(entry.message.trim(), "", `${name}/${key} is empty`);
      const declared = new Set(Object.keys(entry.placeholders || {}));
      for (const match of entry.message.matchAll(/\$([a-zA-Z0-9_]+)\$/g)) {
        assert.ok(declared.has(match[1]), `${name}/${key} uses undeclared $${match[1]}$`);
      }
    }
  }
});
