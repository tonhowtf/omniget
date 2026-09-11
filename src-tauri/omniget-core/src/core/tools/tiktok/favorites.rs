//! `tt-favorites`: exportar os favoritos do usuário e a mídia do perfil dele.
//!
//! Duas fontes de listagem, porque o TikTok trata as duas de formas
//! diferentes:
//!
//! - **Perfil e coleção** são públicos e o yt-dlp já sabe listá-los. Uma só
//!   chamada com `--flat-playlist -J` devolve id, autor, descrição, som, data
//!   e contadores de todos os itens — dá o índice inteiro sem abrir um vídeo
//!   de cada vez.
//! - **Favoritos** (a aba do marcador) e **curtidos** são privados: só saem
//!   com a sessão do usuário, pela API web (`/api/user/collect/item_list/` e
//!   `/api/user/favorite/item_list/`), paginada por cursor. Sem sessão a tool
//!   avisa e não tenta.
//!
//! A saída é sempre um índice (CSV, JSON ou os dois) e, opcionalmente, a
//! mídia baixada — com o caminho local escrito de volta no índice.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{csv_escape, fmt_utc, sound_url, Pacer, TempCookies};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "tt-favorites";

fn def_source() -> String {
    "posts".to_string()
}
fn def_format() -> String {
    "both".to_string()
}
fn def_limit() -> u32 {
    200
}
fn def_delay() -> u64 {
    1200
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// `@usuario` ou a URL do perfil. Para `collection`, a URL da coleção.
    pub user: String,
    /// "posts" | "collection" | "favorites" | "liked".
    #[serde(default = "def_source")]
    pub source: String,
    pub dest: String,
    /// "csv" | "json" | "both".
    #[serde(default = "def_format")]
    pub index_format: String,
    /// Teto de itens. 0 = sem teto.
    #[serde(default = "def_limit")]
    pub limit: u32,
    /// Baixar também a mídia de cada item.
    #[serde(default)]
    pub download_media: bool,
    #[serde(default = "def_delay")]
    pub delay_ms: u64,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

/// Uma linha do índice.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub author: String,
    pub author_name: String,
    pub description: String,
    pub sound: String,
    pub sound_author: String,
    pub sound_url: String,
    pub created: String,
    pub created_ts: i64,
    pub duration: u64,
    pub url: String,
    pub like_count: u64,
    pub comment_count: u64,
    pub view_count: u64,
    pub save_count: u64,
    /// Preenchido quando a mídia foi baixada.
    pub local_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FavoritesResult {
    pub source: String,
    pub user: String,
    pub entries: Vec<Entry>,
    pub files: Vec<String>,
    pub downloaded: usize,
    pub failed: usize,
    pub dest: String,
    pub used_session: bool,
    pub requests: u32,
}

// ───────────────────────── leitura das duas fontes ─────────────────────────

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn u(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// Entradas do `--flat-playlist -J` do yt-dlp (perfil e coleção).
pub fn entries_from_list(v: &Value) -> Vec<Entry> {
    let Some(list) = v.get("entries").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let id = s(e, "id");
        if id.is_empty() {
            continue;
        }
        let author = s(e, "uploader");
        let url = match s(e, "url") {
            u if u.starts_with("http") => u,
            _ => format!(
                "https://www.tiktok.com/@{}/video/{}",
                if author.is_empty() { "_" } else { &author },
                id
            ),
        };
        let sound = s(e, "track");
        let sound_author = e
            .get("artists")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .map(|x| x.to_string())
            .unwrap_or_else(|| s(e, "artist"));
        let ts = e.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0);
        out.push(Entry {
            id,
            author,
            author_name: s(e, "channel"),
            description: s(e, "description"),
            sound,
            sound_author,
            // A listagem do yt-dlp não traz o id do som, então não há link
            // honesto a inventar aqui.
            sound_url: String::new(),
            created: if ts > 0 { fmt_utc(ts) } else { String::new() },
            created_ts: ts,
            duration: u(e, "duration"),
            url,
            like_count: u(e, "like_count"),
            comment_count: u(e, "comment_count"),
            view_count: u(e, "view_count"),
            save_count: u(e, "save_count"),
            local_path: String::new(),
        })
    }
    out
}

