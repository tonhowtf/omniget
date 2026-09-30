//! Pure building blocks for the `/read` superpower: book formats, EPUB,
//! MOBI, FB2 and comic metadata, OPDS catalogs, legal book and manga
//! sources, and the KOReader sync protocol. The app
//! (`src-tauri/src/commands/read`) owns the library database, the local
//! server and the UI glue; the CLI reuses these.

pub mod comic;
pub mod epub;
pub mod epub_writer;
pub mod extract;
pub mod fb2;
pub mod format;
pub mod kepub;
pub mod kosync;
pub mod mobi;
pub mod opds;
pub mod protobuf;
pub mod sources;
pub mod tachibk;
pub mod trackers;
pub mod xml;
