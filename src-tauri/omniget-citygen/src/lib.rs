//! Procedural city of the OmniGet world (estudo 99; plano
//! `omnidisc-server/GameChangeLLM/PLANO.md`).
//!
//! The LLM is the urban planner and the architect; this crate is the engineer
//! with the building code:
//! - [`charter`] and [`terrain`] / [`sites`]: the world a city is founded on;
//! - [`program`]: the declarative city program an LLM edits with patch ops;
//! - [`solver`] + [`grammar`]: program → [`types::Blueprint`] (authoritative);
//! - [`checker`]: the building code (R1..R18);
//! - [`mesher`] + [`wfc`]: blueprint → kit instances and terrain triangles;
//! - [`preview`]: a software raster so an LLM can look at what it built;
//! - [`legacy`]: converts the 2D city that runs in production into a program.

pub mod charter;
pub mod checker;
pub mod grammar;
pub mod hash;
pub mod legacy;
pub mod mesher;
pub mod preview;
pub mod program;
pub mod sites;
pub mod solver;
pub mod terrain;
pub mod types;
pub mod wfc;

pub use types::*;
