//! Tradução de legendas em lote (estudos 08, 12, 13): pelo LLM configurado
//! em `core/ai.rs` (prompt "faithfulness" do VideoLingo, com contexto) ou por
//! um servidor LibreTranslate. O alinhamento é por índice: cada cue volta com
//! o mesmo número, então nunca desloca a legenda.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::core::subtitle_merge::Cue;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Translator {
    /// Provedor de IA já configurado no OmniGet (OpenAI, Anthropic, local).
    Llm,
    LibreTranslate {
        base_url: String,
        #[serde(default)]
        api_key: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranslateOptions {
    pub translator: Translator,
    pub source_lang: String,
    pub target_lang: String,
    /// Contexto extra (tema do vídeo, glossário) que entra no prompt do LLM.
    #[serde(default)]
    pub context: String,
    #[serde(default = "default_batch")]
    pub batch_size: usize,
}

fn default_batch() -> usize {
    25
}

#[derive(Debug, Clone, Serialize)]
pub struct TranslateResult {
    pub cues: Vec<Cue>,
    pub failed: Vec<usize>,
}

fn llm_prompt(
    lines: &[&str],
    src: &str,
    tgt: &str,
    context: &str,
    prev: &str,
    next: &str,
) -> (String, String) {
    let system = format!(
        "You are a professional subtitle translator from {src} to {tgt}. Translate each numbered line faithfully, keeping meaning, tone and terminology. Keep each line a subtitle: short, no explanations, no empty lines. Reply ONLY with a JSON object mapping the line number (as string) to the translated text."
    );
    let mut user = String::new();
    if !context.trim().is_empty() {
        user.push_str("<context>\n");
        user.push_str(context.trim());
        user.push_str("\n</context>\n");
    }
    if !prev.is_empty() {
        user.push_str(&format!("<previous>\n{}\n</previous>\n", prev));
    }
    user.push_str("<subtitles>\n");
    for (i, l) in lines.iter().enumerate() {
        user.push_str(&format!("{}: {}\n", i + 1, l));
    }
    user.push_str("</subtitles>\n");
    if !next.is_empty() {
        user.push_str(&format!("<next>\n{}\n</next>\n", next));
    }
    user.push_str("Output JSON like {\"1\": \"...\", \"2\": \"...\"}.");
    (system, user)
}

fn parse_llm_json(text: &str, n: usize) -> Option<Vec<Option<String>>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let obj = v.as_object()?;
    let mut out = vec![None; n];
    for (k, val) in obj {
        if let (Ok(i), Some(s)) = (k.trim().parse::<usize>(), val.as_str()) {
            if i >= 1 && i <= n {
                out[i - 1] = Some(s.trim().to_string());
            }
        }
    }
    Some(out)
}

async fn translate_batch_llm(
    lines: &[&str],
    opts: &TranslateOptions,
    prev: &str,
    next: &str,
) -> anyhow::Result<Vec<Option<String>>> {
    let (system, user) = llm_prompt(
        lines,
        &opts.source_lang,
        &opts.target_lang,
        &opts.context,
        prev,
        next,
    );
    let mut last_err = String::new();
    for _ in 0..2 {
        match crate::core::ai::chat(&system, &user).await {
            Ok(text) => {
                if let Some(parsed) = parse_llm_json(&text, lines.len()) {
                    return Ok(parsed);
                }
                last_err = "resposta fora do formato JSON".to_string();
            }
            Err(e) => last_err = e,
        }
    }
    Err(anyhow!("tradução por IA falhou: {}", last_err))
}

async fn translate_batch_libre(
    lines: &[&str],
    base_url: &str,
    api_key: &str,
    opts: &TranslateOptions,
) -> anyhow::Result<Vec<Option<String>>> {
    let client = super::client()?;
    let url = format!("{}/translate", base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "q": lines,
        "source": if opts.source_lang.is_empty() { "auto" } else { opts.source_lang.as_str() },
        "target": opts.target_lang,
        "format": "text",
    });
    if !api_key.is_empty() {
        body["api_key"] = serde_json::Value::String(api_key.to_string());
    }
    let resp = client.post(&url).json(&body).send().await?;
    let status = resp.status();
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("LibreTranslate: resposta invalida ({})", e))?;
    if !status.is_success() {
        return Err(anyhow!(
            "LibreTranslate: HTTP {} {}",
            status.as_u16(),
            v["error"].as_str().unwrap_or("")
        ));
    }
    let out = match &v["translatedText"] {
        serde_json::Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::String(s) => vec![Some(s.clone())],
        _ => vec![None; lines.len()],
    };
    Ok(out)
}

pub async fn translate_cues(
    cues: &[Cue],
    opts: &TranslateOptions,
    progress: super::ProgressFn,
) -> anyhow::Result<TranslateResult> {
    let id = "translate";
    let batch = opts.batch_size.clamp(1, 100);
    let mut out = cues.to_vec();
    let mut failed = Vec::new();
    let total = cues.len() as u64;
    let mut done = 0u64;
    for (bi, chunk) in cues.chunks(batch).enumerate() {
        let lines: Vec<&str> = chunk.iter().map(|c| c.text.as_str()).collect();
        let start = bi * batch;
        let prev = cues[start.saturating_sub(3)..start]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let end = (start + chunk.len()).min(cues.len());
        let next = cues[end..(end + 3).min(cues.len())]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let translated = match &opts.translator {
            Translator::Llm => translate_batch_llm(&lines, opts, &prev, &next).await?,
            Translator::LibreTranslate { base_url, api_key } => {
                translate_batch_libre(&lines, base_url, api_key, opts).await?
            }
        };
        for (i, t) in translated.into_iter().enumerate() {
            match t {
                Some(text) if !text.trim().is_empty() => out[start + i].text = text,
                _ => failed.push(start + i),
            }
        }
        done += chunk.len() as u64;
        super::report(&progress, id, "translate", done, Some(total), None);
    }
    Ok(TranslateResult { cues: out, failed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llm_json_with_noise() {
        let t = "Sure! ```json\n{\"1\": \"Olá\", \"2\": \"Mundo\", \"9\": \"x\"}\n```";
        let p = parse_llm_json(t, 2).unwrap();
        assert_eq!(p[0].as_deref(), Some("Olá"));
        assert_eq!(p[1].as_deref(), Some("Mundo"));
    }
}
