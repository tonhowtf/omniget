//! Humanizar texto: aplica o skill "Humanizer" (blader/humanizer, MIT) na
//! IA configurada. O skill inteiro vai no prompt de sistema; o texto do
//! usuário vai como mensagem. Só o texto final volta, sem rascunho nem
//! crítica, porque aqui não há chat para conversar sobre o resultado.

use crate::core::ai;

/// O SKILL.md original, sem o frontmatter. Vive num `.txt` porque o repo
/// não versiona `.md`.
pub const SKILL: &str = include_str!("humanizer_skill.txt");

/// Limite generoso: acima disso o modelo perde o fio e o custo dispara.
pub const MAX_CHARS: usize = 40_000;

pub fn system_prompt(sample: Option<&str>) -> String {
    let mut s = String::with_capacity(SKILL.len() + 1024);
    s.push_str(SKILL);
    s.push_str(
        "\n\n## Embedded mode (this run)\n\
         You are running in embedded mode inside a desktop app. Return ONLY the final rewritten text: \
         no draft, no list of patterns, no preamble, no closing remark, no code fences around the text. \
         Keep the language of the input (Portuguese stays Portuguese, English stays English). \
         Keep code blocks, URLs, numbers, names, quotes and data exactly as they are. \
         Keep the original formatting (paragraphs, lists, headings) unless a pattern above says otherwise.\n",
    );
    if let Some(sample) = sample.map(str::trim).filter(|s| !s.is_empty()) {
        s.push_str("\n## Writing sample from the user (match this voice)\n\n");
        s.push_str(sample);
        s.push('\n');
    }
    s
}

pub async fn humanize(text: &str, sample: Option<&str>) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("texto vazio".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!(
            "texto grande demais ({} caracteres; o limite é {})",
            text.chars().count(),
            MAX_CHARS
        ));
    }
    if !ai::get().is_configured() {
        return Err("ai_not_configured".into());
    }
    let out = ai::chat(&system_prompt(sample), text).await?;
    Ok(strip_fence(out.trim()).to_string())
}

/// Alguns modelos devolvem o texto dentro de ```…``` mesmo pedindo que não.
fn strip_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("```") {
        if let Some(end) = inner.rfind("```") {
            let body = &inner[..end];
            // pula o identificador de linguagem na primeira linha, se houver
            return body.split_once('\n').map(|(_, rest)| rest).unwrap_or(body).trim();
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contem_skill_e_modo_embutido() {
        let p = system_prompt(Some("oi, sou eu"));
        assert!(p.contains("Signs of AI writing"));
        assert!(p.contains("Embedded mode"));
        assert!(p.contains("oi, sou eu"));
        assert!(!system_prompt(None).contains("Writing sample"));
    }

    #[test]
    fn tira_cerca_de_codigo() {
        assert_eq!(strip_fence("```text\nolá\n```"), "olá");
        assert_eq!(strip_fence("```\nolá\n```"), "olá");
        assert_eq!(strip_fence("olá"), "olá");
    }
}
