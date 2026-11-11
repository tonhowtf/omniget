import { describe, expect, it } from "vitest";
import {
  budgetMeter,
  buildUtilArgs,
  excludedRows,
  filterPorts,
  guessOp,
  headersToRecord,
  parseBudget,
  parseGlobs,
  pushHistory,
  removedSummary,
  statusTone,
  utilFields,
  utilOutput,
} from "./helpers";
import type { PackPlan, PortListener } from "./types";

describe("guessOp", () => {
  it("recognizes a JWT", () => {
    expect(guessOp(" eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl ")).toBe("jwt_decode");
  });

  it("recognizes JSON objects and arrays", () => {
    expect(guessOp('{"a": 1}')).toBe("json_format");
    expect(guessOp("[1, 2]")).toBe("json_format");
    expect(guessOp("{not json")).toBeNull();
  });

  it("recognizes unix seconds and milliseconds", () => {
    expect(guessOp("1900000000")).toBe("timestamp");
    expect(guessOp("1900000000000")).toBe("timestamp");
    expect(guessOp("19000000001")).toBeNull();
  });

  it("recognizes base64 and leaves plain words alone", () => {
    expect(guessOp("SGVsbG8sIHdvcmxkIQ==")).toBe("base64_decode");
    expect(guessOp("b21uaWdldCBkZXZ0b29scw==")).toBe("base64_decode");
    expect(guessOp("password")).toBeNull();
    expect(guessOp("documentation")).toBeNull();
    expect(guessOp("hello world, this is text")).toBeNull();
    expect(guessOp("")).toBeNull();
  });
});

describe("util forms", () => {
  it("names the inputs each op needs", () => {
    expect(utilFields("uuid")).toEqual([]);
    expect(utilFields("timestamp")).toEqual(["value"]);
    expect(utilFields("hash")).toEqual(["algorithm", "text"]);
    expect(utilFields("regex_test")).toEqual(["pattern", "text"]);
    expect(utilFields("json_query")).toEqual(["query", "text"]);
    expect(utilFields("text_diff")).toEqual(["text", "other"]);
    expect(utilFields("json_format")).toEqual(["text"]);
  });

  it("sends only the arguments of the op", () => {
    const form = { text: "abc", other: "abd", query: "a.b", pattern: "a+", algorithm: "md5" as const, value: " 1900000000 " };
    expect(buildUtilArgs("uuid", form)).toEqual({});
    expect(buildUtilArgs("hash", form)).toEqual({ text: "abc", algorithm: "md5" });
    expect(buildUtilArgs("text_diff", form)).toEqual({ text: "abc", other: "abd" });
    expect(buildUtilArgs("timestamp", form)).toEqual({ value: 1900000000 });
    expect(buildUtilArgs("timestamp", { ...form, value: "2030-03-17T17:46:40Z" })).toEqual({ value: "2030-03-17T17:46:40Z" });
    expect(buildUtilArgs("timestamp", { ...form, value: "  " })).toEqual({});
  });

  it("turns each result into the text to show", () => {
    expect(utilOutput("json_format", { text: "{}", truncated: false, total: 2 })).toBe("{}");
    expect(utilOutput("base64_decode", { text: null, hex: "ff00" })).toBe("ff00");
    expect(utilOutput("hash", { hash: "abc", algorithm: "md5" })).toBe("abc");
    expect(utilOutput("uuid", { uuid: "u-1" })).toBe("u-1");
    expect(utilOutput("timestamp", { unix: 1, unixMs: 1000, iso: "1970-01-01T00:00:01Z" })).toBe("unix    1\nunixMs  1000\niso     1970-01-01T00:00:01Z");
    expect(utilOutput("jwt_decode", { header: { alg: "HS256" }, payload: { sub: "1" }, expiresAt: null, expired: false, verified: false, note: "x" })).toBe(
      JSON.stringify({ header: { alg: "HS256" }, payload: { sub: "1" }, expiresAt: null, expired: false }, null, 2),
    );
    expect(utilOutput("regex_test", { matches: [{ match: "ab", start: 0, end: 2 }, { match: "b", start: 5, end: 6 }], total: 2 })).toBe("0-2\tab\n5-6\tb");
    expect(utilOutput("regex_test", { matches: [], total: 0, reason: "no_match" })).toBe("");
  });
});

