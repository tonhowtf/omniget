export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const uaData = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  const raw = uaData?.platform || navigator.platform || "";
  return /mac/i.test(raw);
}

function modKey(): string {
  return isMac() ? "⌘" : "Ctrl";
}

export function shortcut(...keys: string[]): string {
  return isMac() ? `${modKey()}${keys.join("")}` : [modKey(), ...keys].join("+");
}

export type Os = "macos" | "windows" | "linux";

export function currentOs(): Os {
  if (typeof navigator === "undefined") return "linux";
  const uaData = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  const raw = `${uaData?.platform || ""} ${navigator.platform || ""} ${navigator.userAgent || ""}`;
  if (/mac/i.test(raw)) return "macos";
  if (/win/i.test(raw)) return "windows";
  return "linux";
}
