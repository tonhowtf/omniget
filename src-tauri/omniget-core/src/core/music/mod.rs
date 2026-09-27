//! Building blocks of the `/music` superpower that don't depend on Tauri:
//! tags, lyrics, playlist files, radio metadata, analysis and conversion.

pub mod convert;
pub mod dsp;
pub mod icy;
pub mod loudness;
pub mod lrc;
pub mod playlist_file;
pub mod smart;
pub mod tags;
pub mod template;
