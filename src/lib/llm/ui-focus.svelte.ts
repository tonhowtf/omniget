import { untrack } from "svelte";

/**
 * What the `/llm` pages show right now, for the backend's notification gate
 * (`llm_attention.rs`): a "needs you" about a job whose row is open, or whose
 * conversation is on screen, is not sent while the window has the focus.
 *
 * Pages set their part (`setLlmFocus`); the root layout sends it together
 * with the route through `llm_ui_focus` whenever either changes, so the order
 * in which page and layout effects run never matters.
 */
type LlmFocus = { jobId: string | null; conversationId: string | null };

let focus = $state<LlmFocus>({ jobId: null, conversationId: null });

export function setLlmFocus(next: Partial<LlmFocus>): void {
  // A page effect writes focus; subscribing it to focus would rerun its
  // cleanup (which clears focus) and create an update loop.
  untrack(() => {
    const merged = { ...focus, ...next };
    if (merged.jobId === focus.jobId && merged.conversationId === focus.conversationId) return;
    focus = merged;
  });
}

export function getLlmFocus(): LlmFocus {
  return focus;
}

/** The payload of `llm_ui_focus`: ids only on the page that shows them. */
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
