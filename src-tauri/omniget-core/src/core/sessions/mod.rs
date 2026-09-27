//! Analytics multi-ferramenta: sessões de todos os agentes de código num modelo
//! neutro (`model.rs`, contrato do orquestrador). Parsers por ferramenta em
//! `parsers/` (grupo A: worker `s1-sessions`; grupo B: worker `s2-sessions`).
//!
//! * `index` — índice incremental em `<app_data>/llm/sessions.db`.
//! * `analysis` — análise por sessão e agregados (uso, heatmap, agentes,
//!   time, retrospectiva).
//! * `search` — busca global e dentro da sessão.
//! * `export` / `import` — Markdown/JSON neutro, retomada no Claude/Codex.
//! * `api` — as operações que os comandos Tauri chamam.

pub mod analysis;
pub mod api;
pub mod cost;
pub mod export;
pub mod import;
pub mod index;
pub mod model;
pub mod parsers;
pub mod search;
pub mod util;
