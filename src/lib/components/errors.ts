export const ERROR_CODES = [
  "url_empty",
  "url_too_long",
  "url_invalid",
  "url_not_https",
  "url_credentials",
  "url_private",
  "url_not_json",
  "network",
  "timeout",
  "too_large",
  "http_status",
  "not_json",
  "shape",
  "project_missing",
  "css_path_invalid",
  "css_path_outside",
  "css_path_not_css",
  "css_path_missing",
  "css_read_failed",
  "css_too_large",
  "css_changed",
  "css_write_failed",
  "run_program",
  "run_args",
  "run_not_found",
  "run_spawn",
  "run_exit",
  "run_cancelled",
  "tokens_json",
  "tokens_empty",
  "unknown",
] as const;

const KNOWN = new Set<string>(ERROR_CODES);

export function errorKey(code: string): string {
  return `components.err.${KNOWN.has(code) ? code : "unknown"}`;
}
