//! Screen-only actions (`washmybelly_ui`): killing a process, renicing,
//! one-time setup, revealing a file. Routed as `<module>.<action>` to the
//! module's `ui_action`. None of these is an MCP tool.

use serde_json::Value;

pub async fn call(action: &str, a: Value) -> Result<Value, String> {
    use super::*;
    let (module, rest) = action
        .split_once('.')
        .ok_or("action must be <module>.<name>")?;
    match module {
        "plan" => plan::ui_action(rest, a).await,
        "monitor" => monitor::ui_action(rest, a).await,
        "disk" => disk::ui_action(rest, a).await,
        "clean" => clean::ui_action(rest, a).await,
        "apps" => apps::ui_action(rest, a).await,
        "startup" => startup::ui_action(rest, a).await,
        "privacy" => privacy::ui_action(rest, a).await,
        "tweaks" => tweaks::ui_action(rest, a).await,
        "hosts" => hosts::ui_action(rest, a).await,
        "packages" => packages::ui_action(rest, a).await,
        "power" => power::ui_action(rest, a).await,
        "game" => game::ui_action(rest, a).await,
        "perf" => perf::ui_action(rest, a).await,
        "security" => security::ui_action(rest, a).await,
        "maintenance" => maintenance::ui_action(rest, a).await,
        _ => Err(format!("unknown module {module}")),
    }
}
