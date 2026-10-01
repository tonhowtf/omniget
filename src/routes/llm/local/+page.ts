import { redirect } from "@sveltejs/kit";
import { LLM_REDIRECTS } from "$lib/llm/workspace-navigation";

export function load() {
  redirect(307, LLM_REDIRECTS["/llm/local"]);
}
