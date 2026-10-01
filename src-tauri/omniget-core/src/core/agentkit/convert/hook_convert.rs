use super::{hook_cline, hook_code_plugin, hook_json, hook_toml};
use super::{ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::{Component, ComponentBody, ComponentKind};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

pub const FORMATS: &[&str] = &[
    "cursor",
    "copilot",
    "gemini",
    "cline_files",
    "kiro",
    "kimi_toml",
    "vibe_toml",
    "cascade",
    "code_plugin",
    "devin_v1",
    "goose",
    "crush",
];

pub struct HookConverter;

pub fn convert_native(
    c: &Component,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let ComponentBody::Hook(h) = &c.body else {
        return Err(AgentkitError::new(
            "AGENTKIT_KIND",
            format!("{} is not a hook", c.id),
        ));
    };
    match target.format(ComponentKind::Hook).unwrap_or("") {
        "cursor" => hook_json::cursor(c, h, target, scope, ctx),
        "copilot" => hook_json::copilot(c, h, target, scope, ctx),
        "gemini" => hook_json::gemini(c, h, target, scope, ctx),
        "kiro" => hook_json::kiro(c, h, target, scope, ctx),
        "cascade" => hook_json::cascade(c, h, target, scope, ctx),
        "goose" => hook_json::goose(c, h, target, scope, ctx),
        "devin_v1" => hook_json::devin(c, h, target, scope, ctx),
        "crush" => hook_json::crush(c, h, target, scope, ctx),
        "kimi_toml" => hook_toml::kimi(c, h, target, scope, ctx),
        "vibe_toml" => hook_toml::vibe(c, h, target, scope, ctx),
        "cline_files" => hook_cline::cline(c, h, target, scope, ctx),
        "code_plugin" => hook_code_plugin::code_plugin(c, h, target, scope, ctx),
        other => Err(AgentkitError::new(
            "AGENTKIT_UNSUPPORTED",
            format!("{} hook format `{other}` has no writer", target.name),
        )),
    }
}

impl Converter for HookConverter {
    fn id(&self) -> &'static str {
        "hook_dialects"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Hook
            && target
                .format(kind)
                .map(|f| FORMATS.contains(&f))
                .unwrap_or(false)
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        convert_native(c, target, scope, ctx)
    }
}
