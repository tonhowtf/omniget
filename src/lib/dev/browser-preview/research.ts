import type { Operation, PlatformRow, Result } from "$lib/research/types";

type Args = Record<string, unknown>;

function fail(code: string, message: string, nextStep: string, retryable = false): never {
  throw `${code}: ${JSON.stringify({ message, retryable, next_step: nextStep })}`;
}

let configured = false;
let remaining = 4821;
let spent = 0;
let baselines: Result[] = [{ name: "Nike", page_id: "15087023444", company: null, country: "ALL", status: "ACTIVE", complete: true, created_ms: Date.parse("2026-11-03T12:00:00Z"), updated_ms: Date.parse("2026-11-03T12:00:00Z"), ads: 42 }];

const OPERATIONS: Operation[] = [
  {
    id: "tiktok_profile",
    platform: "tiktok",
    group: "TikTok",
    method: "GET",
    path: "/v1/tiktok/profile",
    summary: "Profile: Fetches public profile data for a TikTok user by their handle or user_id.",
    params: [
      { name: "handle", in: "query", required: false, type: "string", description: "TikTok handle", example: "stoolpresidente" },
      { name: "cache_max_age", in: "query", required: false, type: "string", enum: ["1d", "3d", "7d", "14d", "30d"], description: "Return a cached response this recent for 0 credits" },
    ],
    pagination: [],
    credits: null,
  },
  {
    id: "tiktok_search_keyword",
    platform: "tiktok",
    group: "TikTok",
    method: "GET",
    path: "/v1/tiktok/search/keyword",
    summary: "Search by Keyword: Searches for TikTok videos by keyword or phrase.",
    params: [
      { name: "query", in: "query", required: true, type: "string", description: "Keyword to search for" },
      { name: "sort_by", in: "query", required: false, type: "string", enum: ["relevance", "most-liked", "date-posted"], description: "Sort by" },
      { name: "trim", in: "query", required: false, type: "boolean", description: "Set to true to get a trimmed response" },
      { name: "cursor", in: "query", required: false, type: "number", description: "Cursor to get more videos" },
    ],
    pagination: ["cursor"],
    credits: null,
  },
  {
    id: "reddit_search",
    platform: "reddit",
    group: "Reddit",
    method: "GET",
    path: "/v1/reddit/search",
    summary: "Search: Searches across all of Reddit for posts or comments matching a query.",
    params: [
      { name: "query", in: "query", required: true, type: "string", description: "Search query" },
      { name: "sort", in: "query", required: false, type: "string", enum: ["relevance", "new", "top", "comment_count"] },
      { name: "after", in: "query", required: false, type: "string", description: "Token for the next page" },
    ],
    pagination: ["after"],
    credits: null,
  },
];

const PLATFORMS: PlatformRow[] = [
  { platform: "tiktok", name: "TikTok", operations: 2 },
  { platform: "reddit", name: "Reddit", operations: 1 },
];

const GUIDE = "Write the deliverable from this evidence only, with the source URL beside every observation.";

function charge(credits: number): number {
  remaining -= credits;
  spent += credits;
  return credits;
}

function needKey() {
  if (!configured) fail("NO_API_KEY", "no ScrapeCreators API key is saved in OmniGet", "ask the person to open OmniGet, go to Research and paste their ScrapeCreators API key; it cannot be passed through a tool");
}

function ad(id: string, text: string, cta: string, days: number): Result {
  return { archive_id: id, url: `https://www.facebook.com/ads/library?id=${id}`, page_name: "Nike", active: true, started: "2026-10-20T07:00:00Z", ended: null, running_days: days, format: "VIDEO", cta_text: cta, cta_type: "SHOP_NOW", title: "Winter look", text, landing_url: "https://example.com/winter", platforms: ["FACEBOOK", "INSTAGRAM"] };
}

