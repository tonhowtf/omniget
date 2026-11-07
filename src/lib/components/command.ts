import type { BuiltCommand, PackageManager } from "./types";

const LOCK_ORDER: [string, PackageManager][] = [
  ["pnpm-lock.yaml", "pnpm"],
  ["bun.lock", "bun"],
  ["bun.lockb", "bun"],
  ["yarn.lock", "yarn"],
  ["package-lock.json", "npm"],
];

export type Detection = { manager: PackageManager; runner: PackageManager; lockfile: string | null; ambiguous: boolean };

export function detectPackageManager(lockfiles: string[], yarnClassic = false): Detection {
  const present = LOCK_ORDER.filter(([file]) => lockfiles.includes(file));
  if (present.length === 0) return { manager: "npm", runner: "npm", lockfile: null, ambiguous: false };
  const managers = new Set(present.map(([, m]) => m));
  const manager = present[0][1];
  return { manager, runner: manager === "yarn" && yarnClassic ? "npm" : manager, lockfile: present[0][0], ambiguous: managers.size > 1 };
}

export type CommandAction = { kind: "add"; targets: string[]; overwrite?: boolean } | { kind: "init" };

const PREFIX: Record<PackageManager, { program: string; args: string[] }> = {
  npm: { program: "npx", args: [] },
  pnpm: { program: "pnpm", args: ["dlx"] },
  yarn: { program: "yarn", args: ["dlx"] },
  bun: { program: "bunx", args: ["--bun"] },
};

function quote(arg: string): string {
  return /^[A-Za-z0-9@%+=:,./_-]+$/.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`;
}

export function buildCommand(manager: PackageManager, action: CommandAction): BuiltCommand {
  const { program, args: prefix } = PREFIX[manager];
  const args = [...prefix, "shadcn@latest"];
  if (action.kind === "add") {
    const targets = action.targets.filter((t) => /^https:\/\//.test(t));
    if (targets.length === 0) throw new Error("no_targets");
    args.push("add", ...targets, "--yes");
    if (action.overwrite) args.push("--overwrite");
  } else {
    args.push("init", "--yes", "--defaults");
  }
  return { program, args, display: [program, ...args].map(quote).join(" ") };
}

export function stripAnsi(line: string): string {
  return line.replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, "").replace(/\u001b\][^\u0007]*\u0007/g, "").replace(/\r/g, "");
}

export type WrittenFiles = { created: string[]; updated: string[]; skipped: string[] };

export function parseWrittenFiles(lines: string[]): WrittenFiles {
  const out: WrittenFiles = { created: [], updated: [], skipped: [] };
  let section: keyof WrittenFiles | null = null;
  for (const raw of lines) {
    const line = stripAnsi(raw);
    const head = /\b(created|updated|skipped)\s+\d+\s+files?\b/i.exec(line);
    if (head) {
      section = head[1].toLowerCase() as keyof WrittenFiles;
      continue;
    }
    const entry = /^\s+-\s+(.+?)\s*$/.exec(line);
    if (section && entry) {
      if (!out[section].includes(entry[1])) out[section].push(entry[1]);
      continue;
    }
    if (line.trim() && !/^\s/.test(line)) section = null;
  }
  out.updated = out.updated.filter((f) => !out.created.includes(f));
  return out;
}