/// Uma página da API web (`itemList`), usada nos favoritos e nos curtidos.
/// Devolve as entradas, o cursor da próxima página e se ainda há mais.
pub fn entries_from_item_list(v: &Value) -> (Vec<Entry>, String, bool) {
    let cursor = match v.get("cursor") {
        Some(Value::String(c)) => c.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let has_more = match v.get("hasMore") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    };
    let Some(list) = v.get("itemList").and_then(|x| x.as_array()) else {
        return (Vec::new(), cursor, has_more);
    };
    let mut out = Vec::with_capacity(list.len());
    for it in list {
        let id = s(it, "id");
        if id.is_empty() {
            continue;
        }
        let author_obj = it.get("author").cloned().unwrap_or(Value::Null);
        let music = it.get("music").cloned().unwrap_or(Value::Null);
        let stats = it.get("stats").cloned().unwrap_or(Value::Null);
        let author = s(&author_obj, "uniqueId");
        let music_id = s(&music, "id");
        let music_title = s(&music, "title");
        let ts = it.get("createTime").and_then(|x| x.as_i64()).unwrap_or(0);
        out.push(Entry {
            id: id.clone(),
            author: author.clone(),
            author_name: s(&author_obj, "nickname"),
            description: s(it, "desc"),
            sound: music_title.clone(),
            sound_author: s(&music, "authorName"),
            sound_url: sound_url(&music_title, &music_id).unwrap_or_default(),
            created: if ts > 0 { fmt_utc(ts) } else { String::new() },
            created_ts: ts,
            duration: it
                .pointer("/video/duration")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
            url: format!(
                "https://www.tiktok.com/@{}/video/{}",
                if author.is_empty() { "_" } else { &author },
                id
            ),
            like_count: u(&stats, "diggCount"),
            comment_count: u(&stats, "commentCount"),
            view_count: u(&stats, "playCount"),
            save_count: u(&stats, "collectCount"),
            local_path: String::new(),
        })
    }
    (out, cursor, has_more)
}

/// O `secUid` do perfil, que é o que a API web pede no lugar do `@usuario`.
pub fn parse_sec_uid(html: &str) -> Option<String> {
    let key = "\"secUid\":\"";
    let at = html.find(key)? + key.len();
    let rest = html.get(at..)?;
    let end = rest.find('"')?;
    let id = rest.get(..end)?.to_string();
    if id.len() < 20 {
        return None;
    }
    Some(id)
}

/// A URL da página da API. `None` quando a fonte não é privada.
pub fn item_list_url(source: &str, sec_uid: &str, cursor: &str, count: u32) -> Option<String> {
    let path = match source {
        // A aba do marcador ("favoritos") é `collect` na API.
        "favorites" => "collect",
        // A aba do coração ("curtidos") é `favorite` na API. Os nomes são
        // trocados mesmo — é herança do app antigo.
        "liked" => "favorite",
        _ => return None,
    };
    if sec_uid.trim().is_empty() {
        return None;
    }
    let cursor = if cursor.trim().is_empty() {
        "0"
    } else {
        cursor.trim()
    };
    Some(format!(
        "https://www.tiktok.com/api/user/{}/item_list/?aid=1988&app_language=en&app_name=tiktok_web\
&browser_language=en-US&browser_name=Mozilla&browser_platform=MacIntel&channel=tiktok_web\
&cookie_enabled=true&count={}&cursor={}&device_platform=web_pc&focus_state=true&from_page=user\
&history_len=3&is_fullscreen=false&is_page_visible=true&os=mac&priority_region=&referer=\
&region=US&screen_height=1080&screen_width=1920&secUid={}&tz_name=UTC&webcast_language=en",
        path,
        count.clamp(1, 30),
        urlencoding::encode(cursor),
        urlencoding::encode(sec_uid.trim())
    ))
}