function creators(args: Args): Result {
  const queries = Array.isArray(args.queries) ? (args.queries as string[]) : [];
  if (!queries.length) {
    if (!args.brief) fail("INVALID_PARAMS", "pass queries, or a brief to get suggested queries", "write 2 to 5 short TikTok search queries from the campaign brief and pass them as queries");
    return { creators: [], total: 0, returned: 0, truncated: false, reason: "no_queries", suggested_queries: ["home gym", "home apartment", "gym apartment"], next_step: "nothing was searched and no credit was spent; review or rewrite suggested_queries and call again with queries", credits_used: 0, untrusted: true };
  }
  needKey();
  const row = (handle: string, followers: number, median: number, mean: number, sample: number, missing: string[]): Result => ({
    handle,
    profile_url: `https://www.tiktok.com/@${handle}`,
    name: handle,
    verified: false,
    bio: "Small space workouts. collabs in bio",
    followers,
    matched_queries: queries,
    recent: { sample_asked: 10, sample, confidence: sample < 10 ? "low" : "medium", date_from: "2026-10-12T15:00:00Z", date_to: "2026-11-08T18:30:00Z", views: { sample, median, mean, min: 900, max: mean * 4, confidence: sample < 10 ? "low" : "medium" }, engagement_rate_median: 0.0834 },
    matched_videos: [
      { url: `https://www.tiktok.com/@${handle}/video/7517114944362499342`, caption: "My whole home gym fits under the bed", created: "2026-11-02T10:00:00Z", views: median * 3, likes: 4100, comments: 96, shares: 310, lift_vs_own_median: 3, lift_label: "strong", transcript: { available: true, matched_terms: ["home", "gym"], excerpts: ["This home gym cost me less than three months of a membership."] } },
      { url: `https://www.tiktok.com/@${handle}/video/7499229683859426602`, caption: "5 moves, no equipment", created: "2026-10-21T10:00:00Z", views: median, likes: 800, comments: 12, shares: 40, lift_vs_own_median: 1, lift_label: null, transcript: { available: false, reason: "no_captions" } },
    ],
    contact: { emails: missing.includes("no_public_contact") ? [] : [`hello@${handle}.example`], links: [`https://linktr.ee/${handle}`], link_in_bio: `https://linktr.ee/${handle}`, source: "link_in_bio" },
    missing,
  });
  return {
    queries,
    sample: 10,
    creators: [row("tinyflatfit", 182_400, 48_200, 97_310, 10, [`no_transcript: https://www.tiktok.com/@tinyflatfit/video/7499229683859426602`]), row("garagelifts", 23_900, 6_150, 6_480, 6, ["sample_smaller_than_asked: 6 of 10", "no_public_contact"])],
    total: 7,
    returned: 2,
    truncated: true,
    failed_queries: [],
    credits_used: charge(11),
    caveats: ["Public data only.", "A sample under 10 videos is directional: confidence is low."],
    guide: GUIDE,
    untrusted: true,
  };
}

function brief(): Result {
  needKey();
  return {
    videos: [
      { url: "https://www.tiktok.com/@tinyflatfit/video/7517114944362499342", author: { handle: "tinyflatfit", name: "Tiny Flat Fit" }, created: "2026-11-02T10:00:00Z", caption: "My whole home gym fits under the bed", stats: { views: 144_600, likes: 4100, comments: 96, shares: 310, saves: 2200 }, hook: "Stop paying for a gym you never visit.", transcript: "Stop paying for a gym you never visit. This home gym cost me less than three months of a membership. You burn 400 calories in twenty minutes.", transcript_chars: 141, transcript_truncated: false, flags: { no_transcript: false, needs_checking: [{ sentence: "You burn 400 calories in twenty minutes.", reasons: ["number", "health"] }], checked_text: "transcript" }, error: null },
      { url: "https://www.tiktok.com/@garagelifts/video/7499229683859426602", author: { handle: "garagelifts", name: "Garage Lifts" }, created: "2026-10-21T10:00:00Z", caption: "5 moves, no equipment", stats: { views: 6150, likes: 800, comments: 12, shares: 40, saves: 90 }, hook: null, transcript: null, transcript_chars: null, transcript_truncated: false, flags: { no_transcript: true, needs_checking: [{ sentence: "5 moves, no equipment", reasons: ["number"] }], checked_text: "caption" }, error: null },
    ],
    total: 2,
    returned: 2,
    truncated: false,
    without_transcript: 1,
    credits_used: charge(3),
    caveats: ["Transcripts are TikTok captions and can be wrong.", "A video without a transcript is represented by its caption only."],
    guide: GUIDE,
    untrusted: true,
  };
}

