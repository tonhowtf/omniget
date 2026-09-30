// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(
    clippy::cognitive_complexity,
    clippy::identity_op,
    clippy::unreadable_literal
)]

//! This module contains all of the bare types, each
//! represented by a `struct`. All of them implement
//! [`Identifiable`], [`Serializable`] and [`Deserializable`].
//!
//! [`Identifiable`]: ../trait.Identifiable.html
//! [`Serializable`]: ../trait.Serializable.html
//! [`Deserializable`]: ../trait.Deserializable.html

include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