// ───────────────────────── índice ─────────────────────────

pub const CSV_HEADER: &str =
    "id,autor,nome,descricao,som,autor_do_som,link_do_som,data,duracao,url,curtidas,comentarios,visualizacoes,salvos,arquivo";

pub fn to_csv(entries: &[Entry]) -> String {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for e in entries {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&e.id),
            csv_escape(&e.author),
            csv_escape(&e.author_name),
            csv_escape(&e.description),
            csv_escape(&e.sound),
            csv_escape(&e.sound_author),
            csv_escape(&e.sound_url),
            csv_escape(&e.created),
            e.duration,
            csv_escape(&e.url),
            e.like_count,
            e.comment_count,
            e.view_count,
            e.save_count,
            csv_escape(&e.local_path),
        ));
    }
    out
}

fn index_stem(user: &str, source: &str) -> String {
    sanitize_name(&format!(
        "tiktok-{}-{}",
        user.trim_start_matches('@'),
        source
    ))
}

// ───────────────────────── execução ─────────────────────────

fn handle_of(input: &str) -> String {
    match super::parse_target(input) {
        Some(super::Target::User { name }) => name,
        Some(super::Target::Video {
            user: Some(user), ..
        }) => user,
        _ => input.trim().trim_start_matches('@').to_ascii_lowercase(),
    }
}

async fn list_public(
    input: &str,
    source: &str,
    limit: u32,
    cookies: Option<&std::path::Path>,
) -> Result<Vec<Entry>> {
    let target = super::parse_target(input)
        .ok_or_else(|| anyhow!("não reconheci esse perfil ou coleção: {}", input))?;
    let url = match (source, &target) {
        ("collection", super::Target::Collection { .. }) => super::canonical_url(&target),
        ("collection", _) => {
            return Err(anyhow!(
                "cole a URL da coleção (…/@usuario/collection/nome-id)"
            ))
        }
        _ => super::canonical_url(&super::Target::User {
            name: handle_of(input),
        }),
    };
    let v = super::ytdlp_json(&super::ytdlp_list_args(&url, limit, cookies)).await?;
    Ok(entries_from_list(&v))
}

async fn list_private(
    opts: &Options,
    handle: &str,
    progress: &ProgressFn,
    pacer: &Pacer,
) -> Result<Vec<Entry>> {
    let session = opts
        .session_netscape
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "os favoritos e os curtidos só saem com a sua sessão: capture os cookies do \
                 tiktok.com pela extensão e escolha a conta aqui"
            )
        })?;
    let (client, _) = super::cookie_client(Some(session))?;

    // O `secUid` vem da própria página do perfil, que só abre inteira para
    // quem está logado.
    pacer.wait().await;
    let profile = format!("https://www.tiktok.com/@{}", handle);
    let html = client.get(&profile).send().await?.text().await?;
    let sec_uid = parse_sec_uid(&html).ok_or_else(|| {
        anyhow!(
            "não achei o secUid de @{} na página do perfil — a sessão pode ter expirado",
            handle
        )
    })?;

    let mut out: Vec<Entry> = Vec::new();
    let mut cursor = String::from("0");
    let teto = if opts.limit == 0 {
        u32::MAX
    } else {
        opts.limit
    };
    for _ in 0..200 {
        let url = item_list_url(&opts.source, &sec_uid, &cursor, 30)
            .ok_or_else(|| anyhow!("fonte desconhecida: {}", opts.source))?;
        pacer.wait().await;
        let resp = client.get(&url).header("Referer", &profile).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("a API do TikTok respondeu HTTP {}", resp.status()));
        }
        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Err(anyhow!(
                "a API do TikTok devolveu resposta vazia — normalmente é a sessão expirada ou a \
                 assinatura da requisição que o site passou a exigir"
            ));
        }
        let v: Value = serde_json::from_str(&body)
            .map_err(|_| anyhow!("a resposta da API do TikTok não era JSON"))?;
        let (page, next, has_more) = entries_from_item_list(&v);
        let vazio = page.is_empty();
        for e in page {
            if out.len() as u32 >= teto {
                break;
            }
            if !out.iter().any(|x| x.id == e.id) {
                out.push(e);
            }
        }
        report(
            progress,
            ID,
            "progress",
            out.len() as u64,
            None,
            Some(format!("{} itens", out.len())),
        );
        if vazio || !has_more || next.is_empty() || next == cursor || out.len() as u32 >= teto {
            break;
        }
        cursor = next;
    }
    if out.is_empty() {
        return Err(anyhow!(
            "a lista voltou vazia — o TikTok não entregou os favoritos para esta sessão"
        ));
    }
    Ok(out)
}

