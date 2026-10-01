export type TranslationBag = Record<string, Record<string, unknown>> | undefined;

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
