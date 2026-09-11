//! "Roda no Linux?" — o resumo do ProtonDB antes de comprar.
//!
//! `https://www.protondb.com/api/v1/reports/summaries/<appid>.json` devolve o
//! consenso dos relatos da comunidade: `tier`, `confidence`, `score`, `total`,
//! `trendingTier` e `bestReportedTier`. Um appid sem nenhum relato responde
//! 404 — isso não é erro, é "pending".
//!
//! A entrada aceita appid, link da loja e nome do jogo (resolvido pelo
//! `appmanifest` local), e a biblioteca Steam inteira de uma vez, com pausa
//! entre as consultas para não bater na API sem educação. Cada resposta fica
//! em cache por um dia, então exportar CSV depois de consultar não repete a
//! rodada de rede.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::steam;
use crate::core::tools::{report, ProgressFn};

const ID: &str = "protondb";
const API: &str = "https://www.protondb.com/api/v1/reports/summaries";
const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// A resposta crua da API. Os campos vêm em camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub total: u64,
    #[serde(rename = "trendingTier", default)]
    pub trending_tier: String,
    #[serde(rename = "bestReportedTier", default)]
    pub best_reported_tier: String,
}

pub fn parse_summary(body: &str) -> anyhow::Result<Summary> {
    let s: Summary = serde_json::from_str(body)?;
    Ok(s)
}

