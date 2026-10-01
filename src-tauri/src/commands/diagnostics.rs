#[tauri::command]
pub fn diagnose_download_error(stderr: String) -> Option<crate::core::root_cause::Diagnosis> {
    crate::core::root_cause::diagnose(&stderr)
}

#[tauri::command]
pub fn flight_recorder_dump() -> Vec<String> {
    crate::core::flight_recorder::dump()
}

#[tauri::command]
pub async fn preflight_batch(
    state: tauri::State<'_, crate::AppState>,
    urls: Vec<String>,
    output_dir: Option<String>,
) -> Result<crate::core::preflight::PreflightReport, String> {
    let checks = classificar_lote(urls, |u| state.registry.find_platform(u).is_some());

    let livre = output_dir
        .map(std::path::PathBuf::from)
        .as_deref()
        .and_then(crate::core::preflight::available_space);

    Ok(crate::core::preflight::build_report(checks, livre))
}

fn classificar_lote(
    urls: Vec<String>,
    suportada: impl Fn(&str) -> bool,
) -> Vec<crate::core::preflight::ItemCheck> {
    use crate::core::preflight::{ItemCheck, Problem};

    let mut vistos = std::collections::HashSet::new();
    urls.into_iter()
        .map(|url| {
            let problem = if !vistos.insert(url.clone()) {
                Some(Problem::AlreadyHave)
            } else if !suportada(&url) {
                Some(Problem::Unsupported)
            } else {
                None
            };
            ItemCheck {
                url,
                title: None,
                estimated_bytes: None,
                problem,
            }
        })
        .collect()
}
