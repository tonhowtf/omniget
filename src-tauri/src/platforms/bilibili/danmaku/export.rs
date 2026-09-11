//! Export standalone dos comentários flutuantes: resolve a parte pedida da
//! URL, puxa o danmaku e grava nos formatos que o motor já sabe gerar.
//!
//! Nada aqui baixa vídeo. O caminho é o mesmo que o motor de download usa de
//! sidecar (`url_kind` → `parser` → `fetch_elems`), só que parando no `cid` e
//! saindo direto para arquivo, com filtros e estilo pelo caminho.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use omniget_core::core::tools::{report, ProgressFn};
use serde::{Deserialize, Serialize};

use super::super::api::{ApiClient, BilibiliError};
use super::super::{active_account_slug, cookie, parser, url_kind};
use super::ass::AssRenderOptions;
use super::filter::{self, DanmakuFilter};
use super::{fetch_elems, DanmakuFormat};

/// Id da tool no evento `tool-progress`.
pub const ID: &str = "bili-danmaku";

/// Quando a API não diz quanto o vídeo dura, o número de segmentos vira chute.
/// Seis minutos por segmento: dez segmentos cobrem uma hora, e segmento vazio
/// só custa uma requisição (o `fetch_elems` avisa no log e segue).
const FALLBACK_DURATION_SECS: u64 = 3600;

