import { describe, expect, it } from "vitest";
import { autonomyOf } from "./autonomy";

const argv = (s: string) => s.split(" ").filter(Boolean);

describe("autonomy chip", () => {
  it("stays silent for the default command", () => {
    expect(autonomyOf(argv("claude"))).toBeNull();
    expect(autonomyOf(argv("codex --model gpt-5"))).toBeNull();
    expect(autonomyOf(argv(""))).toBeNull();
    expect(autonomyOf([])).toBeNull();
  });

  it("flags a command that never asks", () => {
    expect(autonomyOf(argv("claude --dangerously-skip-permissions"))).toEqual({ level: "full", flag: "--dangerously-skip-permissions" });
    expect(autonomyOf(argv("claude --permission-mode bypassPermissions"))?.level).toBe("full");
    expect(autonomyOf(argv("claude --permission-mode=bypassPermissions"))?.level).toBe("full");
    expect(autonomyOf(argv("codex --dangerously-bypass-approvals-and-sandbox"))?.level).toBe("full");
    expect(autonomyOf(argv("codex --yolo"))?.level).toBe("full");
    expect(autonomyOf(argv("gemini --yolo"))?.level).toBe("full");
    expect(autonomyOf(argv("gemini -y"))?.level).toBe("full");
    expect(autonomyOf(argv("codex --ask-for-approval never"))?.level).toBe("full");
  });

  it("flags a command that edits without asking", () => {
    expect(autonomyOf(argv("claude --permission-mode acceptEdits"))).toEqual({ level: "auto_edits", flag: "--permission-mode acceptEdits" });
    expect(autonomyOf(argv("codex --full-auto"))?.level).toBe("auto_edits");
    expect(autonomyOf(argv("gemini --approval-mode auto_edit"))?.level).toBe("auto_edits");
  });

  it("flags a plan-only command", () => {
    expect(autonomyOf(argv("claude --permission-mode plan"))?.level).toBe("plan");
  });

  it("reads the sandbox and config switches of codex", () => {
    expect(autonomyOf(argv("codex --sandbox danger-full-access"))?.level).toBe("full");
    expect(autonomyOf(argv("codex -s danger-full-access"))?.level).toBe("full");
    expect(autonomyOf(argv("codex -c approval_policy=never"))?.level).toBe("full");
    expect(autonomyOf(argv("codex -c sandbox_mode=danger-full-access"))?.level).toBe("full");
    expect(autonomyOf(argv("codex --sandbox read-only"))).toBeNull();
  });

  it("does not take the yes switch of a package runner for the agent's", () => {
    expect(autonomyOf(argv("npx -y @google/gemini-cli"))).toBeNull();
    expect(autonomyOf(argv("npx -y @anthropic-ai/claude-code"))).toBeNull();
    expect(autonomyOf(argv("npx -y @google/gemini-cli --yolo"))?.level).toBe("full");
    expect(autonomyOf(argv("/usr/local/bin/gemini -y"))?.level).toBe("full");
  });

  it("looks inside a shell wrapper", () => {
    expect(autonomyOf(["bash", "-lc", "claude --dangerously-skip-permissions"])?.level).toBe("full");
    expect(autonomyOf(["sh", "-c", "cd repo && codex --full-auto"])?.level).toBe("auto_edits");
  });

  it("accepts a switch written with an explicit true", () => {
    expect(autonomyOf(argv("gemini --yolo=true"))?.level).toBe("full");
    expect(autonomyOf(argv("gemini --yolo=false"))).toBeNull();
  });

  it("the widest signal wins and look-alikes do not count", () => {
    expect(autonomyOf(argv("claude --permission-mode plan --dangerously-skip-permissions"))?.level).toBe("full");
    expect(autonomyOf(["claude", "--append-system-prompt", "never use --yolo"])).toBeNull();
    expect(autonomyOf(argv("myyolo --yolox"))).toBeNull();
  });
});
