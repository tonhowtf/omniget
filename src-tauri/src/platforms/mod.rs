pub use omniget_core::platforms::traits;
pub use omniget_core::platforms::Platform;

pub mod pinterest;
pub mod tiktok;
pub mod twitter;
pub mod twitch;
pub mod bluesky;
pub mod telegram;

#[cfg(not(target_os = "android"))]
pub mod hotmart;
#[cfg(not(target_os = "android"))]
pub mod instagram;
#[cfg(not(target_os = "android"))]
pub mod reddit;
#[cfg(not(target_os = "android"))]
pub mod youtube;
#[cfg(not(target_os = "android"))]
pub mod vimeo;
#[cfg(not(target_os = "android"))]
pub mod generic_ytdlp;
#[cfg(not(target_os = "android"))]
pub mod udemy;