#[derive(Debug, Clone, Deserialize)]
pub struct ExportOptions {
    /// Aceita link completo, `b23.tv`, `BV...`, `av...` ou `ep...`.
    pub url: String,
    /// Parte do vídeo (`?p=N`); 0 usa a que estiver na URL, senão a primeira.
    #[serde(default)]
    pub page: u32,
    /// `xml`, `ass` e/ou `json`. Vazio exporta os três.
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub filter: DanmakuFilter,
    #[serde(default)]
    pub style: AssRenderOptions,
    /// Pasta de saída; vazia cai na pasta de downloads do sistema.
    #[serde(default)]
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DanmakuPart {
    pub page: u32,
    pub title: String,
    pub cid: u64,
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFile {
    pub format: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub title: String,
    pub part: DanmakuPart,
    /// Todas as partes que a URL tem, para a UI montar o seletor.
    pub parts: Vec<DanmakuPart>,
    pub fetched: usize,
    pub kept: usize,
    pub files: Vec<ExportFile>,
    /// `true` quando saiu com a sessão de uma conta, não anônimo.
    pub with_account: bool,
}

pub async fn run(opts: &ExportOptions, progress: &ProgressFn) -> Result<ExportResult> {
    let url = opts.url.trim();
    if url.is_empty() {
        return Err(anyhow!("cole o link do vídeo do Bilibili"));
    }
    report(progress, ID, "started", 0, Some(4), None);

    // Cookies anônimos (buvid3/buvid4/bili_ticket): sem eles a assinatura WBI
    // do endpoint de danmaku é recusada em boa parte dos vídeos.
    if let Err(e) = cookie::ensure_fresh().await {
        tracing::warn!("[bili-danmaku] cookies anônimos falharam: {:?}", e);
    }
    let slug = active_account_slug();
    let with_account = slug.is_some();
    let client = build_client(slug.as_deref())?;

    report(
        progress,
        ID,
        "progress",
        1,
        Some(4),
        Some("resolvendo o link".into()),
    );
    let mut effective = url.to_string();
    if url_kind::is_b23_short(&effective) {
        match url_kind::resolve_b23(&client, &effective).await {
            Ok(full) => effective = full,
            Err(e) => return Err(friendly(e, with_account)),
        }
    }
    let kind = url_kind::detect(&effective)
        .map_err(|_| anyhow!("não reconheci um vídeo do Bilibili em: {}", url))?;
    let parsed = parser::parse(&client, &kind)
        .await
        .map_err(|e| friendly(e, with_account))?;

    let parts = collect_parts(&parsed);
    if parts.is_empty() {
        return Err(anyhow!(
            "esse link não tem parte de vídeo com comentários — abra uma parte específica"
        ));
    }
    let wanted = if opts.page > 0 {
        opts.page
    } else {
        page_from_kind(&kind)
    };
    let part = pick_part(&parts, wanted).ok_or_else(|| {
        anyhow!(
            "a parte {} não existe; esse vídeo tem {}",
            wanted,
            describe_pages(&parts)
        )
    })?;

    let duration = if part.duration_secs > 0 {
        part.duration_secs
    } else {
        FALLBACK_DURATION_SECS
    };
    report(
        progress,
        ID,
        "progress",
        2,
        Some(4),
        Some(format!("baixando os comentários de {}", part.title)),
    );
    let elems = fetch_elems(&client, part.cid, duration)
        .await
        .map_err(|e| friendly(e, with_account))?;
    let fetched = elems.len();

    report(
        progress,
        ID,
        "progress",
        3,
        Some(4),
        Some("filtrando".into()),
    );
    let kept_elems = filter::apply(&elems, &opts.filter)?;
    if kept_elems.is_empty() {
        return Err(anyhow!(
            "os filtros não deixaram nenhum comentário de pé ({} vieram do Bilibili)",
            fetched
        ));
    }

    let dir = output_dir(&opts.output_dir)?;
    std::fs::create_dir_all(&dir)?;
    let stem = file_stem(&parsed.title, part, parts.len());
    let mut files = Vec::new();
    for format in wanted_formats(&opts.formats) {
        let body = match format {
            DanmakuFormat::Ass => super::ass::render(&kept_elems, &opts.style),
            other => super::render(&kept_elems, other),
        };
        let path = dir.join(format!("{}.danmaku.{}", stem, format.extension()));
        std::fs::write(&path, &body)
            .map_err(|e| anyhow!("não consegui gravar {}: {}", path.display(), e))?;
        files.push(ExportFile {
            format: format.extension().to_string(),
            path: path.to_string_lossy().to_string(),
            bytes: body.len() as u64,
        });
    }

    report(progress, ID, "done", 4, Some(4), None);
    Ok(ExportResult {
        title: parsed.title.clone(),
        part: part.clone(),
        parts,
        fetched,
        kept: kept_elems.len(),
        files,
        with_account,
    })
}

fn build_client(slug: Option<&str>) -> Result<ApiClient> {
    let client = ApiClient::new().map_err(|e| anyhow!("{}", e.i18n_key()))?;
    Ok(match slug {
        Some(s) => client.with_account(s),
        None => client.with_anonymous_cookies(),
    })
}

/// Erro de API virando frase. Tudo que depende de sessão degrada com o mesmo
/// recado — é o caso do conteúdo pago (`cheese`) e do bangumi regional.
fn friendly(e: BilibiliError, with_account: bool) -> anyhow::Error {
    match e {
        BilibiliError::NotLoggedIn | BilibiliError::CookieMissing => anyhow!(
            "esse conteúdo pede a sessão do Bilibili — entre na conta pelo gerenciador de cookies e tente de novo"
        ),
        BilibiliError::PremiumRequired if with_account => anyhow!(
            "a conta conectada não tem acesso a esse conteúdo pago"
        ),
        BilibiliError::PremiumRequired => anyhow!(
            "conteúdo pago: conecte uma conta do Bilibili que já tenha a compra"
        ),
        BilibiliError::GeoBlocked => {
            anyhow!("o Bilibili bloqueia esse conteúdo na sua região; tente com proxy")
        }
        BilibiliError::RateLimited => {
            anyhow!("o Bilibili está limitando as requisições; espere um pouco e repita")
        }
        BilibiliError::ContentUnavailable => {
            anyhow!("não achei comentários nessa parte — ela pode ter sido removida")
        }
        other => anyhow!("{}", other.i18n_key()),
    }
}

fn collect_parts(parsed: &parser::ParsedContent) -> Vec<DanmakuPart> {
    let mut out = Vec::new();
    for (i, item) in parsed.items.iter().enumerate() {
        let cid = match item.cid {
            Some(c) if c > 0 => c,
            _ => continue,
        };
        let page = item.page.unwrap_or((i + 1) as u32);
        let title = if item.title.trim().is_empty() {
            format!("P{}", page)
        } else {
            item.title.trim().to_string()
        };
        out.push(DanmakuPart {
            page,
            title,
            cid,
            duration_secs: item.duration_seconds.unwrap_or(0.0).max(0.0) as u64,
        });
    }
    out
}

fn page_from_kind(kind: &url_kind::UrlKind) -> u32 {
    match kind {
        url_kind::UrlKind::Video { page, .. } => page.unwrap_or(0),
        _ => 0,
    }
}

/// `wanted` 0 significa "a primeira que vier".
fn pick_part(parts: &[DanmakuPart], wanted: u32) -> Option<&DanmakuPart> {
    if wanted == 0 {
        return parts.first();
    }
    parts.iter().find(|p| p.page == wanted)
}

fn describe_pages(parts: &[DanmakuPart]) -> String {
    let pages: Vec<String> = parts.iter().map(|p| p.page.to_string()).collect();
    format!("{} parte(s): {}", parts.len(), pages.join(", "))
}

fn wanted_formats(raw: &[String]) -> Vec<DanmakuFormat> {
    let mut out: Vec<DanmakuFormat> = Vec::new();
    for name in raw {
        if let Some(f) = DanmakuFormat::from_name(name) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    if out.is_empty() {
        out = vec![DanmakuFormat::Xml, DanmakuFormat::Ass, DanmakuFormat::Json];
    }
    out
}

fn output_dir(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        return Ok(PathBuf::from(trimmed));
    }
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| anyhow!("escolha a pasta de saída"))
}

/// `Título - P2 Nome da parte`, já sem os caracteres que o sistema recusa.
/// Vídeo de uma parte só não ganha sufixo.
fn file_stem(title: &str, part: &DanmakuPart, total_parts: usize) -> String {
    let base = sanitize_filename::sanitize(title.trim());
    let base = if base.is_empty() {
        "bilibili".to_string()
    } else {
        base
    };
    if total_parts <= 1 {
        return base;
    }
    let part_name = sanitize_filename::sanitize(&part.title);
    let tail = if part_name.is_empty() || part_name == base {
        format!("P{}", part.page)
    } else {
        format!("P{} {}", part.page, part_name)
    };
    let joined = format!("{} - {}", base, tail);
    // Nome de arquivo longo demais quebra em ext4 (255 bytes) e em NTFS.
    if joined.len() > 150 {
        joined.chars().take(150).collect()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::bilibili::parser::{ContentMetadata, EpisodeItem};

    fn item(page: u32, cid: u64, title: &str, dur: f64) -> EpisodeItem {
        EpisodeItem {
            title: title.to_string(),
            cid: Some(cid),
            page: Some(page),
            duration_seconds: Some(dur),
            ..EpisodeItem::default()
        }
    }

    fn parsed(items: Vec<EpisodeItem>) -> parser::ParsedContent {
        parser::ParsedContent {
            title: "Aula de Rust".into(),
            items,
            metadata: ContentMetadata::default(),
            pagination: None,
        }
    }

    #[test]
    fn parts_come_from_the_parsed_items() {
        let p = parsed(vec![
            item(1, 111, "Abertura", 300.0),
            item(2, 222, "Meio", 600.0),
        ]);
        let parts = collect_parts(&p);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].cid, 222);
        assert_eq!(parts[1].duration_secs, 600);
    }

