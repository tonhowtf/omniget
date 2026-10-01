use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thumb {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub artist: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    Live,
    Upcoming,
    Premiere,
    Members,
    New,
    Hd4k,
    Cc,
    Paid,
    Explicit,
    Streamed,
    AutoDubbed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub video_id: Option<String>,
    #[serde(default)]
    pub t: Option<u32>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub channel: Option<ChannelRef>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub duration_s: Option<u32>,
    #[serde(default)]
    pub views: Option<u64>,
    #[serde(default)]
    pub views_text: Option<String>,
    #[serde(default)]
    pub published_text: Option<String>,
    #[serde(default)]
    pub published_ms: Option<i64>,
    #[serde(default)]
    pub badges: Vec<Badge>,
    #[serde(default)]
    pub watched_pct: Option<u8>,
    #[serde(default)]
    pub description_snippet: Option<String>,
    #[serde(default)]
    pub feedback_token: Option<String>,
    #[serde(default)]
    pub set_video_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShortItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub views_text: Option<String>,
    #[serde(default)]
    pub channel: Option<ChannelRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub subscribers_text: Option<String>,
    #[serde(default)]
    pub video_count_text: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub artist: bool,
    #[serde(default)]
    pub subscribed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub channel: Option<ChannelRef>,
    #[serde(default)]
    pub count_text: Option<String>,
    #[serde(default)]
    pub is_mix: bool,
    #[serde(default)]
    pub first_video_id: Option<String>,
    #[serde(default)]
    pub is_music: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<ChannelRef>,
    #[serde(default)]
    pub album: Option<AlbumRef>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub duration_s: Option<u32>,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub is_video: bool,
    #[serde(default)]
    pub plays_text: Option<String>,
    #[serde(default)]
    pub set_video_id: Option<String>,
    #[serde(default)]
    pub liked: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumItem {
    pub id: String,
    #[serde(default)]
    pub playlist_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<ChannelRef>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub album_type: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub subscribers_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MoodItem {
    pub title: String,
    pub browse_id: String,
    #[serde(default)]
    pub params: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Video(VideoItem),
    Short(ShortItem),
    Channel(ChannelItem),
    Playlist(PlaylistItem),
    Song(SongItem),
    Album(AlbumItem),
    Artist(ArtistItem),
    Mood(MoodItem),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Nav {
    pub browse_id: String,
    #[serde(default)]
    pub params: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShelfKind {
    #[default]
    Grid,
    Row,
    Shorts,
    QuickPicks,
    Carousel,
    List,
    Moods,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub strapline: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub kind: ShelfKind,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub more: Option<Nav>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    pub label: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PageHeader {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub header: Option<PageHeader>,
    #[serde(default)]
    pub chips: Vec<Chip>,
    #[serde(default)]
    pub shelves: Vec<Shelf>,
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub corrected_query: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_s: u32,
    #[serde(default)]
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Like,
    Dislike,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WatchPlaylist {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(default)]
    pub items: Vec<VideoItem>,
    #[serde(default)]
    pub is_mix: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WatchInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub channel: Option<ChannelItem>,
    #[serde(default)]
    pub views: Option<u64>,
    #[serde(default)]
    pub views_text: Option<String>,
    #[serde(default)]
    pub published_text: Option<String>,
    #[serde(default)]
    pub date_text: Option<String>,
    #[serde(default)]
    pub description: Vec<TextRun>,
    #[serde(default)]
    pub likes: Option<u64>,
    #[serde(default)]
    pub likes_text: Option<String>,
    #[serde(default)]
    pub rating: Option<Rating>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub related: Vec<Item>,
    #[serde(default)]
    pub related_continuation: Option<String>,
    #[serde(default)]
    pub comments_token: Option<String>,
    #[serde(default)]
    pub comments_count_text: Option<String>,
    #[serde(default)]
    pub comments_sorts: Vec<CommentSort>,
    #[serde(default)]
    pub playlist: Option<WatchPlaylist>,
    #[serde(default)]
    pub is_live: bool,
    #[serde(default)]
    pub badges: Vec<Badge>,
    #[serde(default)]
    pub saved: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: ChannelRef,
    #[serde(default)]
    pub text: Vec<TextRun>,
    #[serde(default)]
    pub published_text: Option<String>,
    #[serde(default)]
    pub likes_text: Option<String>,
    #[serde(default)]
    pub reply_count: u32,
    #[serde(default)]
    pub replies_token: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub hearted: bool,
    #[serde(default)]
    pub is_owner: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommentSort {
    pub label: String,
    pub token: String,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommentsPage {
    #[serde(default)]
    pub count_text: Option<String>,
    #[serde(default)]
    pub sorts: Vec<CommentSort>,
    #[serde(default)]
    pub items: Vec<Comment>,
    #[serde(default)]
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelTab {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub params: Option<String>,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelPage {
    pub channel: ChannelItem,
    #[serde(default)]
    pub banner: Vec<Thumb>,
    #[serde(default)]
    pub tabs: Vec<ChannelTab>,
    #[serde(default)]
    pub page: Page,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub owner: Option<ChannelRef>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub count_text: Option<String>,
    #[serde(default)]
    pub views_text: Option<String>,
    #[serde(default)]
    pub updated_text: Option<String>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub is_music: bool,
    #[serde(default)]
    pub duration_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub id: String,
    #[serde(default)]
    pub playlist_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub album_type: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub artists: Vec<ChannelRef>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub track_count_text: Option<String>,
    #[serde(default)]
    pub duration_text: Option<String>,
    #[serde(default)]
    pub tracks: Vec<SongItem>,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub other_versions: Vec<AlbumItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistPage {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subscribers_text: Option<String>,
    #[serde(default)]
    pub subscribed: Option<bool>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub shuffle_playlist_id: Option<String>,
    #[serde(default)]
    pub radio_playlist_id: Option<String>,
    #[serde(default)]
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueuePage {
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub tracks: Vec<SongItem>,
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub lyrics_browse_id: Option<String>,
    #[serde(default)]
    pub related_browse_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    #[serde(default)]
    pub start_ms: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub source: String,
    pub synced: bool,
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(default)]
    pub footer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind {
    Video,
    Audio,
    Muxed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub itag: u32,
    pub kind: FormatKind,
    pub mime: String,
    pub codecs: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<u32>,
    #[serde(default)]
    pub bitrate: u64,
    #[serde(default)]
    pub content_length: Option<u64>,
    #[serde(default)]
    pub quality_label: Option<String>,
    #[serde(default)]
    pub audio_lang: Option<String>,
    #[serde(default)]
    pub audio_name: Option<String>,
    #[serde(default)]
    pub audio_default: bool,
    #[serde(default)]
    pub auto_dubbed: bool,
    #[serde(default)]
    pub hdr: bool,
    #[serde(default)]
    pub drc: bool,
    #[serde(default)]
    pub init_range: Option<ByteRange>,
    #[serde(default)]
    pub index_range: Option<ByteRange>,
    #[serde(skip_serializing, default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub lang: String,
    pub name: String,
    #[serde(default)]
    pub auto: bool,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Storyboard {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub count: u32,
    pub cols: u32,
    pub rows: u32,
    pub interval_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamSet {
    pub session: String,
    pub video_id: String,
    #[serde(default)]
    pub duration_s: u32,
    pub client: String,
    #[serde(default)]
    pub formats: Vec<Format>,
    #[serde(default)]
    pub loudness_db: Option<f32>,
    #[serde(default)]
    pub expires_ms: i64,
    #[serde(default)]
    pub is_live: bool,
    #[serde(default)]
    pub hls: Option<String>,
    #[serde(default)]
    pub captions: Vec<Caption>,
    #[serde(default)]
    pub storyboard: Option<Storyboard>,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SbCategory {
    Sponsor,
    Selfpromo,
    Interaction,
    Intro,
    Outro,
    Preview,
    Hook,
    MusicOfftopic,
    Filler,
    PoiHighlight,
    ExclusiveAccess,
    Chapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SbAction {
    Skip,
    Mute,
    Full,
    Poi,
    Chapter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SbSegment {
    pub uuid: String,
    pub category: SbCategory,
    pub action: SbAction,
    pub start_s: f64,
    pub end_s: f64,
    #[serde(default)]
    pub votes: i32,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeArrow {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thumb_time_s: Option<f64>,
    #[serde(default)]
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ryd {
    pub likes: u64,
    pub dislikes: u64,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub view_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub logged_in: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub premium: Option<bool>,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalSub {
    pub channel_id: String,
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    pub added_ms: i64,
    #[serde(default)]
    pub last_checked_ms: Option<i64>,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub auto_download: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub format: String,
    #[serde(default)]
    pub subs_added: u32,
    #[serde(default)]
    pub subs_skipped: u32,
    #[serde(default)]
    pub history_added: u32,
    #[serde(default)]
    pub playlists_added: u32,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedSub {
    pub channel_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedWatch {
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub watched_ms: Option<i64>,
    #[serde(default)]
    pub position_s: Option<u32>,
    #[serde(default)]
    pub duration_s: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub channel: Option<ChannelRef>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub duration_s: Option<u32>,
    #[serde(default)]
    pub position_s: u32,
    pub watched_ms: i64,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoRule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    #[serde(default)]
    pub include_words: Vec<String>,
    #[serde(default)]
    pub exclude_words: Vec<String>,
    #[serde(default)]
    pub min_duration_s: Option<u32>,
    #[serde(default)]
    pub max_duration_s: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub include_shorts: bool,
    #[serde(default)]
    pub include_live: bool,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub sponsorblock: String,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub max_gb: f64,
    #[serde(default)]
    pub keep_days: u32,
    #[serde(default)]
    pub keep_per_channel: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_bytes: u64,
    pub files: u32,
    #[serde(default)]
    pub free_bytes: Option<u64>,
    #[serde(default)]
    pub cap_bytes: Option<u64>,
}
