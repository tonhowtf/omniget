/**
 * Prompts for the agents the app seeds itself.
 *
 * Unlike the rest of the dictionary these strings are not pure labels: the seed
 * writes them into the user's roster, where they become editable data. They are
 * still read by the user (the roster list and the wizard show them), so they
 * follow the interface language instead of staying English forever.
 *
 * The tray has the same problem and the same answer: the frontend resolves the
 * text with `$t`/`rawTranslations` and pushes it to Rust through a command
 * (`sync_llm_prompts`), so no language detection lives in the backend. The
 * compiled-in English set in `roster_store.rs` is only the fallback for the
 * window between startup and the first push.
 *
 * Values come from `rawTranslations` and not from `$t` for the same reason as
 * `$lib/tray-strings`: the default parser substitutes any `{{placeholder}}` it
 * has no payload for, and prompts are plain prose that must arrive verbatim.
 */

/** `{ locale: { "flat.key": value } }`, the shape `translationStore` holds. */
export type TranslationBag = Record<string, Record<string, unknown>> | undefined;

// A type alias (not an interface) on purpose: Tauri's `invoke` wants
// `Record<string, unknown>`, and TypeScript only gives an implicit index
// signature to object type aliases.
type AgentPrompts = {
  omni: string;
  builder: string;
  scout: string;
  templateChief: string;
  templateResearcher: string;
  templateWriter: string;
  templateEm: string;
  templateReviewer: string;
  help: string;
};

const KEYS: Record<keyof AgentPrompts, string> = {
  omni: "llm.prompts.omni",
  builder: "llm.prompts.builder",
  scout: "llm.prompts.scout",
  templateChief: "llm.prompts.template_chief",
  templateResearcher: "llm.prompts.template_researcher",
  templateWriter: "llm.prompts.template_writer",
  templateEm: "llm.prompts.template_em",
  templateReviewer: "llm.prompts.template_reviewer",
  help: "llm.prompts.help",
};

/**
 * Builds the payload for `sync_llm_prompts` out of the flat translation bag.
 * Falls back to English, then to the key itself, exactly like `$t` does.
 */
export function agentPrompts(bag: TranslationBag, locale: string): AgentPrompts {
  const pick = (key: string): string => {
    const active = bag?.[locale]?.[key];
    if (typeof active === "string" && active.length > 0) return active;
    const fallback = bag?.en?.[key];
    if (typeof fallback === "string" && fallback.length > 0) return fallback;
    return key;
  };

  return {
    omni: pick(KEYS.omni),
    builder: pick(KEYS.builder),
    scout: pick(KEYS.scout),
    templateChief: pick(KEYS.templateChief),
    templateResearcher: pick(KEYS.templateResearcher),
    templateWriter: pick(KEYS.templateWriter),
    templateEm: pick(KEYS.templateEm),
    templateReviewer: pick(KEYS.templateReviewer),
    help: pick(KEYS.help),
  };
}