    #[test]
    fn parts_without_cid_are_dropped() {
        let mut sem_cid = item(2, 0, "Sem cid", 10.0);
        sem_cid.cid = None;
        let p = parsed(vec![item(1, 111, "Ok", 10.0), sem_cid]);
        assert_eq!(collect_parts(&p).len(), 1);
    }

    #[test]
    fn parts_fall_back_to_the_position_when_there_is_no_page() {
        let mut a = item(1, 111, "Episódio 1", 10.0);
        a.page = None;
        let mut b = item(2, 222, "Episódio 2", 10.0);
        b.page = None;
        let parts = collect_parts(&parsed(vec![a, b]));
        assert_eq!(parts[0].page, 1);
        assert_eq!(parts[1].page, 2);
    }

    #[test]
    fn page_zero_takes_the_first_part() {
        let parts = collect_parts(&parsed(vec![
            item(1, 111, "a", 1.0),
            item(2, 222, "b", 1.0),
        ]));
        assert_eq!(pick_part(&parts, 0).unwrap().cid, 111);
        assert_eq!(pick_part(&parts, 2).unwrap().cid, 222);
        assert!(pick_part(&parts, 9).is_none());
    }

    #[test]
    fn page_comes_from_the_url_when_the_option_is_zero() {
        let kind = url_kind::detect("https://www.bilibili.com/video/BV1xx411c7mu?p=3").unwrap();
        assert_eq!(page_from_kind(&kind), 3);
        let sem = url_kind::detect("https://www.bilibili.com/video/BV1xx411c7mu").unwrap();
        assert_eq!(page_from_kind(&sem), 0);
        let ep = url_kind::detect("https://www.bilibili.com/bangumi/play/ep123456").unwrap();
        assert_eq!(page_from_kind(&ep), 0);
    }

