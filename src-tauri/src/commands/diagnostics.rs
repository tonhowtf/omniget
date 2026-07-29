#[tauri::command]
pub async fn get_hwaccel_info() -> omniget_core::core::hwaccel::HwAccelInfo {
    omniget_core::core::hwaccel::detect_hwaccel().await
}

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

/// Apaga o que a caixa-preta guardou.
///
/// Existe porque o usuario tem que poder decidir nao mandar nada, mesmo depois
/// de ter aberto o relatorio.
#[tauri::command]
pub fn flight_recorder_clear() {
    crate::core::flight_recorder::clear();
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

#[cfg(test)]
mod preflight_lote_tests {
    use super::*;
    use crate::core::preflight::Problem;

    #[test]
    fn url_repetida_no_lote_nao_baixa_duas_vezes() {
        let urls = vec![
            "https://a.com/1".to_string(),
            "https://a.com/1".to_string(),
            "https://a.com/2".to_string(),
        ];
        let checks = classificar_lote(urls, |_| true);
        assert_eq!(checks[0].problem, None, "a primeira ocorrencia baixa");
        assert_eq!(
            checks[1].problem,
            Some(Problem::AlreadyHave),
            "a segunda e a mesma coisa"
        );
        assert_eq!(checks[2].problem, None);
    }

    #[test]
    fn site_sem_downloader_e_marcado_em_vez_de_enfileirado() {
        let checks = classificar_lote(vec!["https://desconhecido.tld/x".to_string()], |_| false);
        assert_eq!(checks[0].problem, Some(Problem::Unsupported));
    }

    #[test]
    fn lote_limpo_nao_inventa_problema() {
        let checks = classificar_lote(
            vec!["https://a.com/1".to_string(), "https://a.com/2".to_string()],
            |_| true,
        );
        assert!(checks.iter().all(|c| c.problem.is_none()));
    }
}
