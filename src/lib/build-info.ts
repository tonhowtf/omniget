export const BUILD_INFO = {
  version: __APP_VERSION__,
  commit: __COMMIT_HASH__,
  commitShort: __COMMIT_HASH__.slice(0, 7),
  branch: __GIT_BRANCH__,
  date: __BUILD_DATE__,
};

export function formatBuildInfo(): string {
  const parts = [`v${BUILD_INFO.version}`];
  if (BUILD_INFO.commit && BUILD_INFO.commit !== "unknown") {
    parts.push(BUILD_INFO.commitShort);
  }
  if (BUILD_INFO.branch && BUILD_INFO.branch !== "unknown") {
    parts.push(BUILD_INFO.branch);
  }
  if (BUILD_INFO.date) {
    parts.push(BUILD_INFO.date);
  }
  return parts.join(" · ");
}
