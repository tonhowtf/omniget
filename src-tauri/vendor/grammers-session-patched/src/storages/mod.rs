// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Built-in [`crate::Session`] storages.
//!
//! Some may require certain features to be enabled. If none fit
//! your needs, you can also implement [`crate::Session`] yourself.

mod memory;
mod sqlite;
mod tl_session;

pub use memory::MemorySession;
pub use sqlite::SqliteSession;
#[allow(deprecated)]
pub use tl_session::TlSession;

/// `TlSession` version.
pub use crate::generated::common::LAYER as TL_VERSION;
