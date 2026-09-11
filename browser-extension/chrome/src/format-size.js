// One byte formatter shared by the sniffer (which labels a detected file) and
// the popup (which labels an estimated HLS total), so the two never disagree
// about how a size reads.
export function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return "";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
