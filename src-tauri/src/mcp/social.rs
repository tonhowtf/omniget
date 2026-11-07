use serde_json::Value;
use tauri::AppHandle;

use super::policy::Principal;
use crate::commands::social::{self, approvals, Caller};

pub async fn call(app: &AppHandle, p: &Principal, name: &str, a: Value) -> Result<Value, String> {
    approvals::set_app(app);
    social::call(Caller::Agent(p.name.clone()), name, a).await
}
