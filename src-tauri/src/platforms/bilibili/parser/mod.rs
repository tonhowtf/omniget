use serde::{Deserialize, Serialize};

use super::api::{ApiClient, Result};
use super::url_kind::UrlKind;

pub mod bangumi;
pub mod cheese;
pub mod favlist;
pub mod festival;
pub mod history;
pub mod list;
pub mod popular;
pub mod space;
pub mod video;
pub mod watch_later;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EpisodeItem {
    pub episode_id: String,
    pub title: String,
    pub aid: Option<u64>,
    pub bvid: Option<String>,
    pub cid: Option<u64>,
    pub ep_id: Option<u64>,
    pub season_id: Option<u64>,
    pub duration_seconds: Option<f64>,
    pub cover_url: Option<String>,
    pub pub_time_secs: Option<u64>,
    pub page: Option<u32>,
    pub page_title: Option<String>,
    pub badge: Option<String>,
    pub section_title: Option<String>,
    pub episode_number: Option<u32>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContentMetadata {
    pub series_title: Option<String>,
    pub season_title: Option<String>,
    pub season_number: Option<u32>,
    pub season_id: Option<u64>,
    pub media_id: Option<u64>,
    pub uploader: Option<String>,
    pub uploader_uid: Option<u64>,
    pub uploader_avatar: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub poster: Option<String>,
    pub areas: Vec<String>,
    pub styles: Vec<String>,
    pub actors: Option<String>,
    pub rating: Option<f32>,
    pub premiered_secs: Option<u64>,
    pub favorites_name: Option<String>,
    pub favorites_owner: Option<String>,
    pub collection_title: Option<String>,
    pub space_owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaginationInfo {
    pub total_items: u32,
    pub total_pages: u32,
    pub current_page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedContent {
    pub title: String,
    pub items: Vec<EpisodeItem>,
    pub metadata: ContentMetadata,
    pub pagination: Option<PaginationInfo>,
}

impl ParsedContent {
    pub fn single(title: String, item: EpisodeItem) -> Self {
        Self {
            title,
            items: vec![item],
            metadata: ContentMetadata::default(),
            pagination: None,
        }
    }
}

pub async fn parse(client: &ApiClient, kind: &UrlKind) -> Result<ParsedContent> {
    match kind {
        UrlKind::Video { bvid_or_av, page } => video::parse(client, bvid_or_av, *page).await,
        UrlKind::BangumiEpisode { ep_id } => bangumi::parse_by_ep(client, *ep_id).await,
        UrlKind::BangumiSeason { season_id } => bangumi::parse_by_season(client, *season_id).await,
        UrlKind::BangumiMedia { media_id } => bangumi::parse_by_media(client, *media_id).await,
        UrlKind::CheeseEpisode { ep_id } => cheese::parse_by_ep(client, *ep_id).await,
        UrlKind::CheeseSeason { season_id } => cheese::parse_by_season(client, *season_id).await,
        UrlKind::Space { mid } => space::parse(client, *mid, 1).await,
        UrlKind::Favlist { fid } => favlist::parse(client, *fid, 1).await,
        UrlKind::Collection { mid, sid } => list::parse_collection(client, *mid, *sid, 1).await,
        UrlKind::Series { mid, sid } => list::parse_series(client, *mid, *sid, 1).await,
        UrlKind::PopularWeek { num } => popular::parse(client, *num).await,
        UrlKind::WatchLater => watch_later::parse(client, 1).await,
        UrlKind::History => history::parse(client, 1).await,
        UrlKind::Festival { url } => festival::parse(client, url).await,
    }
}
