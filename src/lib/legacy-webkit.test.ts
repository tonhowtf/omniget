import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import vm from "node:vm";
import { webcrypto } from "node:crypto";

const code = readFileSync("static/legacy-webkit.js", "utf8");

function oldWebKit() {
  const appended: { textContent: string }[] = [];
  const window: Record<string, unknown> = {
    Promise,
    Object,
    Array,
    String,
    Math,
    Map,
    Set,
    Date,
    RegExp,
    ArrayBuffer,
    Uint8Array,
    Int8Array,
    crypto: { getRandomValues: (b: Uint8Array) => webcrypto.getRandomValues(b) },
    HTMLElement: class {},
    Event: class {},
    document: {
      createElement: () => ({ textContent: "" }),
      head: { appendChild: (el: { textContent: string }) => appended.push(el) },
    },
  };
  return { window, appended };
}

describe("legacy webkit shim", () => {
  it("gives old WebKit a dialog that hides until it is opened", () => {
    const { window, appended } = oldWebKit();
    vm.runInNewContext(code, { window, Object, Array, String, Math, Map, Set, Date, RegExp, ArrayBuffer, Uint8Array, Promise, Event: window.Event });
    const proto = (window.HTMLElement as { prototype: Record<string, unknown> }).prototype;
    expect(typeof proto.showModal).toBe("function");
    expect(typeof proto.close).toBe("function");
    expect(appended[0].textContent).toContain("dialog:not([open]){display:none");
  });

  it("adds a working crypto.randomUUID and structuredClone", () => {
    const { window } = oldWebKit();
    vm.runInNewContext(code, { window, Object, Array, String, Math, Map, Set, Date, RegExp, ArrayBuffer, Uint8Array, Promise, Event: window.Event });
    const uuid = (window.crypto as { randomUUID: () => string }).randomUUID();
    expect(uuid).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    const src = { a: [1, { b: 2 }], when: new Date(5) };
    const copy = (window.structuredClone as (v: typeof src) => typeof src)(src);
    expect(copy).toEqual(src);
    expect(copy.a).not.toBe(src.a);
  });

  it("leaves a modern engine alone", () => {
    const { window, appended } = oldWebKit();
    const modern = { ...window, document: { createElement: () => ({ showModal() {} }), head: (window.document as { head: unknown }).head } };
    vm.runInNewContext(code, { window: modern, Object, Array, String, Math, Map, Set, Date, RegExp, ArrayBuffer, Uint8Array, Promise, Event: window.Event });
    expect(appended).toHaveLength(0);
  });
});
