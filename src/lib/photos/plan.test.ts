import { describe, expect, it } from "vitest";
import {
  EMPTY_EDITS,
  cleanCrop,
  dateKey,
  defaultRemovals,
  filterPhotos,
  fitAspect,
  folderLabel,
  groupPhotos,
  isIdentity,
  keepsOne,
  rangeSelect,
  resizeCrop,
  rotate,
  rotateCrop,
  sameEdits,
  savedBytes,
} from "./plan";
import type { Crop, DupGroup, Edits, Photo } from "./types";

const at = (y: number, m: number, d: number, h = 12) => Date.UTC(y, m - 1, d, h) / 1000;

function photo(path: string, time: number, taken = true): Photo {
  const parts = path.split("/");
  return {
    path,
    name: parts[parts.length - 1],
    folder: parts.slice(0, -1).join("/"),
    format: "jpeg",
    size: 1000,
    modified_at: taken ? at(2030, 1, 1) : time,
    taken_at: taken ? time : null,
    width: 100,
    height: 100,
    thumb: null,
    error: null,
    edited: false,
  };
}

const close = (a: Crop, b: Crop) => {
  for (const k of ["x", "y", "w", "h"] as const) expect(a[k]).toBeCloseTo(b[k], 6);
};

describe("grouping", () => {
  const photos = [
    photo("/u/Pictures/a.jpg", at(2024, 7, 15, 23)),
    photo("/u/Pictures/Trip/b.jpg", at(2024, 7, 16, 1)),
    photo("/u/Downloads/c.jpg", at(2023, 12, 31), false),
  ];

  it("uses the wall-clock date, not the viewer's time zone", () => {
    expect(dateKey(at(2024, 7, 15, 23), "day")).toBe("2024-07-15");
    expect(dateKey(at(2024, 7, 15, 23), "month")).toBe("2024-07");
  });

  it("groups by month newest first, falling back to the file date", () => {
    const s = groupPhotos(photos, "month", "en");
    expect(s.map((x) => x.key)).toEqual(["2024-07", "2023-12"]);
    expect(s[0].title).toBe("July 2024");
    expect(s[0].photos.map((p) => p.name)).toEqual(["b.jpg", "a.jpg"]);
    expect(s[1].photos[0].name).toBe("c.jpg");
  });

  it("groups by day with a localized title", () => {
    const s = groupPhotos(photos, "day", "pt");
    expect(s.map((x) => x.key)).toEqual(["2024-07-16", "2024-07-15", "2023-12-31"]);
    expect(s[1].title).toBe("15 de julho de 2024");
  });

  it("groups by folder labelled from the chosen root", () => {
    const s = groupPhotos(photos, "folder", "en", ["/u/Pictures", "/u/Downloads/"]);
    expect(s.map((x) => x.title)).toEqual(["Downloads", "Pictures", "Pictures / Trip"]);
  });

  it("labels folders outside every root by their full path", () => {
    expect(folderLabel("/x/y", ["/u"])).toBe("/x/y");
    expect(folderLabel("/u/Pictures/A/B", ["/u", "/u/Pictures"])).toBe("Pictures / A / B");
    expect(folderLabel("/u/Pictures2", ["/u/Pictures"])).toBe("/u/Pictures2");
  });

  it("filters by name or folder", () => {
    expect(filterPhotos(photos, " trip ").map((p) => p.name)).toEqual(["b.jpg"]);
    expect(filterPhotos(photos, "")).toHaveLength(3);
  });
});

