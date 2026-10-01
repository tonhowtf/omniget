/// privacy.sexy link is also an AGPL-3.0 obligation (its output can be
export type Credit = { name: string; license: string; url: string; use: "data" | "design" | "tool" };

export const WMB_CREDITS: readonly Credit[] = [
  { name: "privacy.sexy", license: "AGPL-3.0", url: "https://github.com/undergroundwires/privacy.sexy", use: "data" },
  { name: "BleachBit (CleanerML)", license: "GPL-3.0", url: "https://github.com/bleachbit/bleachbit", use: "data" },
  { name: "Mole", license: "GPL-3.0", url: "https://github.com/tw93/Mole", use: "data" },
  { name: "Kudu", license: "MIT", url: "https://github.com/AdventDevInc/kudu", use: "data" },
  { name: "PureMac", license: "MIT", url: "https://github.com/momenbasel/PureMac", use: "data" },
  { name: "Win11Debloat", license: "MIT", url: "https://github.com/Raphire/Win11Debloat", use: "data" },
  { name: "winutil", license: "MIT", url: "https://github.com/ChrisTitusTech/winutil", use: "data" },
  { name: "Sophia Script", license: "MIT", url: "https://github.com/farag2/Sophia-Script-for-Windows", use: "data" },
  { name: "kondo", license: "MIT", url: "https://github.com/tbillington/kondo", use: "data" },
  { name: "Stats (SMC keys)", license: "MIT", url: "https://github.com/exelban/stats", use: "data" },
  { name: "Bulk Crap Uninstaller", license: "Apache-2.0", url: "https://github.com/Klocman/Bulk-Crap-Uninstaller", use: "design" },
  { name: "dust", license: "Apache-2.0", url: "https://github.com/bootandy/dust", use: "design" },
  { name: "czkawka", license: "MIT", url: "https://github.com/qarmin/czkawka", use: "design" },
  { name: "CapFrameX", license: "MIT", url: "https://github.com/CXWorld/CapFrameX", use: "design" },
  { name: "PresentMon", license: "MIT", url: "https://github.com/GameTechDev/PresentMon", use: "tool" },
  { name: "MangoHud", license: "MIT", url: "https://github.com/flightlessmango/MangoHud", use: "tool" },
  { name: "gamemode", license: "BSD-3-Clause", url: "https://github.com/FeralInteractive/gamemode", use: "tool" },
];