    #[test]
    fn formats_default_to_all_three_and_ignore_junk() {
        assert_eq!(wanted_formats(&[]).len(), 3);
        assert_eq!(
            wanted_formats(&["ass".into(), "ass".into(), "srt".into()]),
            vec![DanmakuFormat::Ass]
        );
        assert_eq!(
            wanted_formats(&["JSON".into(), "xml".into()]),
            vec![DanmakuFormat::Json, DanmakuFormat::Xml]
        );
    }

    #[test]
    fn file_stem_only_names_the_part_on_multipart() {
        let part = DanmakuPart {
            page: 2,
            title: "Parte: dois".into(),
            cid: 1,
            duration_secs: 1,
        };
        assert_eq!(file_stem("Aula/Rust", &part, 1), "AulaRust");
        let multi = file_stem("Aula", &part, 4);
        assert!(multi.starts_with("Aula - P2 "), "{}", multi);
        assert!(!multi.contains('/'), "{}", multi);
    }

    /// Puxa um vídeo público de verdade e grava os três formatos.
    /// `cargo test -p omniget --lib -- --ignored live_danmaku_export`
    #[tokio::test]
    #[ignore]
    async fn live_danmaku_export_writes_the_three_formats() {
        let dir = std::env::temp_dir().join("omniget-danmaku-live");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = ExportOptions {
            url: "https://www.bilibili.com/video/BV1xx411c7mu".into(),
            page: 0,
            formats: Vec::new(),
            filter: DanmakuFilter::default(),
            style: AssRenderOptions::default(),
            output_dir: dir.to_string_lossy().to_string(),
        };
        let res = run(&opts, &omniget_core::core::tools::noop_progress())
            .await
            .unwrap();
        eprintln!(
            "{} — parte {} (cid {}), {} comentários, conta: {}",
            res.title, res.part.page, res.part.cid, res.fetched, res.with_account
        );
        assert!(res.fetched > 0);
        assert_eq!(res.kept, res.fetched, "sem filtro nada pode sumir");
        assert_eq!(res.files.len(), 3);
        for f in &res.files {
            let body = std::fs::read_to_string(&f.path).unwrap();
            assert!(f.bytes > 0, "{} saiu vazio", f.format);
            match f.format.as_str() {
                "xml" => assert!(body.contains("<i>") && body.contains("<d p=")),
                "ass" => assert!(body.contains("[Events]") && body.contains("Dialogue:")),
                _ => assert!(body.trim_start().starts_with('[')),
            }
        }

        // Segunda passada com filtro: o teto de densidade e a busca por
        // palavra têm que morder de verdade num vídeo real.
        let filtrado = ExportOptions {
            formats: vec!["json".into()],
            filter: DanmakuFilter {
                kinds: vec![super::super::filter::KIND_SCROLL.to_string()],
                max_per_second: 1,
                ..DanmakuFilter::default()
            },
            ..opts
        };
        let res2 = run(&filtrado, &omniget_core::core::tools::noop_progress())
            .await
            .unwrap();
        assert_eq!(res2.files.len(), 1);
        assert!(res2.kept < res2.fetched, "o filtro não cortou nada");
        assert!(
            res2.kept <= res2.part.duration_secs as usize + 1,
            "{} comentários para {} segundos a 1/s",
            res2.kept,
            res2.part.duration_secs
        );
        eprintln!("com filtro: {} de {}", res2.kept, res2.fetched);
    }
}
