import { describe, expect, it } from "vitest";
import { authProblem, errorKey, filterEntries, localizedDescription, looksLikeUrl, type MarketEntry } from "./connector-market";

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
});

describe("localizedDescription", () => {
  const entry = (extra: Partial<MarketEntry> = {}) =>
    ({ id: "udemy", name: "Udemy", description: "Courses you are enrolled in", domains: [], state: "not_installed", ...extra }) as MarketEntry;

  it("shows the catalog translation for the active locale", () => {
    const card = entry({ description_i18n: { ru: "Курсы, на которые вы записаны" } });
    expect(localizedDescription(card, "ru")).toBe("Курсы, на которые вы записаны");
  });

  it("falls back to the catalog English when the locale has no translation", () => {
    const card = entry({ description_i18n: { ru: "Курсы" } });
    expect(localizedDescription(card, "ja")).toBe("Courses you are enrolled in");
    expect(localizedDescription(card, null)).toBe("Courses you are enrolled in");
    expect(localizedDescription(entry(), "ru")).toBe("Courses you are enrolled in");
  });

  it("accepts a regional locale for a base-language entry", () => {
    const card = entry({ description_i18n: { pt: "Cursos em que você está inscrito" } });
    expect(localizedDescription(card, "pt-BR")).toBe("Cursos em que você está inscrito");
  });
});

describe("filterEntries with the active description", () => {
  const entry = (id: string, extra: Partial<MarketEntry> = {}) =>
    ({ id, name: id, description: "", domains: [], state: "not_installed", ...extra }) as MarketEntry;
  const list = [
    entry("udemy", { description: "Courses you are enrolled in", description_i18n: { ru: "Курсы, на которые вы записаны" } }),
    entry("branas", { description: "LaunchLab schools", description_i18n: { ru: "Школы LaunchLab" } }),
  ];
  const shown = (e: MarketEntry) => localizedDescription(e, "ru");

  it("matches the text the card shows", () => {
    expect(filterEntries(list, "курсы", "", shown).map((e) => e.id)).toEqual(["udemy"]);
  });

  it("keeps matching the English description while the card is translated", () => {
    expect(filterEntries(list, "enrolled", "", shown).map((e) => e.id)).toEqual(["udemy"]);
  });

  it("matches the translated text once the locale changes", () => {
    const ja = (e: MarketEntry) => localizedDescription(e, "ja");
    expect(filterEntries(list, "курсы", "", ja)).toEqual([]);
    expect(filterEntries(list, "enrolled", "", ja).map((e) => e.id)).toEqual(["udemy"]);
  });
});
