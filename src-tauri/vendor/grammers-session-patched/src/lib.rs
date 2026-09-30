// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! This library serves as the session interface for all of the crucial
//! data that other Telegram libraries would need to operate correctly, including:
//! - Datacenter addresses, to know where to connect.
//! - Permanent Authorization Keys bound to each datacenter.
//! - Update state, in order to catch up on missed updates while offline.
//! - Cached peers, necessary to interact with the API.
//!
//! Most of the data is bound to the specific session, and cannot be reused
//! outside of it. The exceptions are datacenter addresses, and update state
//! which is actually account-bound.
//!
//! To use with other libraries, you will want to instantiate one of the
//! [`storages`], which are what implement the [`Session`] trait.
//!
//! To convert between storages, you can use the [`SessionData`] as an
//! intermediate step, and use its `From` implementations in combination
//! with [`SessionData::import_to`]. Note that the `From` implementation
//! will not copy all of the data, only that which is necessary.

#![deny(unsafe_code)]

mod chat;
mod dc_options;
pub mod defs;
mod generated;
mod message_box;
mod peer;
mod session;
mod session_data;
pub mod storages;
pub mod updates;

#[allow(deprecated)]
pub use chat::PeerAuthCache;
pub(crate) use dc_options::{DEFAULT_DC, KNOWN_DC_OPTIONS};
pub use session::Session;
pub use session_data::SessionData;

// Needed for auto-generated definitions.
use generated::{enums, types};
use grammers_tl_types::{Deserializable, Identifiable, Serializable, deserialize};