/// Ordem dos degraus, do melhor para o pior. Serve para ordenar a tabela.
pub fn tier_rank(tier: &str) -> u8 {
    match tier.to_ascii_lowercase().as_str() {
        "platinum" => 6,
        "gold" => 5,
        "silver" => 4,
        "bronze" => 3,
        "borked" => 2,
        "pending" => 1,
        _ => 0,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtonOptions {
    /// Appids, links da loja ou nomes de jogo.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Consulta a biblioteca Steam local inteira.
    #[serde(default)]
    pub scan_library: bool,
    /// Pastas `steamapps` extras.
    #[serde(default)]
    pub steam_dirs: Vec<String>,
    /// Pausa entre consultas, em ms.
    #[serde(default = "default_delay")]
    pub delay_ms: u64,
    /// Ignora o cache do dia.
    #[serde(default)]
    pub refresh: bool,
    /// Caminho do arquivo a escrever; vazio não exporta.
    #[serde(default)]
    pub export_path: String,
    /// "csv" | "md"
    #[serde(default = "default_format")]
    pub export_format: String,
}

fn default_delay() -> u64 {
    250
}
fn default_format() -> String {
    "csv".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtonEntry {
    pub app_id: u32,
    pub name: String,
    pub tier: String,
    pub confidence: String,
    pub score: f64,
    pub total: u64,
    pub trending_tier: String,
    pub best_tier: String,
    pub rank: u8,
    pub url: String,
    pub cached: bool,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtonResult {
    pub entries: Vec<ProtonEntry>,
    pub checked: u64,
    pub from_cache: u64,
    /// O que foi digitado e não virou appid.
    pub unresolved: Vec<String>,
    pub exported: Option<String>,
}

fn cache_path(app_id: u32) -> Option<PathBuf> {
    crate::core::tools::tools_dir().map(|d| d.join("protondb").join(format!("{}.json", app_id)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    fetched_at: i64,
    summary: Summary,
}

fn read_cache(app_id: u32, refresh: bool) -> Option<Summary> {
    if refresh {
        return None;
    }
    let path = cache_path(app_id)?;
    let text = std::fs::read_to_string(path).ok()?;
    let cached: Cached = serde_json::from_str(&text).ok()?;
    let age = chrono::Utc::now().timestamp() - cached.fetched_at;
    (0..CACHE_TTL_SECS).contains(&age).then_some(cached.summary)
}

fn write_cache(app_id: u32, summary: &Summary) {
    let Some(path) = cache_path(app_id) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let body = Cached {
        fetched_at: chrono::Utc::now().timestamp(),
        summary: summary.clone(),
    };
    if let Ok(text) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, text);
    }
}

/// Página da loja, para o botão "abrir" da tabela.
pub fn store_url(app_id: u32) -> String {
    format!("https://store.steampowered.com/app/{}/", app_id)
}

pub fn protondb_url(app_id: u32) -> String {
    format!("https://www.protondb.com/app/{}", app_id)
}

fn csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_csv(entries: &[ProtonEntry]) -> String {
    let mut out =
        String::from("appid,jogo,tier,confianca,score,relatos,tendencia,melhor,protondb\n");
    for e in entries {
        out.push_str(&format!(
            "{},{},{},{},{:.2},{},{},{},{}\n",
            e.app_id,
            csv_cell(&e.name),
            e.tier,
            e.confidence,
            e.score,
            e.total,
            e.trending_tier,
            e.best_tier,
            protondb_url(e.app_id)
        ));
    }
    out
}

pub fn to_markdown(entries: &[ProtonEntry]) -> String {
    let mut out = String::from("| Jogo | Tier | Confiança | Relatos | ProtonDB |\n");
    out.push_str("| --- | --- | --- | ---: | --- |\n");
    for e in entries {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            e.name.replace('|', "\\|"),
            e.tier,
            e.confidence,
            e.total,
            protondb_url(e.app_id)
        ));
    }
    out
}

/// Transforma o que o usuário digitou em `(appid, nome)`. O que não vira
/// appid volta na lista de não resolvidos, para a UI pedir o número.
pub fn resolve_targets(
    inputs: &[String],
    apps: &[steam::SteamApp],
    scan_library: bool,
) -> (Vec<(u32, String)>, Vec<String>) {
    let mut targets: Vec<(u32, String)> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    fn push(id: u32, name: String, targets: &mut Vec<(u32, String)>) {
        if !targets.iter().any(|(a, _)| *a == id) {
            targets.push((id, name));
        }
    }
    for raw in inputs {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(id) = steam::parse_app_id(s) {
            let name = steam::name_for_app(apps, id).unwrap_or_else(|| format!("App {}", id));
            push(id, name, &mut targets);
            continue;
        }
        if let Some(app) = steam::app_for_name(apps, s) {
            push(app.app_id, app.name, &mut targets);
            continue;
        }
        unresolved.push(s.to_string());
    }
    if scan_library {
        for app in apps {
            push(app.app_id, app.name.clone(), &mut targets);
        }
    }
    (targets, unresolved)
}

async fn fetch(client: &reqwest::Client, app_id: u32) -> anyhow::Result<Option<Summary>> {
    let resp = client
        .get(format!("{}/{}.json", API, app_id))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        // Jogo sem nenhum relato: o ProtonDB chama isso de "pending".
        return Ok(None);
    }
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let body = resp.text().await?;
    Ok(Some(parse_summary(&body)?))
}

fn entry_from(app_id: u32, name: &str, summary: Summary, cached: bool) -> ProtonEntry {
    let tier = if summary.tier.is_empty() {
        "pending".to_string()
    } else {
        summary.tier
    };
    let rank = tier_rank(&tier);
    ProtonEntry {
        app_id,
        name: name.to_string(),
        tier,
        confidence: summary.confidence,
        score: summary.score,
        total: summary.total,
        trending_tier: summary.trending_tier,
        best_tier: summary.best_reported_tier,
        rank,
        url: store_url(app_id),
        cached,
        ok: true,
        error: None,
    }
}

fn pending_entry(app_id: u32, name: &str) -> ProtonEntry {
    ProtonEntry {
        app_id,
        name: name.to_string(),
        tier: "pending".into(),
        confidence: String::new(),
        score: 0.0,
        total: 0,
        trending_tier: String::new(),
        best_tier: String::new(),
        rank: tier_rank("pending"),
        url: store_url(app_id),
        cached: false,
        ok: true,
        error: None,
    }
}

pub async fn run(opts: ProtonOptions, progress: ProgressFn) -> anyhow::Result<ProtonResult> {
    let apps = steam::scan_apps(&steam::steam_libraries(&opts.steam_dirs));
    let (targets, unresolved) = resolve_targets(&opts.inputs, &apps, opts.scan_library);
    if targets.is_empty() {
        if unresolved.is_empty() {
            anyhow::bail!("informe um appid, um link da loja ou marque a biblioteca inteira");
        }
        anyhow::bail!(
            "não achei appid para: {} — cole o link da loja ou o número",
            unresolved.join(", ")
        );
    }

    let client = crate::core::tools::client()?;
    let total = targets.len() as u64;
    let mut entries: Vec<ProtonEntry> = Vec::new();
    let mut from_cache = 0u64;

    for (i, (app_id, name)) in targets.iter().enumerate() {
        report(
            &progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(name.clone()),
        );
        if let Some(summary) = read_cache(*app_id, opts.refresh) {
            from_cache += 1;
            entries.push(entry_from(*app_id, name, summary, true));
            continue;
        }
        match fetch(&client, *app_id).await {
            Ok(Some(summary)) => {
                write_cache(*app_id, &summary);
                entries.push(entry_from(*app_id, name, summary, false));
            }
            Ok(None) => entries.push(pending_entry(*app_id, name)),
            Err(e) => entries.push(ProtonEntry {
                ok: false,
                error: Some(e.to_string()),
                ..pending_entry(*app_id, name)
            }),
        }
        if opts.delay_ms > 0 && i + 1 < targets.len() {
            tokio::time::sleep(std::time::Duration::from_millis(opts.delay_ms.min(5_000))).await;
        }
    }

    entries.sort_by(|a, b| {
        b.rank
            .cmp(&a.rank)
            .then_with(|| b.total.cmp(&a.total))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut exported = None;
    let path = opts.export_path.trim();
    if !path.is_empty() {
        let body = if opts.export_format == "md" {
            to_markdown(&entries)
        } else {
            to_csv(&entries)
        };
        let target = PathBuf::from(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, body)?;
        exported = Some(target.to_string_lossy().to_string());
    }

    report(&progress, ID, "done", total, Some(total), None);
    Ok(ProtonResult {
        entries,
        checked: total,
        from_cache,
        unresolved,
        exported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resposta real do appid 570 (Dota 2), copiada da API.
    const FIXTURE: &str = r#"{
  "bestReportedTier": "platinum",
  "confidence": "strong",
  "score": 0.69,
  "tier": "gold",
  "total": 355,
  "trendingTier": "gold"
}"#;

    #[test]
    fn fixture_parses_into_every_field() {
        let s = parse_summary(FIXTURE).expect("resumo válido");
        assert_eq!(s.tier, "gold");
        assert_eq!(s.confidence, "strong");
        assert_eq!(s.total, 355);
        assert_eq!(s.trending_tier, "gold");
        assert_eq!(s.best_reported_tier, "platinum");
        assert!((s.score - 0.69).abs() < 1e-9);
    }

    #[test]
    fn missing_fields_do_not_break_the_parse() {
        let s = parse_summary(r#"{"tier":"borked"}"#).expect("resumo mínimo");
        assert_eq!(s.tier, "borked");
        assert_eq!(s.total, 0);
        assert!(s.best_reported_tier.is_empty());
        assert!(parse_summary("não é json").is_err());
        assert!(parse_summary("42").is_err());
        // Uma lista JSON no lugar do objeto também não passa por resumo.
        assert!(parse_summary("[1,2,3]").is_err());
    }

    #[test]
    fn tiers_are_ordered_from_platinum_to_pending() {
        assert!(tier_rank("platinum") > tier_rank("gold"));
        assert!(tier_rank("gold") > tier_rank("silver"));
        assert!(tier_rank("silver") > tier_rank("bronze"));
        assert!(tier_rank("bronze") > tier_rank("borked"));
        assert!(tier_rank("borked") > tier_rank("pending"));
        assert_eq!(tier_rank("GOLD"), tier_rank("gold"));
        assert_eq!(tier_rank("inventado"), 0);
    }

    fn entry(app_id: u32, name: &str, body: &str) -> ProtonEntry {
        entry_from(app_id, name, parse_summary(body).expect("fixture"), false)
    }

    #[test]
    fn csv_quotes_the_names_with_commas() {
        let e = entry(1, "Jogo, o Retorno", FIXTURE);
        let csv = to_csv(&[e]);
        assert!(csv
            .lines()
            .next()
            .unwrap_or_default()
            .starts_with("appid,jogo,tier"));
        assert!(csv.contains("\"Jogo, o Retorno\""), "{}", csv);
        assert!(
            csv.contains(",gold,strong,0.69,355,gold,platinum,"),
            "{}",
            csv
        );
        assert!(csv.contains("https://www.protondb.com/app/1"));
    }

    #[test]
    fn markdown_escapes_the_pipe() {
        let md = to_markdown(&[entry(2, "A | B", FIXTURE)]);
        assert!(md.contains("A \\| B"), "{}", md);
        assert!(md.lines().count() == 3, "{}", md);
    }

    #[test]
    fn targets_come_from_ids_urls_names_and_the_library() {
        let apps = vec![
            steam::SteamApp {
                app_id: 570,
                name: "Dota 2".into(),
                install_dir: String::new(),
                size_on_disk: 0,
                library: String::new(),
            },
            steam::SteamApp {
                app_id: 620,
                name: "Portal 2".into(),
                install_dir: String::new(),
                size_on_disk: 0,
                library: String::new(),
            },
        ];
        let inputs = vec![
            "570".to_string(),
            "https://store.steampowered.com/app/1091500/Cyberpunk_2077/".to_string(),
            "portal 2".to_string(),
            "Um Jogo Que Nao Tenho".to_string(),
            "  ".to_string(),
        ];
        let (targets, unresolved) = resolve_targets(&inputs, &apps, false);
        assert_eq!(
            targets,
            vec![
                (570, "Dota 2".to_string()),
                (1_091_500, "App 1091500".to_string()),
                (620, "Portal 2".to_string()),
            ]
        );
        assert_eq!(unresolved, vec!["Um Jogo Que Nao Tenho".to_string()]);

        // A biblioteca inteira entra sem repetir o que já estava na lista.
        let (targets, _) = resolve_targets(&["570".into()], &apps, true);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], (570, "Dota 2".to_string()));
    }

    #[tokio::test]
    async fn refuses_an_empty_request() {
        let r = run(
            ProtonOptions {
                inputs: Vec::new(),
                scan_library: false,
                steam_dirs: Vec::new(),
                delay_ms: 0,
                refresh: false,
                export_path: String::new(),
                export_format: "csv".into(),
            },
            crate::core::tools::noop_progress(),
        )
        .await;
        assert!(r.is_err());
    }

    /// Bate na API de verdade com o appid 570 (Dota 2) e confere o formato.
    /// `cargo test -p omniget-core --lib -- --ignored live_protondb`
    #[tokio::test]
    #[ignore]
    async fn live_protondb_answers_in_the_expected_shape() {
        let client = crate::core::tools::client().expect("cliente");
        let s = fetch(&client, 570)
            .await
            .expect("consulta")
            .expect("570 tem relatos");
        eprintln!("570 → {:?}", s);
        assert!(tier_rank(&s.tier) >= 2, "tier fora da escala: {}", s.tier);
        assert!(s.total > 0, "sem relatos");
        assert!(!s.confidence.is_empty());
        assert!(
            (0.0..=1.0).contains(&s.score),
            "score fora de 0..1: {}",
            s.score
        );
        // Um appid que não existe responde 404, e isso vira "pending".
        assert!(fetch(&client, 999_999_999)
            .await
            .expect("404 não é erro")
            .is_none());
    }
}
