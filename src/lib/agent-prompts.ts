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
};

const KEYS: Record<keyof AgentPrompts, string> = {
  omni: "agents.prompts.omni",
  builder: "agents.prompts.builder",
  scout: "agents.prompts.scout",
  templateChief: "agents.prompts.template_chief",
  templateResearcher: "agents.prompts.template_researcher",
  templateWriter: "agents.prompts.template_writer",
  templateEm: "agents.prompts.template_em",
  templateReviewer: "agents.prompts.template_reviewer",
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
  };
}
