//! `tt-sound`: baixar só o áudio — o "som" — **com a atribuição**.
//!
//! Extrair áudio de vídeo é o fácil. O que faz esta ferramenta valer é o
//! crédito: nome do som, quem o criou e o link da página do som, gravados nas
//! tags do arquivo e num sidecar de texto ao lado. Sem isso é só um extrator
//! de áudio qualquer.
//!
//! De onde sai cada pedaço:
//! - o **nome** do som e o **autor** vêm do `-J` do yt-dlp (`track`, `artist`);
//! - o **id** do som — e portanto o link `tiktok.com/music/<slug>-<id>` — só
//!   existe no HTML da página do vídeo, no objeto `music`. Por isso a tool dá
//!   um GET nessa página; se ele falhar, o crédito sai sem o link em vez de a
//!   ferramenta inteira falhar.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{expand_inputs, short_reason, sound_url, Pacer, TempCookies};
use crate::core::tools::{report, sanitize_name, ProgressFn};

const ID: &str = "tt-sound";

fn def_true() -> bool {
    true
}
fn def_format() -> String {
    "mp3".to_string()
}
fn def_delay() -> u64 {
    1200
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Lista colada: uma URL por linha.
    #[serde(default)]
    pub urls: String,
    /// Arquivo `.txt` com uma URL por linha.
    #[serde(default)]
    pub list_file: Option<String>,
    pub dest: String,
    /// "mp3" | "m4a" | "best".
    #[serde(default = "def_format")]
    pub format: String,
    /// Gravar o crédito nas tags do arquivo.
    #[serde(default = "def_true")]
    pub tag_file: bool,
    /// Gravar o crédito num `.txt` ao lado.
    #[serde(default = "def_true")]
    pub write_sidecar: bool,
    /// Pular o que já está no destino.
    #[serde(default = "def_true")]
    pub skip_existing: bool,
    #[serde(default = "def_delay")]
    pub delay_ms: u64,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
    #[serde(skip)]
    pub session_netscape: Option<String>,
}

