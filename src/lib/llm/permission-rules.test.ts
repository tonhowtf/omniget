import { describe, expect, it } from "vitest";
import { emptyPermissionRuleDraft, normalizePermissionRule } from "./permission-rules";

describe("permission rule drafts", () => {
  it("starts with ask and no scope", () => {
    expect(emptyPermissionRuleDraft()).toEqual({ tool: "shell_exec", pattern: "", action: "ask" });
  });

  it("refuses an implicit wildcard for an empty scope", () => {
    expect(normalizePermissionRule({ tool: "shell_exec", pattern: " ", action: "allow" })).toBeNull();
  });

  it("keeps an explicitly entered wildcard visible in the normalized rule", () => {
    expect(normalizePermissionRule({ tool: " shell_exec ", pattern: " * ", action: "deny" })).toEqual({
      tool: "shell_exec",
      pattern: "*",
      action: "deny",
    });
  });
});