function metaAds(args: Args): Result {
  const op = String(args.op ?? "");
  const caveats = ["The Meta Ad Library shows no spend, no conversions and no targeting.", "An ad that has been running for a long time is not proof that it performs."];
  if (op === "list") return { baselines, total: baselines.length, returned: baselines.length, truncated: false };
  if (!["find", "baseline", "compare"].includes(op)) fail("INVALID_PARAMS", `unknown op "${op}" for research_meta_ads`, "use op=find, baseline, compare or list");
  needKey();
  if (op === "find") {
    if (!args.query) fail("INVALID_PARAMS", "find needs query", "pass the company or brand name as query");
    return { pages: [{ page_id: "15087023444", name: "Nike", category: "Sportswear", likes: 37_497_718, verification: "BLUE_VERIFIED", ig_username: "nike" }, { page_id: "51212153078", name: "Nike Football", category: "Product/service", likes: 41_136_495, verification: "BLUE_VERIFIED", ig_username: "nikefootball" }], total: 2, returned: 2, truncated: false, credits_used: charge(1), untrusted: true };
  }
  if (op === "baseline") {
    if (!args.page_id && !args.company) fail("INVALID_PARAMS", "baseline needs page_id or company", "call op=find with the brand name to get its page_id, then op=baseline");
    const name = String(args.name ?? args.company ?? args.page_id);
    const replaced = baselines.some((b) => b.name === name);
    const row = { name, page_id: args.page_id ?? null, company: args.company ?? null, country: String(args.country ?? "ALL"), status: String(args.status ?? "ACTIVE"), complete: true, created_ms: Date.now(), updated_ms: Date.now(), ads: 3 };
    baselines = [row, ...baselines.filter((b) => b.name !== name)];
    return { baseline: row, replaced, ads: [ad("1212560900568826", "Cold is no excuse.", "Shop now", 22), ad("1212560900568827", "The winter collection is here.", "Learn more", 9), ad("1212560900568828", "Run in the rain.", "Shop now", 3)], total: 3, returned: 3, truncated: false, complete: true, credits_used: charge(1), caveats, guide: GUIDE, untrusted: true };
  }
  const saved = baselines.find((b) => String(b.name).toLowerCase() === String(args.name ?? "").toLowerCase());
  if (!saved) fail("UNKNOWN_BASELINE", `no baseline is named "${String(args.name ?? "")}"`, "call research_meta_ads with op=list to see the saved names, or op=baseline to save one");
  return {
    baseline: saved,
    fetched: 3,
    complete: true,
    new_ads: [ad("1212560900568830", "New season, new pace.", "Shop now", 1)],
    no_longer_listed: [ad("1212560900568827", "The winter collection is here.", "Learn more", 16)],
    not_seen_in_partial_fetch: [],
    changed: [{ archive_id: "1212560900568826", url: "https://www.facebook.com/ads/library?id=1212560900568826", running_days: 29, changes: [{ field: "cta_text", before: "Shop now", after: "Sign up" }, { field: "landing_url", before: "https://example.com/winter", after: "https://example.com/members" }] }],
    totals: { new_ads: 1, no_longer_listed: 1, not_seen_in_partial_fetch: 0, changed: 1, unchanged: 1 },
    truncated: false,
    baseline_updated: args.update === true,
    credits_used: charge(1),
    caveats,
    guide: GUIDE,
    untrusted: true,
  };
}

function reels(args: Args): Result {
  const handles = Array.isArray(args.profiles) ? (args.profiles as string[]) : [];
  if (handles.length < 2) fail("INVALID_PARAMS", "profiles needs 2 to 10 Instagram handles", "pass the handles to compare, without @");
  needKey();
  const profiles = handles.map((handle, i) => {
    const median = 12_000 * (i + 1);
    const reel = (code: string, plays: number, day: number): Result => ({ url: `https://www.instagram.com/reel/${code}/`, posted: `2026-11-${String(day).padStart(2, "0")}T18:00:00Z`, plays, likes: Math.round(plays / 20), comments: Math.round(plays / 400), duration_s: null, caption: "Behind the scenes of the new drop", lift_vs_own_median: Math.round((plays / median) * 100) / 100, lift_label: plays / median >= 2 ? "strong" : null });
    const list = [reel(`DKK${i}a`, median * 4, 9), reel(`DKK${i}b`, median, 6), reel(`DKK${i}c`, Math.round(median / 3), 2)];
    return { handle, profile_url: `https://www.instagram.com/${handle}/`, available: i === 0 ? 12 : 7, sample_asked: 12, sample: 7, confidence: "low", date_from: "2026-10-20T18:00:00Z", date_to: "2026-11-09T18:00:00Z", plays: { sample: 7, median, mean: Math.round(median * 1.7), min: Math.round(median / 3), max: median * 4, confidence: "low" }, likes_median: Math.round(median / 20), comments_median: Math.round(median / 400), cadence: { reels_per_week: 2.3, median_gap_days: 3, span_days: 20 }, top: { ...list[0], detail: { caption_full: "Behind the scenes of the new drop, out Friday.", duration_s: 31.4, plays_detail: median * 4, paid_partnership: false } }, bottom: list[2], reels: list, flags: i === 0 ? [] : ["sample_smaller_than_asked"] };
  });
  return { sample_asked: 12, compared_sample: 7, confidence: "low", profiles, total: profiles.length, returned: profiles.length, truncated: false, flags: profiles.filter((p) => p.flags.length).map((p) => ({ handle: p.handle, flags: p.flags })), credits_used: charge(handles.length * 3), caveats: ["Play counts are Instagram plays only; plays of the same reel cross-posted to Facebook are not included.", "A sample under 10 reels is directional: confidence is low."], guide: GUIDE, untrusted: true };
}

