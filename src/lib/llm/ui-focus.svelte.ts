import { untrack } from "svelte";

type LlmFocus = { jobId: string | null; conversationId: string | null };

let focus = $state<LlmFocus>({ jobId: null, conversationId: null });

export function setLlmFocus(next: Partial<LlmFocus>): void {
  untrack(() => {
    const merged = { ...focus, ...next };
    if (merged.jobId === focus.jobId && merged.conversationId === focus.conversationId) return;
    focus = merged;
  });
}

export function getLlmFocus(): LlmFocus {
  return focus;
}

export function focusPayload(
  path: string,
  f: LlmFocus,
): { path: string; jobId: string | null; conversationId: string | null } {
  const clean = path.replace(/\/+$/, "") || "/";
  return {
    path: clean,
    jobId: clean.startsWith("/llm/jobs") ? f.jobId : null,
    conversationId: clean === "/llm" ? f.conversationId : null,
  };
}
