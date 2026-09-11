//! SponsorBlock (estudo 43): segmentos por prefixo de hash, como a extensão
//! faz para não mandar o ID do vídeo ao servidor.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SERVER: &str = "https://sponsor.ajay.app";
pub const CATEGORIES: &[&str] = &[
    "sponsor",
    "selfpromo",
    "interaction",
    "intro",
    "outro",
    "preview",
    "music_offtopic",
    "filler",
    "exclusive_access",
    "poi_highlight",
    "chapter",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(rename = "UUID", default)]
    pub uuid: String,
    pub segment: [f64; 2],
    pub category: String,
    #[serde(rename = "actionType", default)]
    pub action_type: String,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub locked: i64,
    #[serde(rename = "videoDuration", default)]
    pub video_duration: f64,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SponsorResult {
    pub video_id: String,
    pub segments: Vec<Segment>,
    pub skipped_seconds: f64,
    /// Argumentos equivalentes do yt-dlp para baixar sem os trechos.
    pub ytdlp_args: String,
}

/// Aceita URL completa, `youtu.be/ID`, `shorts/ID` ou o próprio ID.
pub fn video_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.len() == 11
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some(s.to_string());
    }
    let re =
        regex::Regex::new(r"(?:v=|youtu\.be/|shorts/|embed/|live/)([A-Za-z0-9_-]{11})").ok()?;
    re.captures(s)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub async fn segments(input: &str, categories: &[String]) -> anyhow::Result<SponsorResult> {
    let id = video_id(input)
        .ok_or_else(|| anyhow!("nao reconheci um video do YouTube em: {}", input))?;
    let cats: Vec<&str> = if categories.is_empty() {
        CATEGORIES.to_vec()
    } else {
        categories.iter().map(|s| s.as_str()).collect()
    };
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    let prefix = &hex::encode(h.finalize())[..4];
    let url = format!(
        "{}/api/skipSegments/{}?categories={}&actionTypes={}",
        SERVER,
        prefix,
        urlencoding::encode(&serde_json::to_string(&cats)?),
        urlencoding::encode(r#"["skip","mute","full","poi","chapter"]"#)
    );
    let client = super::client()?;
    let resp = client.get(&url).send().await?;
    let mut segs: Vec<Segment> = Vec::new();
    if resp.status().as_u16() == 404 {
        // sem segmentos para esse prefixo
    } else if resp.status().is_success() {
        let arr: Vec<serde_json::Value> = resp.json().await?;
        for v in arr {
            if v["videoID"].as_str() == Some(id.as_str()) {
                if let Some(list) = v["segments"].as_array() {
                    segs.extend(
                        list.iter()
                            .filter_map(|s| serde_json::from_value(s.clone()).ok()),
                    );
                }
            }
        }
    } else {
        return Err(anyhow!("SponsorBlock: HTTP {}", resp.status()));
    }
    segs.sort_by(|a, b| {
        a.segment[0]
            .partial_cmp(&b.segment[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let skipped: f64 = segs
        .iter()
        .filter(|s| s.action_type == "skip" || s.action_type.is_empty())
        .map(|s| (s.segment[1] - s.segment[0]).max(0.0))
        .sum();
    let mut used: Vec<&str> = segs
        .iter()
        .filter(|s| s.action_type != "chapter" && s.action_type != "poi")
        .map(|s| s.category.as_str())
        .collect();
    used.sort();
    used.dedup();
    let ytdlp_args = if used.is_empty() {
        String::new()
    } else {
        format!("--sponsorblock-remove {}", used.join(","))
    };
    Ok(SponsorResult {
        video_id: id,
        segments: segs,
        skipped_seconds: skipped,
        ytdlp_args,
    })
}

// ── Envio de segmentos ─────────────────────────────────────────────────
//
// O SponsorBlockServer é AGPL: nada dele entra aqui. O que existe é a API
// pública `POST /api/skipSegments`, a mesma que a extensão usa.
//
// O `userID` é uma chave privada aleatória guardada na máquina, e só. O
// servidor deriva dela o identificador público — por isso ela nunca pode
// nascer do nome, do e-mail ou do hardware de quem usa o app.

pub const USER_AGENT_TAG: &str = concat!("OmniGet/", env!("CARGO_PKG_VERSION"));
pub const USER_FILE: &str = "sponsorblock-user.txt";
/// Um envio de uma vez só; acima disso o servidor devolve limite de taxa.
pub const MAX_SEGMENTS_PER_SUBMIT: usize = 10;
pub const ACTION_TYPES: &[&str] = &["skip", "mute", "full", "poi", "chapter"];

/// Chave privada nova: 256 bits de aleatório em hexadecimal. Não deriva de
/// nada da máquina nem da conta.
pub fn generate_private_id() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// SHA-256 da chave privada, só para a UI mostrar um identificador estável
/// sem exibir a chave. O identificador público de verdade é derivado pelo
/// servidor do SponsorBlock, não por aqui.
pub fn public_fingerprint(private: &str) -> String {
    let mut h = Sha256::new();
    h.update(private.as_bytes());
    hex::encode(h.finalize())
}

fn user_file() -> anyhow::Result<std::path::PathBuf> {
    let dir = super::tools_dir().ok_or_else(|| anyhow!("não achei a pasta de dados do app"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(USER_FILE))
}

/// A chave privada da máquina, criada na primeira vez e reusada depois.
pub fn local_user_id() -> anyhow::Result<String> {
    let path = user_file()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if existing.len() >= 30 {
            return Ok(existing);
        }
    }
    let fresh = generate_private_id();
    std::fs::write(&path, fresh.as_bytes())?;
    Ok(fresh)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct NewSegment {
    pub start: f64,
    pub end: f64,
    pub category: String,
    #[serde(default = "default_action")]
    pub action_type: String,
    /// Só o capítulo usa: é o título que aparece na barra.
    #[serde(default)]
    pub description: String,
}

fn default_action() -> String {
    "skip".to_string()
}

/// Recusa o que o servidor recusaria, com a mensagem em português e antes de
/// gastar um envio (o limite de taxa é por conta, não por tentativa válida).
pub fn validate_segments(segments: &[NewSegment], duration: f64) -> anyhow::Result<()> {
    if segments.is_empty() {
        return Err(anyhow!("marque pelo menos um trecho antes de enviar"));
    }
    if segments.len() > MAX_SEGMENTS_PER_SUBMIT {
        return Err(anyhow!(
            "no máximo {} trechos por envio",
            MAX_SEGMENTS_PER_SUBMIT
        ));
    }
    for (i, s) in segments.iter().enumerate() {
        let n = i + 1;
        if !CATEGORIES.contains(&s.category.as_str()) {
            return Err(anyhow!(
                "trecho {}: categoria desconhecida ({})",
                n,
                s.category
            ));
        }
        if !ACTION_TYPES.contains(&s.action_type.as_str()) {
            return Err(anyhow!(
                "trecho {}: tipo de ação desconhecido ({})",
                n,
                s.action_type
            ));
        }
        if !s.start.is_finite() || !s.end.is_finite() || s.start < 0.0 || s.end < 0.0 {
            return Err(anyhow!("trecho {}: tempo inválido", n));
        }
        let pontual = s.action_type == "poi" || s.action_type == "full";
        if !pontual && s.end <= s.start {
            return Err(anyhow!("trecho {}: o fim tem de vir depois do começo", n));
        }
        if s.action_type == "chapter" && s.description.trim().is_empty() {
            return Err(anyhow!("trecho {}: capítulo precisa de um título", n));
        }
        if duration > 0.0 && s.end > duration + 1.0 {
            return Err(anyhow!("trecho {}: passa do fim do vídeo", n));
        }
    }
    // Sobreposição só importa entre trechos que pulam pedaço.
    let mut cortes: Vec<&NewSegment> = segments
        .iter()
        .filter(|s| s.action_type == "skip" || s.action_type == "mute")
        .collect();
    cortes.sort_by(|a, b| a.start.total_cmp(&b.start));
    for par in cortes.windows(2) {
        if par[1].start < par[0].end {
            return Err(anyhow!("dois trechos se sobrepõem em {:.1}s", par[1].start));
        }
    }
    Ok(())
}

/// O corpo exato do `POST /api/skipSegments`.
pub fn build_payload(
    video_id: &str,
    user_id: &str,
    duration: f64,
    segments: &[NewSegment],
) -> anyhow::Result<serde_json::Value> {
    if video_id.len() != 11 {
        return Err(anyhow!("não reconheci o vídeo do YouTube"));
    }
    if user_id.trim().len() < 30 {
        return Err(anyhow!("a chave local do SponsorBlock está corrompida"));
    }
    validate_segments(segments, duration)?;
    let segs: Vec<serde_json::Value> = segments
        .iter()
        .map(|s| {
            let mut o = serde_json::json!({
                "segment": [s.start, s.end],
                "category": s.category,
                "actionType": s.action_type,
            });
            if !s.description.trim().is_empty() {
                o["description"] = serde_json::Value::String(s.description.trim().to_string());
            }
            o
        })
        .collect();
    let mut body = serde_json::json!({
        "videoID": video_id,
        "userID": user_id.trim(),
        "userAgent": USER_AGENT_TAG,
        "segments": segs,
    });
    if duration > 0.0 {
        body["videoDuration"] = serde_json::json!(duration);
    }
    Ok(body)
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitOptions {
    pub url: String,
    pub segments: Vec<NewSegment>,
    #[serde(default)]
    pub video_duration: f64,
    /// A UI tem de marcar isto depois de o usuário confirmar: o envio é
    /// público e muda o vídeo para todo mundo.
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitResult {
    pub video_id: String,
    pub accepted: usize,
    pub uuids: Vec<String>,
    pub fingerprint: String,
    pub message: String,
}

/// Traduz o que o servidor devolve para algo que dá para ler na tela.
pub fn submit_error(status: u16, body: &str) -> String {
    let corpo = body.trim();
    let curto = if corpo.len() > 200 {
        &corpo[..200]
    } else {
        corpo
    };
    match status {
        400 => format!("o SponsorBlock recusou o envio: {}", curto),
        403 => format!("envio bloqueado pelo SponsorBlock: {}", curto),
        409 => "esse trecho já tinha sido enviado".to_string(),
        429 => "muitos envios seguidos; espere alguns minutos e tente de novo".to_string(),
        _ => format!("SponsorBlock: HTTP {} {}", status, curto),
    }
}

pub async fn submit(opts: SubmitOptions) -> anyhow::Result<SubmitResult> {
    if !opts.confirmed {
        return Err(anyhow!(
            "o envio é público e vale para todo mundo: confirme antes"
        ));
    }
    let id = video_id(&opts.url)
        .ok_or_else(|| anyhow!("nao reconheci um video do YouTube em: {}", opts.url))?;
    let user = local_user_id()?;
    let body = build_payload(&id, &user, opts.video_duration, &opts.segments)?;

    let client = super::client()?;
    let resp = client
        .post(format!("{}/api/skipSegments", SERVER))
        .json(&body)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(anyhow!("{}", submit_error(status, &text)));
    }
    let uuids: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(&text)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v["UUID"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Ok(SubmitResult {
        accepted: if uuids.is_empty() {
            opts.segments.len()
        } else {
            uuids.len()
        },
        video_id: id,
        uuids,
        fingerprint: public_fingerprint(&user),
        message: text.trim().chars().take(200).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::video_id;

    #[test]
    fn extracts_ids() {
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(video_id("dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("https://example.com"), None);
    }
}

#[cfg(test)]
mod submit_tests {
    use super::*;

    fn seg(start: f64, end: f64, cat: &str) -> NewSegment {
        NewSegment {
            start,
            end,
            category: cat.to_string(),
            action_type: "skip".to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn the_private_key_is_random_hex_and_leaks_nothing() {
        let a = generate_private_id();
        let b = generate_private_id();
        assert_eq!(a.len(), 64, "256 bits em hexadecimal");
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b, "cada chave nasce sozinha");
        let baixo = a.to_lowercase();
        for var in ["USER", "USERNAME", "LOGNAME", "HOME", "HOSTNAME"] {
            if let Ok(v) = std::env::var(var) {
                let v = v.trim().to_lowercase();
                if v.len() >= 3 {
                    assert!(
                        !baixo.contains(&v),
                        "a chave não pode carregar {} da máquina",
                        var
                    );
                }
            }
        }
    }

    #[test]
    fn the_fingerprint_is_a_stable_sha256() {
        assert_eq!(
            public_fingerprint("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let k = generate_private_id();
        assert_eq!(public_fingerprint(&k), public_fingerprint(&k));
        assert_ne!(public_fingerprint(&k), k, "a impressão não devolve a chave");
    }

    #[test]
    fn the_payload_is_what_the_api_expects() {
        let chave = "0".repeat(64);
        let body = build_payload("dQw4w9WgXcQ", &chave, 212.0, &[seg(30.0, 45.5, "sponsor")])
            .expect("payload válido");
        assert_eq!(body["videoID"], "dQw4w9WgXcQ");
        assert_eq!(body["userID"], chave.as_str());
        assert_eq!(body["videoDuration"], 212.0);
        assert!(body["userAgent"]
            .as_str()
            .unwrap_or_default()
            .starts_with("OmniGet/"));
        let s = &body["segments"][0];
        assert_eq!(s["segment"][0], 30.0);
        assert_eq!(s["segment"][1], 45.5);
        assert_eq!(s["category"], "sponsor");
        assert_eq!(s["actionType"], "skip");
        assert!(s.get("description").is_none(), "descrição vazia não vai");
    }

    #[test]
    fn the_payload_carries_nothing_that_identifies_the_person() {
        let chave = "a".repeat(64);
        let body = build_payload("dQw4w9WgXcQ", &chave, 0.0, &[seg(1.0, 2.0, "intro")])
            .expect("payload válido");
        let obj = body.as_object().expect("objeto");
        let mut chaves: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        chaves.sort_unstable();
        assert_eq!(
            chaves,
            vec!["segments", "userAgent", "userID", "videoID"],
            "nenhum campo além dos quatro do protocolo"
        );
        assert!(
            body.get("videoDuration").is_none(),
            "duração 0 não vai junto"
        );
        let texto = body.to_string();
        for var in ["USER", "USERNAME", "LOGNAME", "HOSTNAME"] {
            if let Ok(v) = std::env::var(var) {
                if v.trim().len() >= 3 {
                    assert!(!texto.contains(v.trim()), "o corpo vazou {}", var);
                }
            }
        }
    }

    #[test]
    fn a_chapter_needs_a_title() {
        let mut s = seg(10.0, 20.0, "chapter");
        s.action_type = "chapter".into();
        assert!(validate_segments(&[s.clone()], 100.0).is_err());
        s.description = "Receita".into();
        assert!(validate_segments(&[s.clone()], 100.0).is_ok());
        let body =
            build_payload("dQw4w9WgXcQ", &"1".repeat(64), 100.0, &[s]).expect("payload válido");
        assert_eq!(body["segments"][0]["description"], "Receita");
    }

    #[test]
    fn bad_segments_are_refused_before_the_network() {
        assert!(validate_segments(&[], 100.0).is_err(), "lista vazia");
        assert!(
            validate_segments(&[seg(50.0, 20.0, "sponsor")], 100.0).is_err(),
            "fim antes do começo"
        );
        assert!(
            validate_segments(&[seg(10.0, 20.0, "propaganda")], 100.0).is_err(),
            "categoria inventada"
        );
        assert!(
            validate_segments(&[seg(10.0, 500.0, "sponsor")], 100.0).is_err(),
            "passa do fim do vídeo"
        );
        assert!(
            validate_segments(
                &[seg(10.0, 30.0, "sponsor"), seg(20.0, 40.0, "intro")],
                100.0
            )
            .is_err(),
            "sobreposição"
        );
        let muitos: Vec<NewSegment> = (0..MAX_SEGMENTS_PER_SUBMIT + 1)
            .map(|i| seg(i as f64 * 10.0, i as f64 * 10.0 + 5.0, "sponsor"))
            .collect();
        assert!(
            validate_segments(&muitos, 1000.0).is_err(),
            "envio grande demais"
        );
    }

    #[test]
    fn a_point_of_interest_may_have_no_length() {
        let mut s = seg(42.0, 42.0, "poi_highlight");
        s.action_type = "poi".into();
        assert!(validate_segments(&[s], 100.0).is_ok());
    }

    #[test]
    fn a_broken_video_or_key_never_becomes_a_payload() {
        assert!(build_payload("curto", &"0".repeat(64), 10.0, &[seg(1.0, 2.0, "intro")]).is_err());
        assert!(build_payload("dQw4w9WgXcQ", "curta", 10.0, &[seg(1.0, 2.0, "intro")]).is_err());
    }

    #[test]
    fn server_errors_become_readable_messages() {
        assert!(submit_error(429, "").contains("espere"));
        assert!(submit_error(409, "").contains("já tinha sido"));
        assert!(submit_error(403, "banned").contains("bloqueado"));
        assert!(submit_error(400, "Duplicate segment").contains("Duplicate segment"));
        assert!(submit_error(500, "boom").contains("500"));
        let enorme = "x".repeat(1000);
        assert!(
            submit_error(400, &enorme).len() < 300,
            "mensagem não pode ser um muro"
        );
    }

    #[tokio::test]
    async fn nothing_is_sent_without_an_explicit_confirmation() {
        let e = submit(SubmitOptions {
            url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            segments: vec![seg(10.0, 20.0, "sponsor")],
            video_duration: 100.0,
            confirmed: false,
        })
        .await
        .expect_err("sem confirmação não sai");
        assert!(e.to_string().contains("confirme"));
    }
}
