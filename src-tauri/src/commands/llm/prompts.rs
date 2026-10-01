use omniget_core::core::llm::roster_store::{set_prompt_defaults, PromptDefaults};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptPayload {
    pub omni: String,
    pub builder: String,
    pub scout: String,
    pub template_chief: String,
    pub template_researcher: String,
    pub template_writer: String,
    pub template_em: String,
    pub template_reviewer: String,
    pub help: String,
}

#[tauri::command]
pub fn sync_llm_prompts(prompts: PromptPayload) {
    set_prompt_defaults(PromptDefaults {
        omni: prompts.omni,
        builder: prompts.builder,
        scout: prompts.scout,
        template_chief: prompts.template_chief,
        template_researcher: prompts.template_researcher,
        template_writer: prompts.template_writer,
        template_em: prompts.template_em,
        template_reviewer: prompts.template_reviewer,
        help: prompts.help,
    });
}
