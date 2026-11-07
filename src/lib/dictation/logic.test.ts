import { describe, expect, it } from "vitest";
import {
  acceptState,
  captureCombo,
  duplicateFinds,
  errorAction,
  errorKey,
  formatBytes,
  formatElapsed,
  insertAt,
  keyFromCode,
  languageLabel,
  nearLimit,
  shortcutCaps,
  shortcutErrorKey,
  splitError,
  type KeyLike,
} from "./logic";
import type { StateEvent } from "./types";

const key = (code: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  key: code,
  code,
  altKey: false,
  ctrlKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("keyFromCode", () => {
  it("maps physical codes to shortcut names", () => {
    expect(keyFromCode("KeyD")).toBe("D");
    expect(keyFromCode("Digit7")).toBe("7");
    expect(keyFromCode("Numpad3")).toBe("3");
    expect(keyFromCode("Space")).toBe("Space");
    expect(keyFromCode("ArrowUp")).toBe("Up");
    expect(keyFromCode("F13")).toBe("F13");
    expect(keyFromCode("F25")).toBeNull();
    expect(keyFromCode("IntlBackslash")).toBeNull();
  });
});

describe("captureCombo", () => {
  it("builds the combo in canonical modifier order", () => {
    expect(captureCombo(key("Space", { altKey: true })).combo).toBe("Alt+Space");
    expect(captureCombo(key("KeyD", { metaKey: true, ctrlKey: true, shiftKey: true })).combo).toBe("Ctrl+Shift+Super+D");
  });

  it("waits while only modifiers are held", () => {
    const c = captureCombo(key("AltLeft", { altKey: true }));
    expect(c.combo).toBeNull();
    expect(c.held).toEqual(["Alt"]);
    expect(c.cancel).toBe(false);
  });

  it("records dead keys by their physical code", () => {
    expect(captureCombo({ ...key("KeyE", { altKey: true }), key: "Dead" }).combo).toBe("Alt+E");
    expect(captureCombo({ ...key("Backquote", { altKey: true }), key: "Dead" }).combo).toBe("Alt+Backquote");
  });

  it("treats a bare Escape as cancel", () => {
    expect(captureCombo(key("Escape")).cancel).toBe(true);
    expect(captureCombo(key("Escape", { ctrlKey: true })).combo).toBe("Ctrl+Escape");
  });
});

describe("shortcutCaps", () => {
  it("uses symbols on macOS and words elsewhere", () => {
    expect(shortcutCaps("Alt+Space", "macos")).toEqual(["⌥", "Space"]);
    expect(shortcutCaps("Ctrl+Shift+Super+Up", "macos")).toEqual(["⌃", "⇧", "⌘", "↑"]);
    expect(shortcutCaps("Ctrl+Alt+Super+Comma", "windows")).toEqual(["Ctrl", "Alt", "Win", ","]);
    expect(shortcutCaps("Super+D", "linux")).toEqual(["Super", "D"]);
    expect(shortcutCaps("  ", "linux")).toEqual([]);
  });
});

describe("errors", () => {
  it("splits code and detail", () => {
    expect(splitError("SAVE_FAILED: disk full")).toEqual({ code: "SAVE_FAILED", detail: "disk full" });
    expect(splitError("TAKEN")).toEqual({ code: "TAKEN", detail: null });
    expect(splitError("something broke")).toEqual({ code: "UNKNOWN", detail: "something broke" });
  });

  it("maps to i18n keys with a fallback", () => {
    expect(errorKey("SILENT")).toBe("dictation.err_silent");
    expect(errorKey("WHATEVER")).toBe("dictation.err_unknown");
    expect(shortcutErrorKey("NEEDS_MODIFIER")).toBe("dictation.sc_needs_modifier");
  });

  it("suggests the fix per platform", () => {
    expect(errorAction("PASTE_ACCESSIBILITY", "macos")).toBe("open_accessibility");
    expect(errorAction("PASTE_ACCESSIBILITY", "windows")).toBeNull();
    expect(errorAction("MODEL_MISSING", "linux")).toBe("download_model");
    expect(errorAction("SILENT", "macos")).toBe("open_microphone");
    expect(errorAction("SILENT", "linux")).toBe("retry");
    expect(errorAction("BUSY", "macos")).toBeNull();
  });
});

describe("time", () => {
  it("formats elapsed time", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(7_900)).toBe("0:07");
    expect(formatElapsed(65_000)).toBe("1:05");
    expect(formatElapsed(-5)).toBe("0:00");
  });

  it("warns in the last ten seconds", () => {
    expect(nearLimit(109_000, 120_000)).toBe(false);
    expect(nearLimit(110_000, 120_000)).toBe(true);
    expect(nearLimit(5_000, 0)).toBe(false);
  });
});

describe("duplicateFinds", () => {
  it("flags repeated finds ignoring case and blanks", () => {
    const d = duplicateFinds([
      { find: "OmniGet", replace: "a" },
      { find: "", replace: "b" },
      { find: " omniget ", replace: "c" },
      { find: "", replace: "d" },
      { find: "other", replace: "e" },
    ]);
    expect([...d].sort()).toEqual([0, 2]);
  });
});

describe("insertAt", () => {
  it("inserts at the caret with a separating space", () => {
    expect(insertAt("Hello", "world", 5, 5)).toEqual({ value: "Hello world", caret: 11 });
    expect(insertAt("", "Hi", 0, 0)).toEqual({ value: "Hi", caret: 2 });
    expect(insertAt("a  b", "x", 2, 2)).toEqual({ value: "a x b", caret: 3 });
    expect(insertAt("replace me", "new", 0, 10)).toEqual({ value: "new", caret: 3 });
  });
});

describe("misc", () => {
  it("accepts only newer sessions", () => {
    const ev = (session: number) => ({ session }) as StateEvent;
    expect(acceptState(null, ev(1))).toBe(true);
    expect(acceptState(ev(3), ev(2))).toBe(false);
    expect(acceptState(ev(3), ev(3))).toBe(true);
  });

  it("labels languages", () => {
    expect(languageLabel("pt", "en")).toBe("Portuguese");
    expect(languageLabel("xx-invalid-!!", "en")).toBe("xx-invalid-!!");
  });

  it("formats bytes", () => {
    expect(formatBytes(80_000_000)).toBe("80 MB");
    expect(formatBytes(1_500_000_000)).toBe("1.5 GB");
    expect(formatBytes(512)).toBe("512 B");
  });
});