pub async fn run(opts: &Options, progress: ProgressFn) -> Result<FavoritesResult> {
    if opts.user.trim().is_empty() {
        return Err(anyhow!("informe o perfil (@usuario) ou a URL da coleção"));
    }
    if opts.dest.trim().is_empty() {
        return Err(anyhow!("escolha a pasta de destino"));
    }
    let dest = PathBuf::from(&opts.dest);
    std::fs::create_dir_all(&dest)?;

    let session = TempCookies::new(opts.session_netscape.as_deref());
    let used_session = session.is_some();
    let cookies: Option<PathBuf> = opts
        .cookies
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| session.path().map(|p| p.to_path_buf()));

    let handle = handle_of(&opts.user);
    let pacer = Pacer::new(opts.delay_ms);
    report(&progress, ID, "started", 0, None, None);

    let mut entries = match opts.source.as_str() {
        "favorites" | "liked" => list_private(opts, &handle, &progress, &pacer).await?,
        other => list_public(&opts.user, other, opts.limit, cookies.as_deref()).await?,
    };
    if opts.limit > 0 && entries.len() > opts.limit as usize {
        entries.truncate(opts.limit as usize);
    }

    let mut files: Vec<String> = Vec::new();
    let mut downloaded = 0usize;
    let mut failed = 0usize;

    if opts.download_media && !entries.is_empty() {
        let media_dir = dest.join(index_stem(&handle, &opts.source));
        std::fs::create_dir_all(&media_dir)?;
        let selector = super::format_selector(false, "best");
        let ffmpeg = crate::core::dependencies::find_tool("ffmpeg").await;
        let total = entries.len() as u64;
        for (i, e) in entries.iter_mut().enumerate() {
            report(
                &progress,
                ID,
                "progress",
                i as u64,
                Some(total),
                Some(e.url.clone()),
            );
            let stem = super::base_name(Some(e.author.as_str()).filter(|a| !a.is_empty()), &e.id);
            if super::has_stem(&media_dir, &stem) {
                continue;
            }
            pacer.wait().await;
            let args = super::ytdlp_args(
                &e.url,
                &media_dir,
                &stem,
                super::Mode::Video {
                    selector: &selector,
                },
                false,
                ffmpeg.as_deref(),
                cookies.as_deref(),
            );
            match super::run_ytdlp(&args, ID, &progress).await {
                Ok((got, _)) if !got.is_empty() => {
                    e.local_path = got[0].clone();
                    files.extend(got);
                    downloaded += 1;
                }
                _ => failed += 1,
            }
        }
    }

    let stem = index_stem(&handle, &opts.source);
    let quer = |k: &str| opts.index_format == k || opts.index_format == "both";
    if quer("csv") {
        let path = dest.join(format!("{}.csv", stem));
        std::fs::write(&path, to_csv(&entries))?;
        files.push(path.to_string_lossy().to_string());
    }
    if quer("json") {
        let path = dest.join(format!("{}.json", stem));
        std::fs::write(&path, serde_json::to_vec_pretty(&entries)?)?;
        files.push(path.to_string_lossy().to_string());
    }

    report(
        &progress,
        ID,
        "done",
        entries.len() as u64,
        Some(entries.len() as u64),
        None,
    );
    Ok(FavoritesResult {
        source: opts.source.clone(),
        user: handle,
        entries,
        files,
        downloaded,
        failed,
        dest: dest.to_string_lossy().to_string(),
        used_session,
        requests: pacer.count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTAGEM: &str = r#"{
      "_type": "playlist",
      "id": "tiktok",
      "entries": [
        {"id":"7683195368279985438","uploader":"tiktok","channel":"TikTok",
         "description":"hear how @Palina spins, tracks","track":"original sound",
         "artists":["TikTok"],"timestamp":1788883336,"duration":68,
         "url":"https://www.tiktok.com/@tiktok/video/7683195368279985438",
         "view_count":80400,"like_count":1586,"comment_count":400,"save_count":140},
        {"id":"7681695065927912735","uploader":"tiktok","channel":"TikTok",
         "description":"segundo","artist":"Outro","timestamp":0,"duration":58},
        {"description":"sem id"}
      ]
    }"#;

    const ITEM_LIST: &str = r#"{
      "statusCode": 0,
      "itemList": [
        {"id":"7683195368279985438","desc":"legenda com, vírgula","createTime":1788883336,
         "author":{"uniqueId":"tiktok","nickname":"TikTok"},
         "music":{"id":"7683266637168610079","title":"original sound","authorName":"TikTok"},
         "video":{"duration":68},
         "stats":{"diggCount":1586,"commentCount":400,"playCount":80400,"collectCount":140}}
      ],
      "cursor": "1788883336000",
      "hasMore": true
    }"#;

    #[test]
    fn le_a_listagem_do_ytdlp() {
        let v: Value = serde_json::from_str(LISTAGEM).unwrap_or(Value::Null);
        let e = entries_from_list(&v);
        assert_eq!(e.len(), 2, "a entrada sem id não vira linha");
        assert_eq!(e[0].id, "7683195368279985438");
        assert_eq!(e[0].author, "tiktok");
        assert_eq!(e[0].author_name, "TikTok");
        assert_eq!(e[0].sound, "original sound");
        assert_eq!(e[0].sound_author, "TikTok");
        assert_eq!(e[0].created, "2026-09-08 16:02 UTC");
        assert_eq!(e[0].view_count, 80400);
        assert_eq!(
            e[0].url,
            "https://www.tiktok.com/@tiktok/video/7683195368279985438"
        );
        // Sem `url` na entrada, o link é montado a partir do autor e do id.
        assert_eq!(
            e[1].url,
            "https://www.tiktok.com/@tiktok/video/7681695065927912735"
        );
        assert_eq!(e[1].sound_author, "Outro", "cai no campo `artist` singular");
        assert_eq!(e[1].created, "");
    }

    #[test]
    fn le_uma_pagina_da_api_web() {
        let v: Value = serde_json::from_str(ITEM_LIST).unwrap_or(Value::Null);
        let (e, cursor, mais) = entries_from_item_list(&v);
        assert_eq!(cursor, "1788883336000");
        assert!(mais);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].author, "tiktok");
        assert_eq!(e[0].description, "legenda com, vírgula");
        assert_eq!(e[0].duration, 68);
        assert_eq!(e[0].save_count, 140);
        assert_eq!(
            e[0].sound_url,
            "https://www.tiktok.com/music/original-sound-7683266637168610079"
        );
    }

    #[test]
    fn pagina_vazia_nao_quebra() {
        let v: Value =
            serde_json::from_str(r#"{"itemList":null,"hasMore":0}"#).unwrap_or(Value::Null);
        let (e, cursor, mais) = entries_from_item_list(&v);
        assert!(e.is_empty());
        assert_eq!(cursor, "");
        assert!(!mais);
    }

    #[test]
    fn monta_o_csv_com_cabecalho_e_escape() {
        let v: Value = serde_json::from_str(ITEM_LIST).unwrap_or(Value::Null);
        let (mut e, _, _) = entries_from_item_list(&v);
        e[0].local_path = "/tmp/tt/tiktok-7683195368279985438.mp4".to_string();
        let csv = to_csv(&e);
        let linhas: Vec<&str> = csv.lines().collect();
        assert_eq!(linhas[0], CSV_HEADER);
        assert_eq!(linhas[0].split(',').count(), 15);
        assert_eq!(linhas.len(), 2);
        assert!(linhas[1].contains("\"legenda com, vírgula\""));
        assert!(linhas[1].ends_with("/tmp/tt/tiktok-7683195368279985438.mp4"));
        assert_eq!(to_csv(&[]).lines().count(), 1);
    }

    #[test]
    fn url_da_api_so_existe_para_fonte_privada() {
        let u =
            item_list_url("favorites", "MS4wLjABAAAAv7iSuuXDJGDvJkmH", "0", 30).unwrap_or_default();
        assert!(u.starts_with("https://www.tiktok.com/api/user/collect/item_list/"));
        assert!(u.contains("secUid=MS4wLjABAAAAv7iSuuXDJGDvJkmH"));
        assert!(u.contains("count=30"));
        assert!(u.contains("cursor=0"));
        let l =
            item_list_url("liked", "MS4wLjABAAAAv7iSuuXDJGDvJkmH", "123", 99).unwrap_or_default();
        assert!(l.contains("/api/user/favorite/item_list/"));
        assert!(l.contains("cursor=123"));
        assert!(l.contains("count=30"), "o count é limitado a 30");
        assert_eq!(item_list_url("posts", "x", "0", 30), None);
        assert_eq!(item_list_url("favorites", "  ", "0", 30), None);
    }

    #[test]
    fn le_o_sec_uid_da_pagina_do_perfil() {
        let html = r#"{"user":{"id":"107955","secUid":"MS4wLjABAAAAv7iSuuXDJGDvJkmH_vz1qkDZYo1apxgzaxdBSeIuPiM","uniqueId":"tiktok"}}"#;
        assert_eq!(
            parse_sec_uid(html).as_deref(),
            Some("MS4wLjABAAAAv7iSuuXDJGDvJkmH_vz1qkDZYo1apxgzaxdBSeIuPiM")
        );
        assert_eq!(parse_sec_uid("<html>nada</html>"), None);
        assert_eq!(parse_sec_uid(r#"{"secUid":"curto"}"#), None);
    }

    #[test]
    fn handle_sai_de_qualquer_forma_de_entrada() {
        assert_eq!(handle_of("@NASA"), "nasa");
        assert_eq!(handle_of("https://www.tiktok.com/@nasa"), "nasa");
        assert_eq!(
            handle_of("https://www.tiktok.com/@nasa/video/7683195368279985438"),
            "nasa"
        );
        assert_eq!(handle_of("nasa"), "nasa");
    }

    #[test]
    fn nome_do_indice_e_seguro() {
        assert_eq!(index_stem("@na/sa", "favorites"), "tiktok-nasa-favorites");
        assert_eq!(index_stem("nasa", "posts"), "tiktok-nasa-posts");
    }

    #[tokio::test]
    #[ignore = "rede: lista os videos publicos de um perfil do TikTok com o yt-dlp"]
    async fn ao_vivo_lista_o_perfil_publico() {
        let e = list_public("https://www.tiktok.com/@tiktok", "posts", 3, None)
            .await
            .expect("a listagem ao vivo falhou");
        assert!(!e.is_empty());
        assert!(e.iter().all(|x| x.id.chars().all(|c| c.is_ascii_digit())));
        assert!(e
            .iter()
            .all(|x| x.url.starts_with("https://www.tiktok.com/@")));
    }
}
