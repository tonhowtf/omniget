use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdemyCourse {
    pub id: u64,
    pub title: String,
    pub published_title: String,
    pub image_url: Option<String>,
    pub num_lectures: Option<u32>,
    pub num_chapters: Option<u32>,
}
