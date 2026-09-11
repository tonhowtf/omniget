//! Categoria Música: o que fazer com a música que já é sua.
//!
//! - `playlist`: casa uma playlist exportada com os arquivos do disco e
//!   escreve `.m3u8`/`.pls` para o tocador local.
//! - `lyrics`: letra sincronizada (`.lrc`) pelo LRCLIB.
//! - `history`: análise do export oficial do Spotify, sem rede.
//!
//! `norm` é o que os três compartilham: normalizar artista e título de um
//! jeito só, para "Song (Remastered 2011)" e "song" serem a mesma coisa.

pub mod history;
pub mod lyrics;
pub mod norm;
pub mod playlist;
