//! Tauri bridge of the World's city.
//!
//! - [`city`] — the `city_*` commands: the persistent, server-authoritative city
//!   on an OmniDisc instance (join, input, chat, the HTTP API, pictures and
//!   the session).
//!
//! The simulation itself runs on the server (`omniget-world` is its engine);
//! the app only relays inputs and decodes what comes back.

pub mod city;
