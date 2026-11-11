import { describe, expect, it } from "vitest";
import { WORKFLOWS, buildArgs, cellText, coerceParams, evidenceOnly, initialValues, missingRequired, sortRows, tablesFor, toMarkdown, visibleFields } from "./workflows";
import type { EndpointParam, Workflow } from "./types";

function workflow(id: string): Workflow {
  return WORKFLOWS.find((w) => w.id === id) as Workflow;
}

describe("workflow forms", () => {
  it("has the four workflows bound to their tools", () => {
    expect(WORKFLOWS.map((w) => [w.id, w.tool])).toEqual([
      ["creators", "research_tiktok_creators"],
      ["brief", "research_tiktok_brief"],
      ["ads", "research_meta_ads"],
      ["reels", "research_reels"],
    ]);
  });

  it("builds typed arguments and leaves empty fields out", () => {
    const fields = workflow("creators").fields;
    const values = { ...initialValues(fields), queries: "home gym, small apartment workout\n\n gym setup ", brief: "  " };
    expect(buildArgs(fields, values)).toEqual({
      queries: ["home gym", "small apartment workout", "gym setup"],
      sample: 10,
      max_creators: 5,
      transcripts_per_creator: 2,
      ai_transcripts: false,
      follow_bio_link: true,
    });
  });

  it("sends only the fields of the chosen operation", () => {
    const fields = workflow("ads").fields;
    const values = { ...initialValues(fields), op: "compare", name: "Nike", query: "stale", page_id: "1", update: "true" };
    expect(visibleFields(fields, "find").map((f) => f.key)).toEqual(["op", "query"]);
    expect(buildArgs(fields, values)).toEqual({ op: "compare", name: "Nike", max_ads: 100, update: true });
  });
});

describe("api explorer parameters", () => {
  const params: EndpointParam[] = [
    { name: "query", in: "query", required: true, type: "string" },
    { name: "trim", in: "query", required: false, type: "boolean" },
    { name: "cursor", in: "query", required: false, type: "number" },
    { name: "sort_by", in: "query", required: false, type: "string", enum: ["relevance", "most-liked"] },
  ];

  it("coerces by declared type and drops blanks", () => {
    expect(coerceParams(params, { query: " gym ", trim: "true", cursor: "10", sort_by: "" })).toEqual({ query: "gym", trim: true, cursor: 10 });
    expect(coerceParams(params, { cursor: "abc" })).toEqual({ cursor: "abc" });
  });

  it("names the required parameters still empty", () => {
    expect(missingRequired(params, { trim: "true" })).toEqual(["query"]);
    expect(missingRequired(params, { query: "x" })).toEqual([]);
  });
});

