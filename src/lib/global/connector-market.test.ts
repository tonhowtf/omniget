import { describe, expect, it } from "vitest";
import { authProblem, descriptionFor, errorKey, filterEntries, looksLikeUrl, type MarketEntry } from "./connector-market";

describe("errorKey", () => {
  it("names the failed check", () => {
    expect(errorKey("signature: the catalog signature is not valid")).toBe("global_dl.market.err.signature");
    expect(errorKey("hash: expected a, got b")).toBe("global_dl.market.err.hash");
    expect(errorKey("mismatch: the package says")).toBe("global_dl.market.err.mismatch");
    expect(errorKey("network: https://x: HTTP 404")).toBe("global_dl.market.err.network");
  });

  it("keeps the existing message for sites that opted out", () => {
    expect(errorKey("The owner of this site asked OmniGet not to support it.")).toBe("errors.platform_opted_out");
  });

  it("falls back for anything else", () => {
    expect(errorKey("disk full")).toBe("global_dl.market.err.generic");
  });
});

describe("authProblem", () => {
  it("recognises connector messages that ask to sign in", () => {
    expect(authProblem("resolve: Error: Not signed in to Hotmart. Sign in at consumer.hotmart.com")).toBe(true);
    expect(authProblem("HTTP 403 for https://www.udemy.com/api-2.0/x")).toBe(true);
  });

  it("leaves other failures alone", () => {
    expect(authProblem("HTTP 500 for https://x")).toBe(false);
    expect(authProblem("connection reset")).toBe(false);
  });
});

describe("looksLikeUrl", () => {
  it("accepts links and bare hosts", () => {
    expect(looksLikeUrl("https://hotmart.com/pt-br/club/x")).toBe(true);
    expect(looksLikeUrl("consumer.hotmart.com")).toBe(true);
  });

  it("refuses search text", () => {
    expect(looksLikeUrl("how to cook rice")).toBe(false);
    expect(looksLikeUrl("rice")).toBe(false);
    expect(looksLikeUrl("")).toBe(false);
  });
});

describe("filterEntries", () => {
  const entry = (id: string, state: MarketEntry["state"], extra: Partial<MarketEntry> = {}) =>
    ({ id, name: id, description: "", domains: [`${id}.com`], state, ...extra }) as MarketEntry;
  const list = [
    entry("udemy", "not_installed"),
    entry("bunkr", "installed", { description: "Whole albums" }),
    entry("hotmart", "not_installed", { domains: ["hotmart.com", "pandavideo.com.br"] }),
    entry("branas", "update_available"),
  ];

  it("puts updates and installed helpers first, then the rest by name", () => {
    expect(filterEntries(list, "", "").map((e) => e.id)).toEqual(["branas", "bunkr", "hotmart", "udemy"]);
  });

  it("puts the helper opened from a link first", () => {
    expect(filterEntries(list, "", "udemy").map((e) => e.id)).toEqual(["udemy", "branas", "bunkr", "hotmart"]);
  });

  it("searches the name, the sites and the description, ignoring case", () => {
    expect(filterEntries(list, " PANDA ", "").map((e) => e.id)).toEqual(["hotmart"]);
    expect(filterEntries(list, "albums", "").map((e) => e.id)).toEqual(["bunkr"]);
    expect(filterEntries(list, "nothing", "")).toEqual([]);
  });

  it("searches the shown description and the English one", () => {
    const translated = [entry("udemy", "not_installed", { description: "Courses you are enrolled in", description_i18n: { ru: "Курсы, на которые вы записаны" } })];
    const shownIn = (locale: string) => (e: MarketEntry) => descriptionFor(e, locale);
    expect(filterEntries(translated, "курсы", "", shownIn("ru")).map((e) => e.id)).toEqual(["udemy"]);
    expect(filterEntries(translated, "enrolled", "", shownIn("ru")).map((e) => e.id)).toEqual(["udemy"]);
    expect(filterEntries(translated, "курсы", "", shownIn("pt"))).toEqual([]);
    expect(filterEntries(translated, "курсы", "")).toEqual([]);
  });
});

describe("descriptionFor", () => {
  const entry = (description_i18n?: unknown) => ({ id: "udemy", description: "English text.", description_i18n }) as MarketEntry;

  it("returns the text of the active language", () => {
    expect(descriptionFor(entry({ ru: "Русский текст.", pt: "Texto." }), "ru")).toBe("Русский текст.");
    expect(descriptionFor(entry({ "zh-TW": "繁體" }), "zh-TW")).toBe("繁體");
  });

  it("falls back to the English description", () => {
    expect(descriptionFor(entry({ ru: "Русский текст." }), "de")).toBe("English text.");
    expect(descriptionFor(entry(), "ru")).toBe("English text.");
    expect(descriptionFor(entry(null), "ru")).toBe("English text.");
    expect(descriptionFor(entry({ ru: "" }), "ru")).toBe("English text.");
    expect(descriptionFor(entry({ ru: "  " }), "ru")).toBe("English text.");
  });

  it("does not treat inherited object keys as translations", () => {
    for (const locale of ["constructor", "toString", "__proto__", "hasOwnProperty"]) {
      expect(descriptionFor(entry({ ru: "Русский текст." }), locale)).toBe("English text.");
      expect(descriptionFor(entry({}), locale)).toBe("English text.");
    }
  });

  it("ignores values that are not text", () => {
    expect(descriptionFor(entry({ ru: 7 }), "ru")).toBe("English text.");
    expect(descriptionFor(entry({ ru: { toString: () => "x" } }), "ru")).toBe("English text.");
    expect(descriptionFor(entry("ru"), "ru")).toBe("English text.");
  });
});
