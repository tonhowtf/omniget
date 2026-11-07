export function formatTimecode(seconds: number, withMs = true): string {
  const safe = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
  const totalMs = Math.round(safe * 1000);
  const h = Math.floor(totalMs / 3_600_000);
  const m = Math.floor(totalMs / 60_000) % 60;
  const s = Math.floor(totalMs / 1000) % 60;
  const ms = totalMs % 1000;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  const base = h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  return withMs ? `${base}.${String(ms).padStart(3, "0")}` : base;
}

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor(total / 60) % 60;
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${String(m).padStart(2, "0")}:${ss}`;
}

export function parseTimecode(text: string): number | null {
  const raw = text.trim().replace(",", ".");
  if (!raw) return null;
  if (/^\d+(\.\d+)?$/.test(raw)) return Number(raw);
  const parts = raw.split(":");
  if (parts.length < 2 || parts.length > 3) return null;
  const last = parts[parts.length - 1];
  if (!/^\d{1,2}(\.\d+)?$/.test(last)) return null;
  const head = parts.slice(0, -1);
  if (!head.every((p) => /^\d+$/.test(p))) return null;
  const secs = Number(last);
  if (secs >= 60) return null;
  const nums = head.map(Number);
  if (nums.length === 2 && nums[1] >= 60) return null;
  const [a, b] = nums;
  const total = nums.length === 2 ? a * 3600 + b * 60 + secs : a * 60 + secs;
  return Number.isFinite(total) ? total : null;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value >= 100 || i === 0 ? Math.round(value) : value.toFixed(1)} ${units[i]}`;
}
