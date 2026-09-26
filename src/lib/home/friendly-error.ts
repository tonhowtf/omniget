export type ErrorKind =
  | "login"
  | "age"
  | "geo"
  | "removed"
  | "rate"
  | "network"
  | "unsupported"
  | "ffmpeg"
  | "generic";

// First match wins, so the specific causes come before the broad ones
// ("Private video" also says "unavailable" on some extractors).
const RULES: [ErrorKind, RegExp][] = [
  ["age", /age[- ]restricted|confirm your age|inappropriate for some users/i],
  ["login", /private video|sign in|log ?in required|login required|members[- ]only|cookies/i],
  ["geo", /available in your country|geo[- ]?restrict|geo[- ]?block|your region/i],
  ["rate", /\b429\b|too many requests|rate[- ]?limit/i],
  ["unsupported", /unsupported url|no suitable extractor/i],
  ["ffmpeg", /ffmpeg|ffprobe|merging formats|postprocess/i],
  ["removed", /video unavailable|has been removed|was deleted|does not exist|\b404\b|no longer available/i],
  ["network", /timed out|timeout|connection (refused|reset|aborted)|network is unreachable|getaddrinfo|name resolution|ssl/i],
];

export function classifyError(raw: string): ErrorKind {
  for (const [kind, re] of RULES) {
    if (re.test(raw)) return kind;
  }
  return "generic";
}
