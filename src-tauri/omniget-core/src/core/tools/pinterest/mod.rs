//! Pinterest (estudo 67): a API web não documentada (`/resource/*Resource/get/`)
//! que o próprio site usa, lida sem login para conteúdo público e com os
//! cookies do usuário para boards secretos. Nada de senha, nada de Selenium.
//!
//! - `api.rs`: cliente, parsing de URL, paginação por `bookmarks`, o `Pin`
//!   normalizado (imagem original, vídeo, carrossel, story, estatísticas,
//!   sinais de IA e de anúncio).
//! - `media.rs`: download do original com cadeia de fallback, vídeo HLS → MP4,
//!   WebP → PNG, arquivo de já-baixados para sincronizar.
//! - `analysis.rs`: dHash (duplicados quase iguais), paleta por k-means,
//!   heurística de IA (listas do Pinterest Power Menu, MIT).
//! - `export.rs`: CSV, JSON, galeria HTML offline e PDF.

pub mod analysis;
pub mod api;
pub mod export;
pub mod media;

pub use api::{parse_target, Board, Person, Pin, PinClient, Section, Target, User};
