import { describe, expect, it } from "vitest";
import { DEFAULT_RULES, keyLabel, matchKey, mergeRules, parseKey, parseWhen, resolve, whenHolds } from "./shortcuts";

const ev = (key: string, code: string, m: Partial<{ meta: boolean; ctrl: boolean; shift: boolean; alt: boolean }> = {}) => ({
  key,
  code,
  metaKey: !!m.meta,
  ctrlKey: !!m.ctrl,
  shiftKey: !!m.shift,
  altKey: !!m.alt,
});

describe("shortcuts", () => {
  it("parses keys", () => {
    expect(parseKey("mod+shift+[")).toMatchObject({ key: "[", mod: true, shift: true });
    expect(parseKey("mod++")).toMatchObject({ key: "+", mod: true });
    expect(parseKey("esc")?.key).toBe("escape");
    expect(parseKey("mod+a+b")).toBeNull();
  });

  it("matches by physical key when ⌥/⇧ change the character", () => {
    expect(matchKey(parseKey("mod+shift+[")!, ev("{", "BracketLeft", { meta: true, shift: true }), true)).toBe(true);
    expect(matchKey(parseKey("alt+d")!, ev("∂", "KeyD", { alt: true }), true)).toBe(true);
    expect(matchKey(parseKey("mod+k")!, ev("k", "KeyK", { ctrl: true }), true)).toBe(false);
    expect(matchKey(parseKey("mod+k")!, ev("k", "KeyK", { ctrl: true }), false)).toBe(true);
    expect(matchKey(parseKey("?")!, ev("?", "Slash", { shift: true }), true)).toBe(true);
  });

  it("parses and evaluates when", () => {
    expect(parseWhen("a && !(b || c)")).not.toBeNull();
    expect(parseWhen("a &&")).toBeNull();
    expect(parseWhen("(a")).toBeNull();
    expect(whenHolds("a && !(b || c)", { a: true, b: false, c: false })).toBe(true);
    expect(whenHolds("a && !(b || c)", { a: true, c: true })).toBe(false);
    expect(whenHolds("broken &&", { broken: true })).toBe(false);
    expect(whenHolds(undefined, {})).toBe(true);
  });

  it("resolves last rule first and respects when/handlers", () => {
    const k = ev("k", "KeyK", { meta: true });
    expect(resolve(DEFAULT_RULES, k, { isMac: true })?.command).toBe("palette.toggle");
    expect(resolve(DEFAULT_RULES, k, { isMac: true, terminalFocus: true })?.command).toBe("terminal.clear");
    expect(resolve(DEFAULT_RULES, k, { isMac: true }, (c) => c !== "palette.toggle")).toBeNull();
    const merged = mergeRules(DEFAULT_RULES, [{ key: "mod+p", command: "palette.toggle" }]);
    expect(resolve(merged, k, { isMac: true })).toBeNull();
    expect(resolve(merged, ev("p", "KeyP", { meta: true }), { isMac: true })?.command).toBe("palette.toggle");
  });

  it("labels", () => {
    expect(keyLabel("mod+shift+[", true)).toBe("⇧⌘[");
    expect(keyLabel("mod+shift+[", false)).toBe("Ctrl+Shift+[");
    expect(keyLabel("alt+up", true)).toBe("⌥↑");
  });
});