describe("pack helpers", () => {
  it("splits glob lists on commas and lines", () => {
    expect(parseGlobs(" src/**, *.md\n\n docs/* ")).toEqual(["src/**", "*.md", "docs/*"]);
    expect(parseGlobs("  ")).toEqual([]);
  });

  it("reads a positive whole budget or none", () => {
    expect(parseBudget("8000")).toBe(8000);
    expect(parseBudget(" 8 000 ")).toBe(8000);
    expect(parseBudget("")).toBeUndefined();
    expect(parseBudget("0")).toBeUndefined();
    expect(parseBudget("abc")).toBeUndefined();
  });

  it("measures the total against the budget", () => {
    expect(budgetMeter(4000, 8000)).toEqual({ ratio: 0.5, over: false });
    expect(budgetMeter(9000, 8000)).toEqual({ ratio: 1, over: true });
    expect(budgetMeter(9000, undefined)).toBeNull();
  });

  it("lists what was left out with the place of each secret and never a value", () => {
    const plan = {
      excluded: [
        { path: ".env", reason: "secret" },
        { path: "pnpm-lock.yaml", reason: "lockfile", summary: "8421 lines" },
        { path: "src/config.ts", reason: "secret" },
      ],
      secrets: [
        { path: ".env", line: null, rule: "env_file" },
        { path: "src/config.ts", line: 12, rule: "api_token" },
        { path: "src/config.ts", line: 40, rule: "private_key" },
      ],
    } as PackPlan;
    expect(excludedRows(plan)).toEqual([
      { where: ".env", reason: "secret", detail: "env_file" },
      { where: "pnpm-lock.yaml", reason: "lockfile", detail: "8421 lines" },
      { where: "src/config.ts:12", reason: "secret", detail: "api_token" },
      { where: "src/config.ts:40", reason: "secret", detail: "private_key" },
    ]);
  });
});

describe("scrape helpers", () => {
  it("summarizes what was removed, biggest first", () => {
    expect(removedSummary({ nav: 1, script: 3, style: 1 })).toBe("3 script, 1 nav, 1 style");
    expect(removedSummary({})).toBe("");
  });
});

describe("http helpers", () => {
  it("builds the header map from the filled rows", () => {
    expect(
      headersToRecord([
        { name: " Accept ", value: "application/json" },
        { name: "", value: "orphan" },
        { name: "X-Empty", value: "" },
      ]),
    ).toEqual({ Accept: "application/json", "X-Empty": "" });
  });

  it("maps a status to a tone", () => {
    expect(statusTone(204)).toBe("success");
    expect(statusTone(301)).toBe("info");
    expect(statusTone(404)).toBe("warning");
    expect(statusTone(503)).toBe("danger");
  });

  it("keeps the newest ten requests", () => {
    let history: number[] = [];
    for (let i = 1; i <= 12; i++) history = pushHistory(history, i);
    expect(history).toEqual([12, 11, 10, 9, 8, 7, 6, 5, 4, 3]);
  });
});

describe("filterPorts", () => {
  const listeners = [
    { port: 1420, pid: 48211, process: "node", command: "node vite dev" },
    { port: 5432, pid: 912, process: "postgres", command: null },
  ] as PortListener[];

  it("matches port, pid, process and command", () => {
    expect(filterPorts(listeners, "")).toHaveLength(2);
    expect(filterPorts(listeners, "54").map((l) => l.port)).toEqual([5432]);
    expect(filterPorts(listeners, "VITE").map((l) => l.port)).toEqual([1420]);
    expect(filterPorts(listeners, "912").map((l) => l.port)).toEqual([5432]);
    expect(filterPorts(listeners, "redis")).toEqual([]);
  });
});
