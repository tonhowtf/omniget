import { redirect } from "@sveltejs/kit";
import { LLM_REDIRECTS } from "$lib/llm/workspace-navigation";

// The old "Local" tab was a placeholder that only pointed at Models (U1):
// its address still works and lands there.
export function load() {
  redirect(307, LLM_REDIRECTS["/llm/local"]);
}
