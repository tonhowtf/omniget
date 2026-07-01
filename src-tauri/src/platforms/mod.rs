pub use crate::platforms::magnet::MagnetDownloader;
pub use omniget_core::platforms::traits;
pub use omniget_core::platforms::BlueskyDownloader;
pub use omniget_core::platforms::DirectFileDownloader;
pub use omniget_core::platforms::DouyinDownloader;
pub use omniget_core::platforms::GenericYtdlpDownloader;
pub use omniget_core::platforms::InstagramDownloader;
pub use omniget_core::platforms::P2pDownloader;
pub use omniget_core::platforms::PinterestDownloader;
pub use omniget_core::platforms::Platform;
pub use omniget_core::platforms::RedditDownloader;
pub use omniget_core::platforms::TikTokDownloader;
pub use omniget_core::platforms::TwitchClipsDownloader;
pub use omniget_core::platforms::TwitterDownloader;
pub use omniget_core::platforms::VimeoDownloader;
pub use omniget_core::platforms::YouTubeDownloader;

pub mod magnet;
pub mod noop;
pub mod twitter;

#[cfg(not(target_os = "android"))]
pub mod bilibili;
#[cfg(not(target_os = "android"))]
pub mod gallerydl;
#[cfg(not(target_os = "android"))]
pub mod generic_ytdlp;
// Ported to omniget-core: bluesky, direct_file, douyin, instagram, pinterest, p2p, reddit, tiktok, twitch, twitter, vimeo, youtube
