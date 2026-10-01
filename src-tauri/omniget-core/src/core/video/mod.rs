//! Pure building blocks for the `/video` superpower (Watch): file naming,
//! natural order, course trees, subtitles, progress rules, IPTV playlists,
//! XMLTV guides, the open Stremio addon protocol and note export.

pub mod course;
pub mod m3u;
pub mod naming;
pub mod notes_md;
pub mod progress;
pub mod stremio;
pub mod subs;
pub mod xmltv;

pub use notes_md::fmt_ts;
