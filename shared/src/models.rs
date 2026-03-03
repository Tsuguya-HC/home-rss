use serde::{Deserialize, Serialize};

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateFeedRequest {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ArticleListQuery {
    pub feed_id: Option<String>,
    pub unread: Option<bool>,
}

// --- DB models ---

#[derive(Debug, Serialize, Deserialize)]
pub struct Feed {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_fetched_at: Option<i64>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Article {
    pub id: String,
    pub feed_id: String,
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<i64>,
    pub fetched_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadStatus {
    pub article_id: String,
    pub read_at: Option<i64>,
}
