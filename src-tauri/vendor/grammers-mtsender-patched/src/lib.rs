// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! This library serves to abstract the connection to Telegram's servers.
//!
//! The [`Sender`] is the base building block that manages a single network
//! connection, the transport state, and the MTP state, as well as taking
//! care of buffering messages before sending them off in a single container.
//!
//! To interact with the API, it is often needed to create more than one
//! connection, for either migrations during sign in or media downloads.
//! To that end, the [`SenderPool`] is the one that manages any number
//! of `Sender`. This will commonly be the entry point to using this library.
//!
//! Generally, there will be a single `SenderPool` instance per client,
//! although many tasks can share access to the same `SenderPool` via
//! multiple [`SenderPoolHandle`]s.

#![deny(unsafe_code)]

mod configuration;
mod errors;
mod net;
mod sender;
mod sender_pool;

pub use configuration::ConnectionParams;
pub use errors::{InvocationError, ReadError, RpcError};
pub use net::ServerAddr;
pub use sender::{Sender, connect, connect_with_auth, generate_auth_key};
pub use sender_pool::{SenderPool, SenderPoolHandle, SenderPoolRunner};