function endpoints(args: Args): Result {
  if (!args.platform && !args.query) return { total: PLATFORMS.length, returned: PLATFORMS.length, truncated: false, operations_total: OPERATIONS.length, platforms: PLATFORMS };
  const found = OPERATIONS.filter((op) => (!args.platform || op.platform === args.platform) && (!args.query || `${op.id} ${op.summary}`.toLowerCase().includes(String(args.query).toLowerCase())));
  return { operations: found, total: found.length, returned: found.length, truncated: false, ...(found.length ? {} : { reason: "no_operation_matched" }) };
}

function call(args: Args): Result {
  const op = OPERATIONS.find((o) => o.id === args.endpoint || o.path === args.endpoint);
  if (!op) fail("UNKNOWN_ENDPOINT", `no operation has the id or path "${String(args.endpoint ?? "")}"`, "call research_endpoints with platform or query to get a valid id");
  needKey();
  const params = (args.params ?? {}) as Args;
  const data = op.id === "tiktok_profile" ? { success: true, user: { uniqueId: params.handle ?? "tiktok", nickname: "Preview", signature: "Preview profile", verified: true }, stats: { followerCount: 91_200_000, videoCount: 1320 } } : { success: true, items: [{ id: "1", title: `Preview result for ${String(params.query ?? "")}`, url: "https://example.com/1" }], [op.pagination[0] ?? "cursor"]: "next-token" };
  const text = JSON.stringify(data, null, 2);
  return { endpoint: op.id, method: op.method, path: op.path, credits_used: charge(1), credits_remaining: remaining, next_page: op.pagination.length ? { [op.pagination[0]]: "next-token" } : null, total_chars: text.length, truncated: false, saved_to: null, data, untrusted: true };
}

function usage(args: Args): Result {
  const out: Result = { credits_remaining: spent ? remaining : null, session_credits_used: spent, total: 0, returned: 0, truncated: false, calls: [], key_configured: configured, balance_source: spent ? "last_response" : null };
  if (args.refresh === true) {
    needKey();
    return { ...out, credits_remaining: remaining, session_credits_used: spent, credit_balance: remaining, balance_source: "refreshed" };
  }
  return out;
}

function tool(name: string, args: Args): Result {
  if (name === "research_endpoints") return endpoints(args);
  if (name === "research_call") return call(args);
  if (name === "research_usage") return usage(args);
  if (name === "research_tiktok_creators") return creators(args);
  if (name === "research_tiktok_brief") return brief();
  if (name === "research_meta_ads") return metaAds(args);
  if (name === "research_reels") return reels(args);
  return fail("UNKNOWN_TOOL", `${name} is not a research tool`, "use one of the research_* tools");
}

export const researchFixtures: Record<string, (args: Args) => unknown> = {
  research_key_status: () => ({ configured }),
  research_key_set: ({ key }) => {
    if (!String(key ?? "").trim()) fail("INVALID_API_KEY", "the key is empty or has spaces", "paste the key exactly as shown in the ScrapeCreators dashboard");
    configured = true;
    return { configured };
  },
  research_key_clear: () => {
    configured = false;
    return { configured };
  },
  research_tool: ({ name, args }) => tool(String(name), (args ?? {}) as Args),
};
