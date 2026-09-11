//! Twitch: ferramentas que só usam o que o site expõe sem login.
//!
//! - `gql`: o GraphQL público (Client-ID do player web), parsing de links de
//!   canal, VOD e clipe.
//! - `emotes`: bulk-download de emotes e badges nativos + BTTV, FFZ e 7TV.
//! - `chat`: replay de chat de VOD/clipe exportado em JSON, CSV e legenda.

pub mod chat;
pub mod emotes;
pub mod gql;

pub use gql::{parse_channel, parse_video, ChatTarget, Gql};
