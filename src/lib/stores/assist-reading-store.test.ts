import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type Store = typeof import("./assist-reading-store.svelte");
let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./assist-reading-store.svelte");
});

afterEach(() => {
  store.resetReadingStore();
  invoke.mockReset();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("pure helpers", () => {
  it("maps backend codes to i18n keys and keeps the detail", () => {
    expect(store.readingErrorKey("ERR_READING_INPUT: a page must be a number")).toBe("assist.reading.err_input");
    expect(store.readingErrorKey("ERR_READING_SCOPE: nope")).toBe("assist.reading.err_scope");
    expect(store.readingErrorKey(new Error("boom"))).toBe("assist.reading.err_generic");
    expect(store.errorDetail("ERR_READING_INPUT: a page must be a number")).toBe("a page must be a number");
  });

  it("renders only http(s) links", () => {
    expect(store.safeHref("https://www.netflix.com/br/title/1")).toBe("https://www.netflix.com/br/title/1");
    expect(store.safeHref("javascript:alert(1)")).toBeNull();
    expect(store.safeHref("file:///etc/passwd")).toBeNull();
    expect(store.safeHref(null)).toBeNull();
  });

  it("splits lists and formats progress", () => {
    expect(store.splitList("Netflix, Max;\n Prime Video ,")).toEqual(["Netflix", "Max", "Prime Video"]);
    expect(store.progressValue({ position_kind: "percent", value: "42" })).toBe("42%");
    expect(store.progressValue({ position_kind: "chapter", value: "3" })).toBe("3");
  });

  it("reads the Study reader's list shape (0–1 percentage)", () => {
    const books = store.normaliseReaderBooks({
      items: [
        { id: 7, title: "O Livro", author: "A", format: "epub", reading_pct: 0.25, last_opened_at: 1 },
        { id: 8, title: null, file_path: "/x/y/Outro.pdf", reading_pct: 1 },
        { title: "sem id" },
      ],
      total: 3,
    });
    expect(books.map((b) => b.id)).toEqual([7, 8]);
    expect(books[1].title).toBe("Outro.pdf");
    expect(store.readerPercent(books[0].reading_pct)).toBe(25);
    expect(store.readerPercent(1)).toBe(100);
  });
});

describe("commands", () => {
  const overview = { journeys: [], prefs: { bot_id: "b", region: "BR", subscriptions: [], access_kinds: ["subscription"], only_confirmed: false, freshness_days: 7, updated_ms: 0 }, skill: null };

  it("loads the overview of a bot", async () => {
    invoke.mockResolvedValueOnce(overview);
    await store.loadOverview("b");
    expect(invoke).toHaveBeenCalledWith("assist_reading_overview", { botId: "b" });
    expect(store.getOverview("b")?.prefs.region).toBe("BR");
  });

  it("a failed write returns the error key and does not reload", async () => {
    invoke.mockRejectedValueOnce("ERR_READING_INPUT: a percentage must be a number from 0 to 100");
    const out = await store.recordProgress("b", "j1", "percent", "140");
    expect(out.ok).toBe(false);
    if (!out.ok) {
      expect(out.errorKey).toBe("assist.reading.err_input");
      expect(out.detail).toContain("0 to 100");
    }
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("recording a viewing sends the reaction and refreshes", async () => {
    invoke.mockResolvedValueOnce({ id: "e1" }).mockResolvedValueOnce(overview);
    const out = await store.recordViewing("b", "j1", "m1", "watched", "  gostei  ");
    expect(out.ok).toBe(true);
    expect(invoke).toHaveBeenNthCalledWith(1, "assist_reading_record_viewing", { journeyId: "j1", movieId: "m1", kind: "watched", reaction: "gostei" });
    expect(invoke).toHaveBeenNthCalledWith(2, "assist_reading_overview", { botId: "b" });
  });
});
