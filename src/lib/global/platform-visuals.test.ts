import { describe, expect, it } from "vitest";
import {
  collectDownloadable,
  countDrm,
  countModules,
  findThumbnail,
  isCourseLibrary,
  platformVisual,
  type GdTreeNode,
} from "./platform-visuals";

function leaf(id: string, opts: { drm?: boolean; format?: string } = {}): GdTreeNode {
  return {
    id,
    title: id,
    kind: "video",
    drm: opts.drm,
    source: opts.drm ? undefined : { format: opts.format ?? "hls", url: `https://example.com/${id}` },
  };
}

describe("platformVisual", () => {
  it("returns known brand colors for course platforms", () => {
    expect(platformVisual("hotmart").color).toBe("#F04E23");
    expect(platformVisual("Udemy").color).toBe("#A435F0");
    expect(platformVisual("branas").color).toBe("#0EA5E9");
  });

  it("returns a deterministic fallback for unknown platforms", () => {
    const a = platformVisual("my-custom-academy");
    const b = platformVisual("my-custom-academy");
    expect(a.color).toBe(b.color);
    expect(a.gradient).toContain(a.color);
    expect(a.iconPaths.length).toBeGreaterThan(10);
  });
});

describe("tree helpers", () => {
  it("counts downloadable and drm leaves recursively", () => {
    const tree: GdTreeNode = {
      id: "root",
      title: "Course",
      kind: "folder",
      children: [
        { id: "m1", title: "Module 1", kind: "folder", children: [leaf("l1"), leaf("l2", { drm: true })] },
        { id: "m2", title: "Module 2", kind: "folder", children: [leaf("l3")] },
      ],
    };
    expect(collectDownloadable(tree).map((n) => n.id)).toEqual(["l1", "l3"]);
    expect(countDrm(tree)).toBe(1);
    expect(countModules(tree)).toBe(2);
  });

  it("detects a multi-course library when courses contain modules or thumbnails", () => {
    const library: GdTreeNode = {
      id: "lib",
      title: "Hotmart",
      kind: "folder",
      children: [
        {
          id: "c1",
          title: "Course 1",
          kind: "folder",
          thumbnail: "https://cdn.example.com/c1.jpg",
          children: [{ id: "c1-m1", title: "Intro", kind: "folder", children: [leaf("c1-l1")] }],
        },
        {
          id: "c2",
          title: "Course 2",
          kind: "folder",
          children: [{ id: "c2-m1", title: "Start", kind: "folder", children: [leaf("c2-l1")] }],
        },
      ],
    };
    expect(isCourseLibrary(library)).toBe(true);
    expect(findThumbnail(library)).toBe("https://cdn.example.com/c1.jpg");
  });

  it("treats a single course with flat module folders as curriculum, not a library", () => {
    const singleCourse: GdTreeNode = {
      id: "course",
      title: "TypeScript Mastery",
      kind: "folder",
      children: [
        { id: "m1", title: "Module 1", kind: "folder", children: [leaf("l1"), leaf("l2")] },
        { id: "m2", title: "Module 2", kind: "folder", children: [leaf("l3")] },
      ],
    };
    expect(isCourseLibrary(singleCourse)).toBe(false);
  });
});
