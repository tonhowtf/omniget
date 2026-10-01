export type PermissionRuleAction = "allow" | "ask" | "deny";
type PermissionRuleDraft = {
  tool: string;
  pattern: string;
  action: PermissionRuleAction;
};

export function emptyPermissionRuleDraft(): PermissionRuleDraft {
  return { tool: "shell_exec", pattern: "", action: "ask" };
}

export function normalizePermissionRule(draft: PermissionRuleDraft): PermissionRuleDraft | null {
  const tool = draft.tool.trim();
  const pattern = draft.pattern.trim();
  return tool && pattern ? { ...draft, tool, pattern } : null;
}
