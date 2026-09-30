//! Reading help from the AI provider set up in Settings: translate or
//! explain a passage, define a word in context, summarize a chapter, recap
//! the story so far without spoilers, or answer a question about the book
//! using only the text up to where you are.

use omniget_core::core::ai;

/// Roughly 30k tokens of context, enough for a long chapter.
const MAX_CONTEXT: usize = 120_000;

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Keep the end: it is closest to where the reader is.
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

pub fn prompt(
    task: &str,
    text: &str,
    context: &str,
    lang: &str,
    question: &str,
    title: &str,
) -> Result<(String, String), String> {
    let lang = if lang.trim().is_empty() {
        "the language of the passage"
    } else {
        lang.trim()
    };
    let book = if title.is_empty() {
        String::new()
    } else {
        format!(" from the book \"{title}\"")
    };
    let ctx = clip(context, MAX_CONTEXT);
    Ok(match task {
        "translate" => (
            format!("You translate passages{book} faithfully into {lang}, keeping tone, names and formatting. Reply with the translation only."),
            text.to_string(),
        ),
        "explain" => (
            format!("You help someone reading{book}. Explain the passage in plain words in {lang}: meaning, references, idioms and why it matters here. Be concise. Never reveal anything that happens later in the book."),
            format!("Context before the passage:\n{}\n\nPassage:\n{text}", clip(context, 8_000)),
        ),
        "define" => (
            format!("You are a dictionary. Define the word or phrase as used in this sentence, in {lang}: part of speech, meaning in context, one short example. Reply in under 80 words."),
            format!("Word: {text}\nSentence: {}", clip(context, 1_000)),
        ),
        "summarize" => (
            format!("Summarize this chapter{book} in {lang} as 5 to 8 short bullet points, in story order. Only use the text given."),
            ctx,
        ),
        "recap" => (
            format!("Someone is returning to the book{book}. Write a spoiler-free recap in {lang} of everything that happened up to the end of the text given: main characters, where things stand, open questions. Under 250 words. Only use the text given; nothing after it."),
            ctx,
        ),
        "ask" => (
            format!("Answer questions about the book{book} in {lang}, using only the text given (it ends where the reader is). If the answer is not in it, say so instead of guessing, and never reveal later events."),
            format!("Text so far:\n{ctx}\n\nQuestion: {question}"),
        ),
        "characters" => (
            format!("List the characters who appear in this text{book}, in {lang}: name, who they are, relationships. Only use the text given."),
            ctx,
        ),
        "quiz" => (
            format!("Write 5 short comprehension questions (with answers after a line '---') in {lang} about this text{book}."),
            ctx,
        ),
        "simplify" => (
            format!("Rewrite the passage in simpler {lang} for a language learner (around CEFR B1), keeping the meaning."),
            text.to_string(),
        ),
        _ => return Err(format!("unknown task {task}")),
    })
}

#[tauri::command]
pub async fn read_ai(
    task: String,
    text: String,
    context: Option<String>,
    lang: Option<String>,
    question: Option<String>,
    title: Option<String>,
) -> Result<String, String> {
    if !ai::get().is_configured() {
        return Err("set up an AI provider in Settings → AI first".into());
    }
    let (system, user) = prompt(
        &task,
        &text,
        context.as_deref().unwrap_or(""),
        lang.as_deref().unwrap_or(""),
        question.as_deref().unwrap_or(""),
        title.as_deref().unwrap_or(""),
    )?;
    ai::chat(&system, &user).await
}

/// Sends a comic page to a manga-image-translator server the user runs
/// (`python server/main.py --start-instance`) and returns the translated
/// page as a data URL. `image_url` is the page on our local server.
#[tauri::command]
pub async fn read_translate_page(
    image_url: String,
    server: String,
    target_lang: String,
) -> Result<String, String> {
    use base64::Engine;
    let http = super::sources::http();
    let img = http
        .get(&image_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let data = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&img)
    );
    let body = serde_json::json!({
        "image": data,
        "config": { "translator": { "target_lang": if target_lang.is_empty() { "ENG".to_string() } else { target_lang.to_uppercase() } } }
    });
    let base = if server.trim().is_empty() {
        "http://127.0.0.1:8000"
    } else {
        server.trim().trim_end_matches('/')
    };
    let r = http
        .post(format!("{base}/translate/image"))
        .timeout(std::time::Duration::from_secs(300))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("translator not reachable at {base}: {e}"))?;
    if !r.status().is_success() {
        return Err(format!("translator: {}", r.status()));
    }
    let ctype = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png")
        .to_string();
    let out = r.bytes().await.map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{ctype};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&out)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_from_the_end() {
        assert_eq!(clip("abcdef", 3), "…def");
        assert_eq!(clip("abc", 3), "abc");
        let (sys, user) =
            prompt("ask", "", "The butler did it.", "pt", "Who did it?", "X").unwrap();
        assert!(sys.contains("pt"));
        assert!(user.ends_with("Question: Who did it?"));
        assert!(prompt("nope", "", "", "", "", "").is_err());
    }
}
