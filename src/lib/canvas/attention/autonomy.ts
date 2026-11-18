export type AutonomyLevel = "plan" | "auto_edits" | "full";

export interface Autonomy {
  level: AutonomyLevel;
  flag: string;
}

const RANK: Record<AutonomyLevel, number> = { plan: 0, auto_edits: 1, full: 2 };

const BARE: Record<string, AutonomyLevel> = {
  "--dangerously-skip-permissions": "full",
  "--dangerously-bypass-approvals-and-sandbox": "full",
  "--yolo": "full",
  "--full-auto": "auto_edits",
};

const VALUED: Record<string, Record<string, AutonomyLevel>> = {
  "--permission-mode": { bypassPermissions: "full", acceptEdits: "auto_edits", plan: "plan" },
  "--approval-mode": { yolo: "full", auto_edit: "auto_edits" },
  "--ask-for-approval": { never: "full" },
  "-a": { never: "full" },
  "--sandbox": { "danger-full-access": "full" },
  "-s": { "danger-full-access": "full" },
};

const CONFIG: Record<string, AutonomyLevel> = {
  "approval_policy=never": "full",
  "sandbox_mode=danger-full-access": "full",
};

const SHELLS = new Set(["sh", "bash", "zsh", "fish"]);

function base(word: string): string {
  return word.split(/[\\/]/).pop() ?? word;
}

export function autonomyOf(argv: readonly string[]): Autonomy | null {
  let best: Autonomy | null = null;
  const take = (found: Autonomy | null | undefined) => {
    if (found && (!best || RANK[found.level] > RANK[best.level])) best = found;
  };
  const program = base(argv[0] ?? "");
  for (let i = 1; i < argv.length; i++) {
    const word = argv[i];
    const next = argv[i + 1];
    if (SHELLS.has(program) && /^-[a-z]*c$/.test(word) && next) {
      for (const part of next.split(/&&|;|\|\|/)) take(autonomyOf(part.trim().split(/\s+/).filter(Boolean)));
      continue;
    }
    if (word === "-y" && program === "gemini") {
      take({ level: "full", flag: word });
      continue;
    }
    if ((word === "-c" || word === "--config") && next) {
      const level = CONFIG[next.replace(/["']/g, "")];
      if (level) take({ level, flag: `${word} ${next}` });
      continue;
    }
    const eq = word.indexOf("=");
    const name = eq > 0 ? word.slice(0, eq) : word;
    if (BARE[name]) {
      if (eq < 0 || word.slice(eq + 1) === "true") take({ level: BARE[name], flag: name });
      continue;
    }
    const values = VALUED[name];
    if (!values) continue;
    const value = eq > 0 ? word.slice(eq + 1) : next;
    if (value && values[value]) take({ level: values[value], flag: `${name} ${value}` });
  }
  return best;
}
