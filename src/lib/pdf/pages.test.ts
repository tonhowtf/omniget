import { describe, expect, it } from "vitest";
import {
  A4,
  everyN,
  insertBlank,
  isUntouched,
  joinPath,
  maybeSamePath,
  moveItems,
  pagesOf,
  parseRanges,
  pdfName,
  plural,
  removeItems,
  rotateItems,
  samePath,
  selectRange,
  sliceRange,
  sourcesOf,
  stem,
  toSpecs,
  type PageItem,
} from "./pages";

function order(items: PageItem[]): string[] {
  return items.map((i) => (i.kind === "page" ? `${i.path === "/a.pdf" ? "a" : "b"}${i.page + 1}` : "blank"));
}

describe("page list edits", () => {
  const a = () => pagesOf("/a.pdf", 5);

  it("moves a selection as a block to the drop position", () => {
    const items = a();
    const keys = new Set([items[0].key, items[2].key]);
    expect(order(moveItems(items, keys, 4))).toEqual(["a2", "a4", "a1", "a3", "a5"]);
    expect(order(moveItems(items, keys, 99))).toEqual(["a2", "a4", "a5", "a1", "a3"]);
    expect(order(moveItems(items, new Set([items[4].key]), 0))).toEqual(["a5", "a1", "a2", "a3", "a4"]);
  });

  it("rotates in quarter turns and swaps blank sizes", () => {
    let items = insertBlank(a(), 1, A4);
    const keys = new Set([items[0].key, items[1].key]);
    items = rotateItems(items, keys, -90);
    expect(items[0]).toMatchObject({ rotate: 270 });
    expect(items[1]).toMatchObject({ width: A4.height, height: A4.width });
    items = rotateItems(items, keys, 450);
    expect(items[0]).toMatchObject({ rotate: 0 });
  });

  it("removes and inserts without losing order", () => {
    const items = a();
    const left = removeItems(items, new Set([items[1].key, items[3].key]));
    expect(order(left)).toEqual(["a1", "a3", "a5"]);
    expect(order(insertBlank(left, 99, A4))).toEqual(["a1", "a3", "a5", "blank"]);
  });

  it("selects ranges in either direction", () => {
    const items = a();
    expect(selectRange(items, items[3].key, items[1].key)).toEqual(new Set([items[1].key, items[2].key, items[3].key]));
  });

  it("knows when nothing changed", () => {
    expect(isUntouched(a(), "/a.pdf", 5)).toBe(true);
    const items = a();
    expect(isUntouched(rotateItems(items, new Set([items[0].key]), 90), "/a.pdf", 5)).toBe(false);
    expect(isUntouched(items.slice(1), "/a.pdf", 5)).toBe(false);
  });
});

describe("specs for the backend", () => {
  it("indexes sources in first-use order", () => {
    const items = [...pagesOf("/b.pdf", 1), ...insertBlank(pagesOf("/a.pdf", 2), 1, { width: 100, height: 200 })];
    const sources = sourcesOf(items);
    expect(sources).toEqual(["/b.pdf", "/a.pdf"]);
    expect(toSpecs(rotateItems(items, new Set([items[0].key]), 90), sources)).toEqual([
      { kind: "page", source: 0, page: 0, rotate: 90 },
      { kind: "page", source: 1, page: 0, rotate: 0 },
      { kind: "blank", width: 100, height: 200 },
      { kind: "page", source: 1, page: 1, rotate: 0 },
    ]);
  });
});

describe("ranges", () => {
  it("accepts the ways people type page ranges", () => {
    expect(parseRanges("1-3, 5; 8-", 10)).toEqual({ ok: true, ranges: [[1, 3], [5, 5], [8, 10]] });
    expect(parseRanges(" -2 ,4 – 6", 10)).toEqual({ ok: true, ranges: [[1, 2], [4, 6]] });
  });

  it("explains what is wrong", () => {
    expect(parseRanges("  ", 5)).toEqual({ ok: false, error: "empty" });
    expect(parseRanges("1-2-3", 5)).toEqual({ ok: false, error: "syntax" });
    expect(parseRanges("abc", 5)).toEqual({ ok: false, error: "syntax" });
    expect(parseRanges("-", 5)).toEqual({ ok: false, error: "syntax" });
    expect(parseRanges("4-9", 5)).toEqual({ ok: false, error: "bounds" });
    expect(parseRanges("0", 5)).toEqual({ ok: false, error: "bounds" });
    expect(parseRanges("4-2", 5)).toEqual({ ok: false, error: "order" });
  });

  it("splits every n pages", () => {
    expect(everyN(7, 3)).toEqual([[1, 3], [4, 6], [7, 7]]);
    expect(everyN(2, 0)).toEqual([[1, 1], [2, 2]]);
    const items = pagesOf("/a.pdf", 7);
    expect(order(sliceRange(items, [4, 6]))).toEqual(["a4", "a5", "a6"]);
  });
});

describe("file names", () => {
  it("builds safe pdf names next to the source", () => {
    expect(stem("/books/My.Book.pdf")).toBe("My.Book");
    expect(pdfName('Report: "final"?')).toBe("Report final.pdf");
    expect(pdfName("notes.PDF")).toBe("notes.PDF");
    expect(pdfName("...")).toBe("document.pdf");
    expect(joinPath("/books", "x.pdf")).toBe("/books/x.pdf");
    expect(joinPath("C:\\Docs\\", "x.pdf")).toBe("C:\\Docs\\x.pdf");
  });
});

describe("plural", () => {
  it("picks the _one key for a single item", () => {
    const tr = (key: string, vars?: Record<string, unknown>) => `${key}:${vars?.count}`;
    expect(plural(tr, "pdf.page_count", 1)).toBe("pdf.page_count_one:1");
    expect(plural(tr, "pdf.page_count", 3)).toBe("pdf.page_count:3");
  });
});

describe("path comparison", () => {
  it("treats the two Unicode spellings of an accented name as the same file", () => {
    const composed = "/Users/ana/Apostila de Matem\u00e1tica.pdf";
    const decomposed = "/Users/ana/Apostila de Matema\u0301tica.pdf";
    expect(samePath(composed, decomposed)).toBe(true);
    expect(samePath("C:\\Docs\\a.pdf", "C:/Docs/a.pdf")).toBe(true);
    expect(samePath("/a/Book.pdf", "/a/book.pdf")).toBe(false);
  });

  it("errs on the side of asking before replacing when only the case differs", () => {
    expect(maybeSamePath("/a/Book.pdf", "/a/book.PDF")).toBe(true);
    expect(maybeSamePath("/a/book.pdf", "/b/book.pdf")).toBe(false);
  });
});
