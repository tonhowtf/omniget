//! Optional providers (estudo 05). Nothing here runs unless the user
//! configured it: Tripo needs the user's API key, skin-tokens.cpp needs its
//! binary and 1.25 GB of weights downloaded on request. Every output goes
//! through the normalizer like any other import.

pub mod skintokens;
pub mod tripo;

use async_trait_free::BoxFuture;

/// Text or image to a GLB (cloud or local generator).
pub trait Generator: Send + Sync {
    fn id(&self) -> &'static str;
    /// Cost/credit line shown before the user confirms.
    fn quote(&self) -> BoxFuture<'_, Result<Quote, String>>;
    fn generate(&self, req: GenerateRequest) -> BoxFuture<'_, Result<Generated, String>>;
}

/// Static mesh to a skinned mesh.
pub trait Rigger: Send + Sync {
    fn id(&self) -> &'static str;
    fn rig(&self, static_glb: Vec<u8>) -> BoxFuture<'_, Result<Vec<u8>, String>>;
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Quote {
    pub provider: String,
    /// Credits one generation costs (provider units).
    pub cost: f64,
    /// Credits left on the user's account, when the provider says.
    pub balance: Option<f64>,
    pub note: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct GenerateRequest {
    pub prompt: Option<String>,
    /// PNG/JPEG/WebP bytes for image-to-3D.
    pub image: Option<Vec<u8>>,
    pub face_limit: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Generated {
    pub glb: Vec<u8>,
    pub provider: String,
    pub model: String,
    pub task_id: String,
}

/// Minimal boxed future, to keep the traits object-safe without pulling
/// `async-trait` into this crate.
pub mod async_trait_free {
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
}