describe("edits", () => {
  it("knows when nothing changed", () => {
    expect(isIdentity(EMPTY_EDITS)).toBe(true);
    expect(isIdentity({ ...EMPTY_EDITS, contrast: 1 })).toBe(false);
    expect(sameEdits(EMPTY_EDITS, { ...EMPTY_EDITS, crop: null })).toBe(true);
    expect(sameEdits({ ...EMPTY_EDITS, crop: { x: 0, y: 0, w: 0.5, h: 0.5 } }, { ...EMPTY_EDITS, crop: { x: 0, y: 0, w: 0.5, h: 0.5 } })).toBe(true);
  });

  it("rotates the crop with the photo", () => {
    const c = { x: 0.1, y: 0.2, w: 0.3, h: 0.4 };
    close(rotateCrop(c, true), { x: 0.4, y: 0.1, w: 0.4, h: 0.3 });
    close(rotateCrop(rotateCrop(c, true), false), c);
    let e: Edits = { ...EMPTY_EDITS, crop: c };
    for (let i = 0; i < 4; i++) e = rotate(e, true);
    expect(e.rotate).toBe(0);
    close(e.crop!, c);
    expect(rotate(EMPTY_EDITS, false).rotate).toBe(270);
  });

  it("fits an aspect ratio inside the crop, centered", () => {
    const full = { x: 0, y: 0, w: 1, h: 1 };
    close(fitAspect(full, 1, 2), { x: 0.25, y: 0, w: 0.5, h: 1 });
    close(fitAspect(full, 16 / 9, 1), { x: 0, y: (1 - 9 / 16) / 2, w: 1, h: 9 / 16 });
  });

  it("resizes from a corner and stays inside the photo", () => {
    const c = { x: 0.2, y: 0.2, w: 0.5, h: 0.5 };
    close(resizeCrop(c, "se", 0.1, 0.1, null, 1), { x: 0.2, y: 0.2, w: 0.6, h: 0.6 });
    close(resizeCrop(c, "nw", -0.5, -0.5, null, 1), { x: 0, y: 0, w: 0.7, h: 0.7 });
    close(resizeCrop(c, "se", -1, -1, null, 1), { x: 0.2, y: 0.2, w: 0.05, h: 0.05 });
    close(resizeCrop(c, "move", 0.9, -0.9, null, 1), { x: 0.5, y: 0, w: 0.5, h: 0.5 });
  });

  it("keeps the ratio while resizing", () => {
    const c = { x: 0, y: 0, w: 0.5, h: 0.5 };
    const r = resizeCrop(c, "se", 0.2, 0, 1, 2);
    expect(r.w / r.h).toBeCloseTo(0.5, 6);
    const capped = resizeCrop({ x: 0, y: 0.5, w: 0.2, h: 0.2 }, "se", 0.8, 0, 1, 1);
    close(capped, { x: 0, y: 0.5, w: 0.5, h: 0.5 });
  });

  it("drops a crop that covers the whole photo", () => {
    expect(cleanCrop({ x: 0, y: 0, w: 1, h: 1 })).toBeNull();
    expect(cleanCrop({ x: 0.123456, y: 0, w: 0.5, h: 1 })).toEqual({ x: 0.1235, y: 0, w: 0.5, h: 1 });
  });
});

describe("duplicates", () => {
  const groups: DupGroup[] = [
    { kind: "exact", keep: "/a", paths: ["/a", "/b", "/c"], bytes: 20 },
    { kind: "similar", keep: "/x", paths: ["/x", "/y"], bytes: 5 },
  ];

  it("suggests removing everything except the best copy", () => {
    expect([...defaultRemovals(groups)].sort()).toEqual(["/b", "/c", "/y"]);
  });

  it("adds up the space and warns when a group would be emptied", () => {
    const sizes = new Map([["/a", 10], ["/b", 10], ["/c", 10], ["/y", 5]]);
    const chosen = defaultRemovals(groups);
    expect(savedBytes(groups, chosen, sizes)).toBe(25);
    chosen.add("/a");
    expect(keepsOne(groups[0], chosen)).toBe(false);
    expect(keepsOne(groups[1], chosen)).toBe(true);
  });
});

describe("selection", () => {
  const order = ["a", "b", "c", "d"];
  it("selects a range in either direction", () => {
    expect(rangeSelect(order, "b", "d")).toEqual(["b", "c", "d"]);
    expect(rangeSelect(order, "d", "a")).toEqual(["a", "b", "c", "d"]);
    expect(rangeSelect(order, null, "c")).toEqual(["c"]);
    expect(rangeSelect(order, "a", "z")).toEqual([]);
  });
});