describe("evidence tables", () => {
  it("shows creators with median beside mean, sample, contact and missing flags", () => {
    const [creators, matched] = tablesFor("creators", {
      creators: [
        {
          handle: "ana",
          profile_url: "https://www.tiktok.com/@ana",
          followers: 1200,
          recent: { sample: 6, confidence: "low", date_from: "2026-10-01T10:00:00Z", date_to: "2026-10-20T10:00:00Z", views: { median: 750, mean: 10630 }, engagement_rate_median: 0.1234 },
          contact: { emails: ["ana@example.com"], links: ["https://ana.fit"], link_in_bio: "https://linktr.ee/ana" },
          missing: ["sample_smaller_than_asked: 6 of 10"],
          matched_videos: [
            { url: "https://www.tiktok.com/@ana/video/1", caption: "Home gym", views: 900, lift_vs_own_median: 1.2, transcript: { available: true, excerpts: ["My home gym is tiny."] } },
            { url: "https://www.tiktok.com/@ana/video/2", caption: "", views: 10, transcript: { available: false, reason: "no_captions" } },
          ],
        },
      ],
    });
    expect(creators.rows[0]).toEqual({
      creator: { href: "https://www.tiktok.com/@ana", label: "@ana" },
      followers: 1200,
      views_median: 750,
      views_mean: 10630,
      sample: 6,
      confidence: "low",
      engagement: "12.3%",
      period: "2026-10-01 → 2026-10-20",
      contact: "ana@example.com, https://linktr.ee/ana",
      missing: ["sample_smaller_than_asked: 6 of 10"],
    });
    expect(matched.rows[0].evidence).toBe("My home gym is tiny.");
    expect(matched.rows[0].video).toEqual({ href: "https://www.tiktok.com/@ana/video/1", label: "Home gym" });
    expect(matched.rows[1].missing).toEqual(["no_transcript: no_captions"]);
    expect(matched.rows[1].lift).toBeNull();
  });

  it("flags videos without transcript and counts claims to check", () => {
    const [videos] = tablesFor("brief", {
      videos: [
        { url: "https://www.tiktok.com/@a/video/1", author: { handle: "a" }, stats: { views: 5, likes: 1 }, hook: "Stop.", flags: { no_transcript: false, needs_checking: [{ sentence: "x" }, { sentence: "y" }] } },
        { url: "https://www.tiktok.com/@a/video/2", author: null, stats: null, hook: null, error: "NOT_FOUND", flags: { no_transcript: true, needs_checking: [] } },
      ],
    });
    expect(videos.rows[0]).toMatchObject({ author: "@a", views: 5, hook: "Stop.", claims: 2, missing: [] });
    expect(videos.rows[1]).toMatchObject({ author: null, views: null, claims: 0, missing: ["no_transcript", "NOT_FOUND"] });
  });

  it("builds only the ad tables present in the result and spells out each change", () => {
    const found = tablesFor("ads", { pages: [{ name: "Nike", page_id: "15087023444", category: "Brand", likes: 3 }] });
    expect(found.map((t) => t.id)).toEqual(["pages"]);
    const compared = tablesFor("ads", {
      new_ads: [{ archive_id: "4", url: "https://www.facebook.com/ads/library?id=4", format: "VIDEO", cta_text: "Shop now", running_days: 3, started: "2026-11-01T00:00:00Z", text: "fresh" }],
      no_longer_listed: [],
      not_seen_in_partial_fetch: [],
      changed: [{ archive_id: "1", url: "https://www.facebook.com/ads/library?id=1", running_days: 9, changes: [{ field: "cta_text", before: "Learn more", after: "Sign up" }, { field: "title", before: null, after: "New" }] }],
    });
    expect(compared.map((t) => t.id)).toEqual(["new_ads", "no_longer_listed", "changed"]);
    expect(compared[0].rows[0]).toEqual({ ad: { href: "https://www.facebook.com/ads/library?id=4", label: "4" }, format: "VIDEO", cta: "Shop now", running_days: 3, started: "2026-11-01", text: "fresh" });
    expect(compared[2].rows[0].changes).toBe("cta_text: Learn more → Sign up; title: ∅ → New");
  });

  it("lists reel profiles with their flags and every compared reel", () => {
    const [profiles, reels] = tablesFor("reels", {
      profiles: [
        { handle: "jane", profile_url: "https://www.instagram.com/jane/", sample: 5, confidence: "low", plays: { median: 200, mean: 1150 }, likes_median: 10, comments_median: 2, cadence: { reels_per_week: 3.5 }, flags: ["sample_smaller_than_asked"], reels: [{ url: "https://www.instagram.com/reel/c5/", caption: "Summer", plays: 5000, likes: 1, comments: 2, posted: "2026-10-01T00:00:00Z", lift_vs_own_median: 25 }] },
        { handle: "ghost", profile_url: "https://www.instagram.com/ghost/", sample: 0, flags: ["unavailable"], error: "NOT_FOUND" },
      ],
    });
    expect(profiles.rows[0]).toMatchObject({ sample: 5, plays_median: 200, plays_mean: 1150, per_week: 3.5, missing: ["sample_smaller_than_asked"] });
    expect(profiles.rows[1]).toMatchObject({ plays_median: null, missing: ["unavailable", "NOT_FOUND"] });
    expect(reels.rows).toHaveLength(1);
    expect(reels.rows[0]).toMatchObject({ profile: "@jane", plays: 5000, posted: "2026-10-01", lift: 25 });
  });
});

describe("sorting and copy", () => {
  const list = [
    { name: "b", views: 10, flags: ["x"] },
    { name: "a", views: null, flags: [] },
    { name: { href: "https://x", label: "C" }, views: 300, flags: ["x", "y"] },
  ];

  it("sorts numbers numerically, text by label and keeps empty cells last", () => {
    expect(sortRows(list, "views", true).map((r) => r.views)).toEqual([300, 10, null]);
    expect(sortRows(list, "views", false).map((r) => r.views)).toEqual([10, 300, null]);
    expect(sortRows(list, "name", false).map((r) => cellText(r.name))).toEqual(["a", "b", "[C](https://x)"]);
    expect(sortRows(list, "flags", true).map((r) => (r.flags as string[]).length)).toEqual([2, 1, 0]);
    expect(list[0].name).toBe("b");
  });

  it("copies the guide, caveats, tables and data as Markdown", () => {
    const text = toMarkdown({
      title: "Reels",
      guide: "Write it from the evidence.",
      caveats: ["Instagram plays only."],
      tables: [
        { title: "Profiles", columns: ["Profile", "Median | plays"], rows: [[{ href: "https://www.instagram.com/jane/", label: "@jane" }, 200], ["line\nbreak", null]] },
        { title: "Reels", columns: ["Reel"], rows: [] },
      ],
      data: { a: 1 },
      labels: { guide: "How to write it", caveats: "Caveats", data: "Data", cut: "Data cut.", empty: "Nothing." },
    });
    expect(text).toContain("# Reels\n\n## How to write it\n\nWrite it from the evidence.\n\n## Caveats\n\n- Instagram plays only.");
    expect(text).toContain("| Profile | Median \\| plays |\n|---|---|\n| [@jane](https://www.instagram.com/jane/) | 200 |\n| line break | — |");
    expect(text).toContain("## Reels\n\nNothing.");
    expect(text).toContain('## Data\n\n```json\n{\n "a": 1\n}\n```');
    expect(text).not.toContain("Data cut.");
    const cut = toMarkdown({ title: "t", guide: "", caveats: [], tables: [], data: { long: "x".repeat(100) }, labels: { guide: "g", caveats: "c", data: "d", cut: "Data cut.", empty: "e" }, maxDataChars: 20 });
    expect(cut).toContain("Data cut.");
    expect(cut).not.toContain("## g");
  });

  it("keeps the guide and caveats out of the copied data", () => {
    expect(evidenceOnly({ guide: "g", caveats: ["c"], total: 1 })).toEqual({ total: 1 });
  });
});
