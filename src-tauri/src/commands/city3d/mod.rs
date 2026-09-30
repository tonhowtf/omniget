//! Tauri bridge of the 3D city (estudo 99, GameChangeLLM/PLANO.md).
//!
//! - [`city`] — local city store, founding, propose/apply, region data (G2).
//! - [`avatar`] — genomes, catalog, identities (D).
//! - [`codegeo`] — geometry scripts and the kit (E).
//! - [`atlas`] — the kit baked into sprites for the lite (2D) mode.
//! - [`asset3d`] — import normalizer and exports (F).
//! - [`generate`] — optional Tripo generation and skin-tokens auto-rig.
//! - [`live`] — native camera path for Linux (I).

pub mod asset3d;
pub mod atlas;
pub mod avatar;
pub mod city;
pub mod codegeo;
pub mod generate;
pub mod live;
