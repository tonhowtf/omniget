//! Um arquivo por ferramenta. Grupo A (worker s1): claude, codex, gemini, qwen,
//! opencode (serve também kilo), crush, goose. Grupo B (worker s2): cursor,
//! copilot, cline, zed, droid, kimi, pi, amp, aider, junie, auggie, grok, kiro.
//! Cada worker declara aqui só os seus `pub mod`, cada um na sua linha, e
//! registra a fonte em `all_sources_a()` / `all_sources_b()` no próprio arquivo
//! `group_a.rs` / `group_b.rs`.

pub mod group_a;
pub mod group_b;

use super::model::SessionSource;

/// Todas as fontes conhecidas, na ordem de exibição.
pub fn all_sources() -> Vec<Box<dyn SessionSource>> {
    let mut v = group_a::sources();
    v.extend(group_b::sources());
    v
}
