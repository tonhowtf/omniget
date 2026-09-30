//! The world's visual contract and the two tools that enforce it.
//!
//! * [`schema`] — the Rust mirror of `static/world/atlas.schema.json`;
//! * [`pack`] — `atlas-pack`: source PNGs into pages plus `atlas.json`;
//! * [`check`] — `atlas-check`: the validation gate, `ERR_ATLAS_*` codes;
//! * [`skyline`] — the packer itself;
//! * [`png`] — a chunk walker, because a colour profile is invisible to a decoder;
//! * [`fixture`] — synthetic source art for the tests.
//!
//! This crate is tooling. It is a workspace member but not a dependency of the
//! app, so nothing here ships in the binary.

pub mod check;
pub mod fixture;
pub mod pack;
pub mod png;
pub mod schema;
pub mod skyline;
