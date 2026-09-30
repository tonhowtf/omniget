//! Pure building blocks for OmniTube (`/youtube`): the innertube client
//! registry and request bodies, response parsers for YouTube and YouTube
//! Music, stream selection and the synthetic DASH manifest, distraction
//! filters, SponsorBlock/DeArrow/RYD payloads, and import formats.
//! No I/O here: the Tauri side (`commands::tube`) does the HTTP.

pub mod extras;
pub mod filters;
pub mod imports;
pub mod innertube;
pub mod lrc;
pub mod model;
pub mod mpd;
pub mod parse;
pub mod player;
pub mod rss;
pub mod walk;

pub use model::*;
