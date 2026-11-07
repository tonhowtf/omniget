import { describe, expect, it } from "vitest";
import {
  commonTargets,
  failureFix,
  formatBytes,
  groupByCategory,
  labeler,
  mergePaths,
  planRow,
  routeLabel,
  tally,
  tracesToSvg,
} from "./plan";
import type { Category, Engine, FormatInfo, InspectResult, TargetOption } from "./types";

function fmt(id: string, category: Category): FormatInfo {
  return { id, label: id.toUpperCase(), category, extensions: [id] };
}

function target(from: string, to: string, category: Category, engine: Engine = "native", opts: { lossy?: boolean; missing?: Engine } = {}): TargetOption {
  if (opts.missing) {
    return { format: to, label: to.toUpperCase(), category, route: { steps: [], lossy: false, engines: [] }, missing_tool: opts.missing };
  }
  return {
    format: to,
    label: to.toUpperCase(),
    category,
    route: { steps: [{ from, to, engine, lossy: !!opts.lossy }], lossy: !!opts.lossy, engines: [engine] },
    missing_tool: null,
  };
}

function file(path: string, format: FormatInfo | null, targets: TargetOption[], error: string | null = null): InspectResult {
  return { path, name: path.split("/").pop() ?? path, size_bytes: 10, format, targets, error };
}

const png = file("/a/photo.png", fmt("png", "image"), [
  target("png", "jpeg", "image", "native", { lossy: true }),
  target("png", "webp", "image"),
  target("png", "avif", "image", "imagemagick", { missing: "imagemagick" }),
]);
const jpg = file("/a/shot.jpeg", fmt("jpeg", "image"), [
  target("jpeg", "png", "image"),
  target("jpeg", "webp", "image", "native", { lossy: true }),
  target("jpeg", "avif", "image", "imagemagick", { missing: "imagemagick" }),
]);

describe("commonTargets", () => {
  it("keeps formats reachable by every file and counts files already in that format", () => {
    const common = commonTargets([png, jpg]);
    const ids = common.map((c) => c.format);
    expect(ids).toEqual(["jpeg", "webp", "avif", "png"]);
    expect(common.find((c) => c.format === "jpeg")?.skipped).toBe(1);
    expect(common.find((c) => c.format === "webp")?.lossy).toBe(true);
  });

  it("marks a target disabled with the missing tool", () => {
    const avif = commonTargets([png, jpg]).find((c) => c.format === "avif");
    expect(avif?.available).toBe(false);
    expect(avif?.missingTool).toBe("imagemagick");
  });

  it("drops targets one file cannot reach and ignores unreadable files", () => {
    const json = file("/a/data.json", fmt("json", "data"), [target("json", "yaml", "data")]);
    const broken = file("/a/x.bin", null, [], "Empty file");
    expect(commonTargets([png, json])).toEqual([]);
    expect(commonTargets([json, broken]).map((c) => c.format)).toEqual(["yaml"]);
  });

  it("returns nothing when every file is already the only format", () => {
    const onlySelf = file("/a/b.png", fmt("png", "image"), []);
    expect(commonTargets([onlySelf])).toEqual([]);
  });
});

describe("groupByCategory", () => {
  it("orders groups data, documents, images, audio, video", () => {
    const groups = groupByCategory([
      { category: "video" as Category, id: 1 },
      { category: "data" as Category, id: 2 },
      { category: "image" as Category, id: 3 },
    ]);
    expect(groups.map((g) => g.category)).toEqual(["data", "image", "video"]);
  });
});

describe("planRow", () => {
  it("covers each row state", () => {
    expect(planRow(png, null).kind).toBe("none");
    expect(planRow(png, "png").kind).toBe("skip");
    expect(planRow(png, "jpeg").kind).toBe("convert");
    expect(planRow(png, "avif")).toMatchObject({ kind: "missing", tool: "imagemagick" });
    expect(planRow(png, "mp3").kind).toBe("unsupported");
    expect(planRow(file("/x", null, []), "png").kind).toBe("unknown");
    expect(planRow(file("/x", null, [], "Empty file"), "png").kind).toBe("unreadable");
  });
});

describe("routeLabel", () => {
  const labelOf = labeler([fmt("docx", "document"), { ...fmt("html", "document"), label: "HTML" }, fmt("pdf", "document")]);
  it("formats a direct external route", () => {
    const route = { steps: [{ from: "docx", to: "pdf", engine: "libreoffice" as Engine, lossy: false }], lossy: false, engines: ["libreoffice" as Engine] };
    expect(routeLabel(route, labelOf, "Built-in")).toBe("DOCX → PDF · LibreOffice");
  });
  it("formats a multi-hop route with distinct engines", () => {
    const route = {
      steps: [
        { from: "docx", to: "html", engine: "pandoc" as Engine, lossy: false },
        { from: "html", to: "pdf", engine: "native" as Engine, lossy: true },
      ],
      lossy: true,
      engines: [] as Engine[],
    };
    expect(routeLabel(route, labelOf, "Built-in")).toBe("DOCX → HTML → PDF · Pandoc, Built-in");
  });
  it("falls back to the upper-cased id", () => {
    expect(labeler([])("yaml")).toBe("YAML");
  });
});

describe("failureFix", () => {
  it("detects a missing external tool", () => {
    expect(failureFix("pandoc is not installed. Install it to convert DOCX.")).toEqual({ kind: "missing_tool", tool: "pandoc" });
    expect(failureFix("soffice not found")).toEqual({ kind: "missing_tool", tool: "libreoffice" });
  });
  it("falls back to a generic fix", () => {
    expect(failureFix("The file is corrupted.")).toEqual({ kind: "generic" });
    expect(failureFix(null)).toEqual({ kind: "generic" });
  });
});

describe("small helpers", () => {
  it("formats sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MB");
  });
  it("merges paths without duplicates", () => {
    expect(mergePaths(["/a", "/b"], ["/b", "/c", "/c"])).toEqual(["/a", "/b", "/c"]);
  });
  it("tallies statuses", () => {
    expect(tally(["done", "failed", "done", "skipped", "running"])).toMatchObject({ total: 5, done: 2, failed: 1, skipped: 1, running: 1 });
  });
});

describe("tracesToSvg", () => {
  it("is true only when a picture is traced into an SVG", () => {
    const step = (from: string, to: string) => ({ from, to, engine: "native" as Engine, lossy: true });
    expect(tracesToSvg({ steps: [step("png", "svg")], lossy: true, engines: ["native"] })).toBe(true);
    expect(tracesToSvg({ steps: [step("heic", "png"), step("png", "svg")], lossy: true, engines: ["native"] })).toBe(true);
    expect(tracesToSvg({ steps: [step("dxf", "svg")], lossy: true, engines: ["native"] })).toBe(false);
    expect(tracesToSvg({ steps: [step("svg", "png")], lossy: false, engines: ["native"] })).toBe(false);
  });
});

describe("cad category", () => {
  it("groups drawings after images", () => {
    const groups = groupByCategory([
      { category: "audio" as Category },
      { category: "cad" as Category },
      { category: "image" as Category },
    ]);
    expect(groups.map((g) => g.category)).toEqual(["image", "cad", "audio"]);
  });
});
