#[tauri::command]
pub fn diagnose_download_error(stderr: String) -> Option<crate::core::root_cause::Diagnosis> {
    crate::core::root_cause::diagnose(&stderr)
}

/// Despejo da caixa-preta, para colar num relatorio de bug.
///
/// As linhas ja saem redigidas pelo `flight_recorder::redact` — token, cookie e
/// caminho com nome de usuario nao chegam ate aqui.
#[tauri::command]
pub fn flight_recorder_dump() -> Vec<String> {
    crate::core::flight_recorder::dump()
}

/// Confere um lote antes de enfileirar: o que da para baixar, o que nao da, e
/// se cabe no disco.
///
/// B34. Existe porque descobrir que o disco encheu na metade de um lote de 40
/// aulas custa muito mais do que a checagem. Nao consulta a rede: as perguntas
/// caras (tamanho real de cada item) ficam de fora de proposito — a resposta
/// util e "vale comecar?", nao "quanto exatamente".
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

/// Classifica cada URL do lote. Separado do comando para poder ser testado sem
/// montar um `AppState`.
fn classificar_lote(
    urls: Vec<String>,
    suportada: impl Fn(&str) -> bool,
) -> Vec<crate::core::preflight::ItemCheck> {
    use crate::core::preflight::{ItemCheck, Problem};

    let mut vistos = std::collections::HashSet::new();
    urls.into_iter()
        .map(|url| {
            let problem = if !vistos.insert(url.clone()) {
                // Duplicata dentro do proprio lote conta como "ja tenho": o
                // usuario colou a mesma URL duas vezes.
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