/// O crédito de um som. É o coração da ferramenta.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Attribution {
    /// Nome do som ("original sound", "Beat do X"…).
    pub sound: String,
    /// Quem assina o som (nem sempre é quem postou o vídeo).
    pub sound_author: String,
    /// `tiktok.com/music/<slug>-<id>`, quando o id foi encontrado.
    pub sound_url: String,
    pub sound_id: String,
    /// Se o som é original do vídeo ou pegado de outro.
    pub original: bool,
    /// Quem postou o vídeo.
    pub video_author: String,
    pub video_title: String,
    pub video_url: String,
    pub video_id: String,
    pub created: String,
    pub duration: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundItem {
    pub url: String,
    /// "ok" | "skipped" | "failed"
    pub status: String,
    pub reason: String,
    pub file: String,
    pub sidecar: String,
    pub tagged: bool,
    pub attribution: Attribution,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundResult {
    pub items: Vec<SoundItem>,
    pub ok: usize,
    pub failed: usize,
    pub skipped: usize,
    pub dest: String,
    pub used_session: bool,
}

// ───────────────────────── crédito ─────────────────────────

fn s(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Monta o crédito com o que o yt-dlp devolveu e, quando ela existe, com a
/// leitura do objeto `music` da página (que é quem tem o id do som).
pub fn attribution_from(meta: &serde_json::Value, music: Option<&super::Music>) -> Attribution {
    let track = s(meta, "track");
    let artist = meta
        .get("artists")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .unwrap_or_else(|| s(meta, "artist"));
    let ts = meta.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0);
    let sound = music
        .map(|m| m.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or(track);
    let sound_author = music
        .map(|m| m.author.clone())
        .filter(|a| !a.is_empty())
        .unwrap_or(artist);
    let sound_id = music.map(|m| m.id.clone()).unwrap_or_default();
    Attribution {
        sound_url: sound_url(&sound, &sound_id).unwrap_or_default(),
        sound,
        sound_author,
        sound_id,
        original: music.map(|m| m.original).unwrap_or(false),
        video_author: s(meta, "uploader"),
        video_title: s(meta, "title"),
        video_url: s(meta, "webpage_url"),
        video_id: s(meta, "id"),
        created: if ts > 0 {
            super::fmt_utc(ts)
        } else {
            String::new()
        },
        duration: meta.get("duration").and_then(|x| x.as_u64()).unwrap_or(0),
    }
}

/// O sidecar de texto: o crédito escrito para uma pessoa ler e copiar.
pub fn sidecar_text(a: &Attribution) -> String {
    let mut out = String::new();
    let linha = |out: &mut String, rotulo: &str, valor: &str| {
        if !valor.trim().is_empty() {
            out.push_str(rotulo);
            out.push_str(": ");
            out.push_str(valor.trim());
            out.push('\n');
        }
    };
    out.push_str("Crédito do som\n");
    out.push_str("==============\n");
    linha(&mut out, "Som", &a.sound);
    linha(&mut out, "Autor do som", &a.sound_author);
    linha(&mut out, "Link do som", &a.sound_url);
    linha(
        &mut out,
        "Tipo",
        if a.original {
            "som original do vídeo"
        } else {
            "som pego de outro vídeo"
        },
    );
    out.push('\n');
    linha(&mut out, "Vídeo", &a.video_title);
    linha(&mut out, "Postado por", &a.video_author);
    linha(&mut out, "Link do vídeo", &a.video_url);
    linha(&mut out, "Data", &a.created);
    if a.duration > 0 {
        out.push_str(&format!("Duração: {}s\n", a.duration));
    }
    out.push_str("\nBaixado com o OmniGet. Dê o crédito ao usar.\n");
    out
}

/// A mesma coisa em uma linha, para caber no campo de comentário da tag.
pub fn comment_line(a: &Attribution) -> String {
    let mut partes: Vec<String> = Vec::new();
    if !a.sound.is_empty() {
        partes.push(format!("Som: {}", a.sound));
    }
    if !a.sound_author.is_empty() {
        partes.push(format!("por {}", a.sound_author));
    }
    if !a.sound_url.is_empty() {
        partes.push(a.sound_url.clone());
    }
    if !a.video_url.is_empty() {
        partes.push(format!("vídeo: {}", a.video_url));
    }
    partes.join(" · ")
}

/// Nome de arquivo do som: `<autor>-<id>-<som>`.
pub fn sound_base(a: &Attribution) -> String {
    let autor = if a.video_author.is_empty() {
        "tiktok".to_string()
    } else {
        a.video_author.clone()
    };
    let som = if a.sound.is_empty() {
        "som".to_string()
    } else {
        a.sound.clone()
    };
    let som: String = som.chars().take(60).collect();
    sanitize_name(&format!("{}-{}-{}", autor, a.video_id, som.trim()))
}

/// Grava o crédito nas tags do arquivo de áudio.
fn tag_audio(path: &Path, a: &Attribution) -> Result<()> {
    use lofty::config::WriteOptions;
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::prelude::{Accessor, ItemKey};
    use lofty::tag::Tag;

    let mut tagged = lofty::read_from_path(path).map_err(|e| anyhow!(e.to_string()))?;
    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| anyhow!("o formato não aceita a tag {:?}", tag_type))?;

    if !a.sound.is_empty() {
        tag.set_title(a.sound.clone());
    }
    if !a.sound_author.is_empty() {
        tag.set_artist(a.sound_author.clone());
    }
    if !a.video_author.is_empty() {
        let _ = tag.insert_text(ItemKey::AlbumArtist, a.video_author.clone());
    }
    tag.set_album("TikTok".to_string());
    tag.set_genre("TikTok".to_string());
    let comentario = comment_line(a);
    if !comentario.is_empty() {
        tag.set_comment(comentario);
    }
    let fonte = if a.sound_url.is_empty() {
        a.video_url.clone()
    } else {
        a.sound_url.clone()
    };
    if !fonte.is_empty() {
        let _ = tag.insert_text(ItemKey::AudioFileUrl, fonte);
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| anyhow!(e.to_string()))?;
    Ok(())
}

// ───────────────────────── execução ─────────────────────────

/// Busca o objeto `music` na página do vídeo. Melhor esforço: qualquer
/// tropeço devolve `None` e o crédito segue com o que o yt-dlp deu.
async fn fetch_music(client: &reqwest::Client, url: &str, pacer: &Pacer) -> Option<super::Music> {
    pacer.wait().await;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let html = resp.text().await.ok()?;
    super::parse_music(&html)
}

pub async fn run(opts: &Options, progress: ProgressFn) -> Result<SoundResult> {
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

    let list_text = match opts.list_file.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| anyhow!("não consegui ler a lista {}: {}", path, e))?,
        None => String::new(),
    };
    let mut queue = expand_inputs(&opts.urls);
    for url in expand_inputs(&list_text) {
        if !queue.contains(&url) {
            queue.push(url);
        }
    }
    if queue.is_empty() {
        return Err(anyhow!("nenhum link de vídeo do TikTok na entrada"));
    }

    let audio_format = match opts.format.as_str() {
        "m4a" => "m4a",
        "best" => "best",
        _ => "mp3",
    };
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg").await;
    let (client, _) = super::cookie_client(opts.session_netscape.as_deref())?;
    let pacer = Pacer::new(opts.delay_ms);
    let total = queue.len() as u64;
    let mut items: Vec<SoundItem> = Vec::with_capacity(queue.len());

    for (i, url) in queue.iter().enumerate() {
        report(
            &progress,
            ID,
            "progress",
            i as u64,
            Some(total),
            Some(url.clone()),
        );
        let falha = |motivo: String| SoundItem {
            url: url.clone(),
            status: "failed".to_string(),
            reason: motivo,
            file: String::new(),
            sidecar: String::new(),
            tagged: false,
            attribution: Attribution::default(),
        };

        // 1. metadados (nome do som, autor, data)
        pacer.wait().await;
        let mut meta_args = vec![
            "--ignore-config".to_string(),
            "--no-warnings".to_string(),
            "--no-playlist".to_string(),
            "--skip-download".to_string(),
            "-J".to_string(),
        ];
        if let Some(c) = cookies.as_deref() {
            meta_args.push("--cookies".to_string());
            meta_args.push(c.to_string_lossy().to_string());
        }
        meta_args.push(url.clone());
        let meta = match super::ytdlp_json(&meta_args).await {
            Ok(v) => v,
            Err(e) => {
                items.push(falha(e.to_string()));
                continue;
            }
        };

        // 2. o id do som, que só a página tem
        let pagina = match meta.get("webpage_url").and_then(|x| x.as_str()) {
            Some(u) if u.starts_with("http") => u.to_string(),
            _ => url.clone(),
        };
        let music = fetch_music(&client, &pagina, &pacer).await;
        let attribution = attribution_from(&meta, music.as_ref());
        let base = sound_base(&attribution);

        if opts.skip_existing && super::has_stem(&dest, &base) {
            items.push(SoundItem {
                url: url.clone(),
                status: "skipped".to_string(),
                reason: "já está na pasta".to_string(),
                file: String::new(),
                sidecar: String::new(),
                tagged: false,
                attribution,
            });
            continue;
        }

        // 3. o áudio
        pacer.wait().await;
        let args = super::ytdlp_args(
            url,
            &dest,
            &base,
            super::Mode::Audio {
                format: audio_format,
            },
            false,
            ffmpeg.as_deref(),
            cookies.as_deref(),
        );
        let (files, tail) = match super::run_ytdlp(&args, ID, &progress).await {
            Ok(v) => v,
            Err(e) => {
                items.push(falha(e.to_string()));
                continue;
            }
        };
        let Some(file) = files.first().cloned() else {
            items.push(falha(short_reason(&tail)));
            continue;
        };

        // 4. o crédito
        let mut tagged = false;
        let mut aviso = String::new();
        if opts.tag_file {
            match tag_audio(Path::new(&file), &attribution) {
                Ok(()) => tagged = true,
                Err(e) => aviso = format!("o áudio saiu, mas as tags não: {}", e),
            }
        }
        let mut sidecar = String::new();
        if opts.write_sidecar {
            let path = dest.join(format!("{}.credito.txt", base));
            match std::fs::write(&path, sidecar_text(&attribution)) {
                Ok(()) => sidecar = path.to_string_lossy().to_string(),
                Err(e) => aviso = format!("o sidecar de crédito não foi gravado: {}", e),
            }
        }
        items.push(SoundItem {
            url: url.clone(),
            status: "ok".to_string(),
            reason: aviso,
            file,
            sidecar,
            tagged,
            attribution,
        });
    }

    let ok = items.iter().filter(|i| i.status == "ok").count();
    let failed = items.iter().filter(|i| i.status == "failed").count();
    let skipped = items.iter().filter(|i| i.status == "skipped").count();
    report(&progress, ID, "done", total, Some(total), None);
    Ok(SoundResult {
        items,
        ok,
        failed,
        skipped,
        dest: dest.to_string_lossy().to_string(),
        used_session,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta() -> serde_json::Value {
        json!({
            "id": "7683195368279985438",
            "title": "hear how @Palina spins",
            "uploader": "tiktok",
            "track": "original sound",
            "artists": ["TikTok"],
            "timestamp": 1788883336,
            "duration": 68,
            "webpage_url": "https://www.tiktok.com/@tiktok/video/7683195368279985438"
        })
    }

    fn music() -> super::super::Music {
        super::super::Music {
            id: "7683266637168610079".to_string(),
            title: "original sound".to_string(),
            author: "TikTok".to_string(),
            original: true,
            duration: 68,
            play_url: "https://v58.tiktokcdn.com/x".to_string(),
        }
    }

    #[test]
    fn credito_completo_quando_a_pagina_deu_o_id_do_som() {
        let a = attribution_from(&meta(), Some(&music()));
        assert_eq!(a.sound, "original sound");
        assert_eq!(a.sound_author, "TikTok");
        assert_eq!(a.sound_id, "7683266637168610079");
        assert_eq!(
            a.sound_url,
            "https://www.tiktok.com/music/original-sound-7683266637168610079"
        );
        assert!(a.original);
        assert_eq!(a.video_author, "tiktok");
        assert_eq!(a.created, "2026-09-08 16:02 UTC");
        assert_eq!(a.duration, 68);
    }

    #[test]
    fn sem_a_pagina_o_credito_sai_sem_link_mas_com_nome_e_autor() {
        let a = attribution_from(&meta(), None);
        assert_eq!(a.sound, "original sound");
        assert_eq!(a.sound_author, "TikTok");
        assert_eq!(a.sound_url, "");
        assert_eq!(a.sound_id, "");
        assert!(!a.original);
    }

    #[test]
    fn artista_cai_no_campo_singular_quando_nao_ha_lista() {
        let m = json!({"id":"1","track":"beat","artist":"Fulano"});
        let a = attribution_from(&m, None);
        assert_eq!(a.sound_author, "Fulano");
    }

    #[test]
    fn sidecar_traz_som_autor_e_link() {
        let texto = sidecar_text(&attribution_from(&meta(), Some(&music())));
        assert!(texto.starts_with("Crédito do som"));
        assert!(texto.contains("Som: original sound"));
        assert!(texto.contains("Autor do som: TikTok"));
        assert!(texto.contains(
            "Link do som: https://www.tiktok.com/music/original-sound-7683266637168610079"
        ));
        assert!(texto.contains("Tipo: som original do vídeo"));
        assert!(texto
            .contains("Link do vídeo: https://www.tiktok.com/@tiktok/video/7683195368279985438"));
        assert!(texto.contains("Duração: 68s"));
    }

    #[test]
    fn sidecar_pula_o_que_esta_vazio() {
        let texto = sidecar_text(&Attribution {
            sound: "x".to_string(),
            ..Default::default()
        });
        assert!(texto.contains("Som: x"));
        assert!(!texto.contains("Link do som"));
        assert!(!texto.contains("Autor do som"));
        assert!(!texto.contains("Duração"));
    }

    #[test]
    fn comentario_de_tag_cabe_em_uma_linha() {
        let c = comment_line(&attribution_from(&meta(), Some(&music())));
        assert_eq!(
            c,
            "Som: original sound · por TikTok · \
             https://www.tiktok.com/music/original-sound-7683266637168610079 · \
             vídeo: https://www.tiktok.com/@tiktok/video/7683195368279985438"
        );
        assert!(!c.contains('\n'));
        assert_eq!(comment_line(&Attribution::default()), "");
    }

    #[test]
    fn nome_do_arquivo_junta_autor_id_e_som() {
        let a = attribution_from(&meta(), Some(&music()));
        assert_eq!(sound_base(&a), "tiktok-7683195368279985438-original sound");
        let vazio = sound_base(&Attribution::default());
        assert_eq!(vazio, "tiktok--som");
        let longo = sound_base(&Attribution {
            video_author: "a".to_string(),
            video_id: "1".to_string(),
            sound: "s".repeat(200),
            ..Default::default()
        });
        assert!(
            longo.len() <= 70,
            "o nome do som é cortado: {}",
            longo.len()
        );
    }

    #[tokio::test]
    #[ignore = "rede: baixa o som de um video publico do TikTok e confere a atribuicao"]
    async fn ao_vivo_baixa_o_som_com_credito() {
        let dir = crate::core::tools::temp_dir().join("tt-live-sound");
        let _ = std::fs::create_dir_all(&dir);
        let opts = Options {
            urls: "https://www.tiktok.com/@tiktok/video/7683195368279985438".to_string(),
            list_file: None,
            dest: dir.to_string_lossy().to_string(),
            format: "mp3".to_string(),
            tag_file: true,
            write_sidecar: true,
            skip_existing: false,
            delay_ms: 1200,
            cookies: None,
            account_slug: None,
            session_netscape: None,
        };
        let r = run(&opts, crate::core::tools::noop_progress())
            .await
            .expect("o download ao vivo falhou");
        assert_eq!(r.failed, 0, "{:?}", r.items);
        let item = r.items.first().cloned().unwrap_or_else(|| SoundItem {
            url: String::new(),
            status: "failed".to_string(),
            reason: "sem item".to_string(),
            file: String::new(),
            sidecar: String::new(),
            tagged: false,
            attribution: Attribution::default(),
        });
        assert!(!item.attribution.sound.is_empty());
        assert!(
            !item.attribution.sound_url.is_empty(),
            "o link do som é o ponto da tool"
        );
        assert!(item.tagged);
        assert!(Path::new(&item.sidecar).exists());
    }
}
