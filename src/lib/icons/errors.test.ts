import { describe, expect, it } from "vitest";
import { errorKey, errorOf } from "./errors";

describe("errors", () => {
  it("keeps the code and params sent by Rust", () => {
    const e = errorOf({ kind: "http", code: "status", params: { status: 404 }, message: "The icon server answered with error 404." });
    expect(e).toEqual({ kind: "http", code: "status", params: { status: "404" }, message: "The icon server answered with error 404." });
    expect(errorKey(e)).toEqual({ key: "icons.err.status", params: { status: "404" } });
  });

  it("falls back by kind for unknown codes", () => {
    expect(errorKey(errorOf({ kind: "offline", code: "new_thing", message: "" })).key).toBe("icons.err.unreachable");
    expect(errorKey(errorOf({ kind: "io", message: "x" })).key).toBe("icons.err.io");
  });

  it("wraps plain errors as unknown", () => {
    expect(errorOf(new Error("boom"))).toEqual({ kind: "io", code: "unknown", params: {}, message: "boom" });
    expect(errorKey(errorOf("text")).key).toBe("icons.err.unknown");
  });
});
